//! The M1 gate, end to end: a folder of plaintext notes is dragged into a store folder, encrypted in
//! place, indexed, and queried. Nothing readable may be left on disk except hidden files, which the
//! app never touches.
//!
//! `synthetic_vault_gate` always runs. `real_vault_gate` runs on your own notes (and prints what to
//! compare with Obsidian): FLINT_SAMPLE_VAULT=/path cargo test -p flint-testkit --test m1_gate -- --ignored --nocapture

use std::collections::BTreeSet;
use std::time::Duration;

use flint_core::StorePath;
use flint_index::Index;
use flint_testkit::{
    Finding, TestStore, assert_backlinks_agree_with_resolution, copy_dir_all, find_plaintext,
    index_store, is_hidden,
};
use tempfile::TempDir;

const NOW: Duration = Duration::ZERO;

fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

fn open_index(store: &TestStore) -> Index {
    std::fs::create_dir_all(store.root().join(".store")).unwrap();
    Index::open(
        &store.root().join(".store/index.db"),
        store.keys.index_key(),
    )
    .unwrap()
}

fn names(paths: impl IntoIterator<Item = StorePath>) -> Vec<String> {
    paths.into_iter().map(|p| p.to_string()).collect()
}

fn backlink_sources(index: &Index, target: &str) -> Vec<String> {
    index
        .backlinks(&p(target))
        .unwrap()
        .into_iter()
        .map(|b| b.from.to_string())
        .collect()
}

fn paths_of(findings: &[Finding]) -> BTreeSet<String> {
    findings
        .iter()
        .map(|f| f.path.to_string_lossy().into_owned())
        .collect()
}

// ---- the synthetic vault -------------------------------------------------------------------

const VISIBLE: &[(&str, &str)] = &[
    (
        "Back story.md",
        "# Back story\n\nMARKER-BACKSTORY-71f3 The campaign opens in [[Vallaki]]. See [[Characters/Baron]] and [[Vistani]].",
    ),
    (
        "Characters/Baron.md",
        "---\naliases: [The Baron]\n---\nMARKER-BARON-2c9e lives in [[Vallaki]]. #character",
    ),
    (
        "Characters/Strahd.md",
        "---\naliases: [Count von Zarovich]\n---\nMARKER-STRAHD-88aa rules [[Vallaki]]. #character",
    ),
    (
        "Locations/Vallaki.md",
        "# Vallaki\n\nMARKER-VALLAKI-5d01 is a town. [[Strahd]] and [[The Baron]]. #location",
    ),
    (
        "Quests/shortage of wine.md",
        "![[Pasted image 1.png]]\nMARKER-WINE-93be goes to [[Vallaki]] via [[Count von Zarovich]].",
    ),
    ("2026-07-03.md", "MARKER-SESSION-14ab we met [[Baron]]."),
    (
        "Transcripts/2026-07-03.txt",
        "MARKER-TRANSCRIPT-b7c0 the spoken words",
    ),
];
const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR....MARKER-IMAGE-6e42....IEND";
const HIDDEN: &[(&str, &str)] = &[
    (".git/HEAD", "MARKER-GIT-aa11 ref: refs/heads/main"),
    (".claude/commands/x.md", "MARKER-CLAUDE-0f0f # a command"),
    (".DS_Store", "MARKER-DSSTORE-9999"),
];

fn write_synthetic_vault(root: &std::path::Path) {
    let mut all: Vec<(&str, &[u8])> = VISIBLE.iter().map(|(p, t)| (*p, t.as_bytes())).collect();
    all.push(("Images/Pasted image 1.png", IMAGE));
    all.extend(HIDDEN.iter().map(|(p, t)| (*p, t.as_bytes())));
    for (path, bytes) in all {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
    }
}

#[test]
fn synthetic_vault_gate() {
    let source = TempDir::new().unwrap();
    write_synthetic_vault(source.path());
    let store = TestStore::new();
    copy_dir_all(source.path(), store.root()).unwrap(); // the user drags the folder in

    let visible_markers: Vec<&str> = VISIBLE
        .iter()
        .map(|(_, t)| {
            &t[t.find("MARKER").unwrap()..][..t[t.find("MARKER").unwrap()..].find(' ').unwrap()]
        })
        .chain(["MARKER-IMAGE-6e42"])
        .collect();
    assert_eq!(visible_markers.len(), 8, "{visible_markers:?}");
    let hidden_markers = [
        "MARKER-GIT-aa11",
        "MARKER-CLAUDE-0f0f",
        "MARKER-DSSTORE-9999",
    ];
    let all_markers: Vec<&str> = visible_markers
        .iter()
        .chain(hidden_markers.iter())
        .copied()
        .collect();
    assert_eq!(
        find_plaintext(store.root(), &all_markers).len(),
        11,
        "before adoption everything is readable"
    );

    // 1. adopt: encrypt everything visible, in place
    let report = store.fs.adopt_plaintext(NOW).unwrap();
    assert!(
        report.waiting.is_empty() && report.failed.is_empty(),
        "{report:?}"
    );
    assert_eq!(
        names(report.adopted.clone()),
        [
            "2026-07-03.md",
            "Back story.md",
            "Characters/Baron.md",
            "Characters/Strahd.md",
            "Images/Pasted image 1.png",
            "Locations/Vallaki.md",
            "Quests/shortage of wine.md",
            "Transcripts/2026-07-03.txt"
        ]
    );

    // 2. nothing readable left except the hidden files, which are untouched
    let mut index = open_index(&store);
    let stats = index_store(&store.fs, &mut index).unwrap();
    assert_eq!(stats.indexed.len(), 8);
    let leftover = find_plaintext(store.root(), &all_markers);
    let leftover_paths = paths_of(&leftover);
    assert_eq!(
        leftover_paths,
        BTreeSet::from([
            ".DS_Store".to_string(),
            ".claude/commands/x.md".to_string(),
            ".git/HEAD".to_string()
        ]),
        "only hidden files may stay readable (this scan also covers the index file and its WAL)"
    );
    for (path, text) in HIDDEN {
        assert_eq!(
            std::fs::read_to_string(store.root().join(path)).unwrap(),
            *text,
            "{path} must be untouched"
        );
    }

    // 3. every original round-trips byte for byte through the app's decrypting view
    for (path, text) in VISIBLE {
        assert_eq!(store.fs.read(&p(path)).unwrap(), text.as_bytes(), "{path}");
    }
    assert_eq!(
        store.fs.read(&p("Images/Pasted image 1.png")).unwrap(),
        IMAGE
    );

    // 4. backlinks, resolution and search give the right answers
    assert_eq!(
        backlink_sources(&index, "Locations/Vallaki.md"),
        [
            "Back story.md",
            "Characters/Baron.md",
            "Characters/Strahd.md",
            "Quests/shortage of wine.md"
        ]
    );
    assert_eq!(
        backlink_sources(&index, "Characters/Baron.md"),
        ["2026-07-03.md", "Back story.md", "Locations/Vallaki.md"],
        "by path, by bare name and by alias"
    );
    assert_eq!(
        backlink_sources(&index, "Characters/Strahd.md"),
        ["Locations/Vallaki.md", "Quests/shortage of wine.md"],
        "by name and by alias"
    );
    let image = index.backlinks(&p("Images/Pasted image 1.png")).unwrap();
    assert_eq!(
        (image.len(), image[0].from.as_str(), image[0].link.embed),
        (1, "Quests/shortage of wine.md", true)
    );
    let unresolved: Vec<_> = index
        .unresolved()
        .unwrap()
        .into_iter()
        .map(|u| u.link.target)
        .collect();
    assert_eq!(unresolved, ["Vistani"]);
    assert_eq!(
        names(index.notes_with_tag("character").unwrap()),
        ["Characters/Baron.md", "Characters/Strahd.md"]
    );
    assert_eq!(
        index.search("vallaki", 5).unwrap()[0].path.as_str(),
        "Locations/Vallaki.md"
    );
    assert_eq!(
        names(index.search("93be", 5).unwrap().into_iter().map(|h| h.path)),
        ["Quests/shortage of wine.md"]
    );
    assert!(
        index.search("spoken", 5).unwrap().is_empty(),
        "transcripts are attachments, not searchable notes"
    );
    // Back story 2 + Baron 1 + Strahd 1 + Vallaki 2 + the quest 3 + the session note 1
    assert_eq!(assert_backlinks_agree_with_resolution(&index), 10);

    // 5. running everything again changes nothing
    assert!(store.fs.adopt_plaintext(NOW).unwrap().adopted.is_empty());
    let again = index_store(&store.fs, &mut index).unwrap();
    assert!(again.indexed.is_empty() && again.removed.is_empty());
    assert_eq!(again.unchanged, 8);

    // 6. an edit through the app re-indexes only that note, and links follow
    store
        .fs
        .write(
            &p("Characters/Baron.md"),
            b"MARKER-BARON-EDITED-77aa moved away. No links now.",
        )
        .unwrap();
    let edit = index_store(&store.fs, &mut index).unwrap();
    assert_eq!(names(edit.indexed), ["Characters/Baron.md"]);
    assert_eq!(
        backlink_sources(&index, "Locations/Vallaki.md"),
        [
            "Back story.md",
            "Characters/Strahd.md",
            "Quests/shortage of wine.md"
        ]
    );

    // 7. a note dropped in later (the file-watcher case) is picked up by path
    std::fs::write(
        store.root().join("Locations/Blue Water Inn.md"),
        "MARKER-INN-d4e5 in [[Vallaki]]",
    )
    .unwrap();
    let late = store
        .fs
        .adopt_paths(&[p("Locations/Blue Water Inn.md")], NOW)
        .unwrap();
    assert_eq!(names(late.adopted), ["Locations/Blue Water Inn.md"]);
    assert_eq!(
        names(index_store(&store.fs, &mut index).unwrap().indexed),
        ["Locations/Blue Water Inn.md"]
    );
    assert_eq!(
        names(index.search("d4e5", 5).unwrap().into_iter().map(|h| h.path)),
        ["Locations/Blue Water Inn.md"]
    );
    assert!(
        backlink_sources(&index, "Locations/Vallaki.md")
            .contains(&"Locations/Blue Water Inn.md".to_string())
    );

    // 8. a rename in the app: the old path goes, the new one appears (rewriting links is M7)
    store
        .fs
        .rename(&p("Locations/Vallaki.md"), &p("Locations/Vallaki Town.md"))
        .unwrap();
    let renamed = index_store(&store.fs, &mut index).unwrap();
    assert_eq!(names(renamed.removed), ["Locations/Vallaki.md"]);
    assert_eq!(names(renamed.indexed), ["Locations/Vallaki Town.md"]);
    assert!(
        index
            .search("5d01", 5)
            .unwrap()
            .iter()
            .all(|h| h.path.as_str() == "Locations/Vallaki Town.md")
    );

    // 9. after all of that, still nothing readable outside the hidden files
    drop(index);
    let more_markers = [
        all_markers.as_slice(),
        &["MARKER-BARON-EDITED-77aa", "MARKER-INN-d4e5"],
    ]
    .concat();
    assert_eq!(
        paths_of(&find_plaintext(store.root(), &more_markers)),
        BTreeSet::from([
            ".DS_Store".to_string(),
            ".claude/commands/x.md".to_string(),
            ".git/HEAD".to_string()
        ])
    );
}

// ---- the real vault ------------------------------------------------------------------------

#[test]
#[ignore = "needs your notes: set FLINT_SAMPLE_VAULT"]
fn real_vault_gate() {
    let Some(vault) = std::env::var_os("FLINT_SAMPLE_VAULT") else {
        panic!("set FLINT_SAMPLE_VAULT to a copy of your vault")
    };
    let vault = std::path::PathBuf::from(vault);
    let store = TestStore::new();
    copy_dir_all(&vault, store.root()).unwrap();

    // what is in the vault: visible files, and the distinctive text in each, to scan for afterwards
    let mut visible: Vec<(StorePath, Vec<u8>)> = Vec::new();
    let mut hidden_files = 0usize;
    let mut needles: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(&vault)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let relative = entry.path().strip_prefix(&vault).unwrap();
        if is_hidden(relative) {
            hidden_files += 1;
            continue;
        }
        let bytes = std::fs::read(entry.path()).unwrap();
        if let Ok(text) = std::str::from_utf8(&bytes)
            && let Some(line) = text
                .lines()
                .map(str::trim)
                .find(|l| l.chars().count() >= 24)
        {
            needles.push(line.chars().take(40).collect());
        }
        visible.push((StorePath::new(relative.to_str().unwrap()).unwrap(), bytes));
    }

    let report = store.fs.adopt_plaintext(NOW).unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.waiting.is_empty(), "{:?}", report.waiting);
    assert_eq!(
        report.adopted.len(),
        visible.len(),
        "every visible file adopted"
    );

    let mut index = open_index(&store);
    let stats = index_store(&store.fs, &mut index).unwrap();
    assert_eq!(stats.indexed.len(), visible.len());

    // no plaintext outside hidden files; and every file round-trips exactly
    let needle_refs: Vec<&str> = needles.iter().map(String::as_str).collect();
    let leftover = find_plaintext(store.root(), &needle_refs);
    let visible_leftover: Vec<_> = leftover.iter().filter(|f| !is_hidden(&f.path)).collect();
    assert!(
        visible_leftover.is_empty(),
        "readable text left on disk: {visible_leftover:?}"
    );
    for (path, original) in &visible {
        assert_eq!(
            &store.fs.read(path).unwrap(),
            original,
            "{path} did not round-trip"
        );
    }
    let resolved = assert_backlinks_agree_with_resolution(&index);
    assert!(store.fs.adopt_plaintext(NOW).unwrap().adopted.is_empty());
    assert!(
        index_store(&store.fs, &mut index)
            .unwrap()
            .indexed
            .is_empty()
    );

    // ---- the report for the hand check ----
    let hidden_leftover: BTreeSet<String> = leftover
        .iter()
        .filter(|f| is_hidden(&f.path))
        .map(|f| {
            f.path
                .components()
                .next()
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let unresolved = index.unresolved().unwrap();
    let mut unresolved_names: Vec<_> = unresolved.iter().map(|u| u.link.target.clone()).collect();
    unresolved_names.sort();
    unresolved_names.dedup();
    println!("\n== M1 gate on {} ==", vault.display());
    println!(
        "{} visible files encrypted and round-tripped byte for byte; {hidden_files} hidden files left untouched",
        visible.len()
    );
    println!(
        "hidden folders still holding readable text (never synced, never touched): {hidden_leftover:?}"
    );
    println!(
        "{resolved} resolved links; {} unresolved links to {} names: {unresolved_names:?}",
        unresolved.len(),
        unresolved_names.len()
    );
    println!("tags: {:?}", index.tags().unwrap());

    let mut counted: Vec<(usize, StorePath)> = index
        .entries()
        .unwrap()
        .into_iter()
        .map(|e| (index.backlinks(&e.path).unwrap().len(), e.path))
        .collect();
    counted.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    println!(
        "\n-- backlinks for the {} most linked-to files: compare each with Obsidian's Backlinks pane --",
        counted.len().min(8)
    );
    for (count, path) in counted.iter().take(8) {
        println!("\n{path}  ({count} backlinks)");
        for back in index.backlinks(path).unwrap() {
            let source = String::from_utf8_lossy(&store.fs.read(&back.from).unwrap()).into_owned();
            let start = source.floor_char_boundary(back.link.range.start.saturating_sub(30));
            let end = source.ceil_char_boundary((back.link.range.end + 30).min(source.len()));
            println!(
                "   {:<28} …{}…",
                back.from,
                source[start..end].replace('\n', " ")
            );
        }
    }

    println!("\n-- search --");
    for entry in index
        .entries()
        .unwrap()
        .iter()
        .filter(|e| e.path.as_str().starts_with("Characters/"))
    {
        let name = entry.path.as_path().file_stem().unwrap();
        let hits: Vec<_> = index
            .search(name, 3)
            .unwrap()
            .into_iter()
            .map(|h| h.path.to_string())
            .collect();
        println!("{name:<20} -> {hits:?}");
    }
}
