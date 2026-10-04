use flint_core::StorePath;
use flint_md_parse::{Fragment, Heading};
use proptest::prelude::*;
use serde_json::json;
use tempfile::TempDir;

use crate::{Error, HIGHLIGHT_END, HIGHLIGHT_START, Index};

const KEY: [u8; 32] = [7; 32];

fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

fn index() -> Index {
    Index::open_in_memory(&KEY).unwrap()
}

fn add(index: &mut Index, path: &str, text: &str) {
    index.upsert_note(&p(path), text, "stamp").unwrap();
}

fn attach(index: &mut Index, path: &str) {
    index.upsert_attachment(&p(path), "stamp").unwrap();
}

fn names(paths: Vec<StorePath>) -> Vec<String> {
    paths.into_iter().map(|p| p.to_string()).collect()
}

fn resolve(index: &Index, from: &str, target: &str) -> Option<String> {
    index
        .resolve(&p(from), target)
        .unwrap()
        .map(|p| p.to_string())
}

fn backlinks(index: &Index, target: &str) -> Vec<String> {
    index
        .backlinks(&p(target))
        .unwrap()
        .into_iter()
        .map(|b| b.from.to_string())
        .collect()
}

/// A vault shaped like the real sample: folders, spaces and apostrophes, a dated note that also
/// exists as a transcript, an image, and an accented name.
fn vault() -> Index {
    let mut i = index();
    for note in [
        "Back story.md",
        "2026-07-03.md",
        "Characters/Baron.md",
        "Characters/Édward Black.md",
        "Characters/Strahd.md",
        "Locations/Vallaki.md",
        "Locations/Wizard's tower.md",
        "Quests/shortage of wine.md",
    ] {
        add(&mut i, note, "");
    }
    attach(&mut i, "Transcripts/2026-07-03.txt");
    attach(&mut i, "Images/Pasted image 20250723195845.png");
    i
}

// ---- the database itself ----------------------------------------------------------------------

#[test]
fn it_is_real_sqlcipher_with_fts5_and_json() {
    let index = index();
    let version: String = index
        .conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    assert!(version.starts_with("4."), "{version}");
    let n: i64 = index
        .conn
        .query_row("SELECT json_extract('{\"a\":[1,2]}', '$.a[1]')", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 2);
}

#[test]
fn the_file_on_disk_is_encrypted() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("index.db");
    let marker = "PLAINTEXT-MARKER-9d2f7a";
    {
        let mut index = Index::open(&db, &KEY).unwrap();
        index
            .upsert_note(
                &p("secret.md"),
                &format!("# {marker}\n\nbody {marker} [[{marker}]] #{marker}"),
                "s",
            )
            .unwrap();
    }
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let path = entry.unwrap().path();
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.starts_with(b"SQLite format 3"),
            "{path:?} is a plain SQLite file"
        );
        assert!(
            !bytes.windows(marker.len()).any(|w| w == marker.as_bytes()),
            "{path:?} contains the plaintext"
        );
    }
}

#[test]
fn reopening_with_the_same_key_keeps_the_data() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("index.db");
    Index::open(&db, &KEY)
        .unwrap()
        .upsert_note(&p("a.md"), "# Title\n[[b]]", "v1")
        .unwrap();
    let again = Index::open(&db, &KEY).unwrap();
    assert_eq!(again.entries().unwrap().len(), 1);
    assert_eq!(again.search("title", 5).unwrap().len(), 1);
}

#[test]
fn a_wrong_key_gives_an_empty_index_not_an_error_or_a_peek() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("index.db");
    Index::open(&db, &KEY)
        .unwrap()
        .upsert_note(&p("a.md"), "hello", "v1")
        .unwrap();

    let other = Index::open(&db, &[8; 32]).unwrap();
    assert!(
        other.entries().unwrap().is_empty(),
        "the old contents are gone, not readable"
    );
    drop(other);
    assert!(
        Index::open(&db, &KEY)
            .unwrap()
            .entries()
            .unwrap()
            .is_empty(),
        "and the old key no longer matches either"
    );
}

#[test]
fn a_file_that_is_not_a_database_is_replaced() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("index.db");
    std::fs::write(&db, b"definitely not sqlite, just some bytes ".repeat(100)).unwrap();
    let mut index = Index::open(&db, &KEY).unwrap();
    index.upsert_note(&p("a.md"), "works", "s").unwrap();
    assert_eq!(index.entries().unwrap().len(), 1);
}

#[test]
fn an_index_from_another_schema_version_is_rebuilt() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("index.db");
    {
        let mut index = Index::open(&db, &KEY).unwrap();
        index.upsert_note(&p("a.md"), "hello", "s").unwrap();
        index.conn.pragma_update(None, "user_version", 99).unwrap();
    }
    assert!(
        Index::open(&db, &KEY)
            .unwrap()
            .entries()
            .unwrap()
            .is_empty()
    );
}

// ---- files ------------------------------------------------------------------------------------

#[test]
fn entries_report_what_is_indexed_with_its_stamp() {
    let mut i = index();
    i.upsert_note(&p("b.md"), "x", "stamp-b").unwrap();
    i.upsert_attachment(&p("a/pic.png"), "stamp-pic").unwrap();
    let entries = i.entries().unwrap();
    let got: Vec<_> = entries
        .iter()
        .map(|e| (e.path.as_str(), e.stamp.as_str(), e.is_note))
        .collect();
    assert_eq!(
        got,
        [("a/pic.png", "stamp-pic", false), ("b.md", "stamp-b", true)]
    );

    i.upsert_note(&p("b.md"), "changed", "stamp-b2").unwrap();
    assert_eq!(i.entries().unwrap()[1].stamp, "stamp-b2");
    assert_eq!(i.entries().unwrap().len(), 2);
}

#[test]
fn notes_and_attachments_are_told_apart_by_extension() {
    let mut i = index();
    assert!(matches!(
        i.upsert_note(&p("pic.png"), "x", "s"),
        Err(Error::WrongKind(_))
    ));
    assert!(matches!(
        i.upsert_attachment(&p("note.md"), "s"),
        Err(Error::WrongKind(_))
    ));
    add(&mut i, "UPPER.MD", "ok");
    assert!(i.entries().unwrap()[0].is_note);
}

#[test]
fn removing_a_note_removes_everything_about_it() {
    let mut i = index();
    add(
        &mut i,
        "a.md",
        "---\ntags: [t]\naliases: [al]\n---\n# H\n[[b]] #inline words",
    );
    i.remove(&p("a.md")).unwrap();
    for table in [
        "files", "notes", "headings", "tags", "aliases", "links", "note_fts",
    ] {
        let n: i64 = i
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table} still has rows");
    }
    i.remove(&p("never-existed.md")).unwrap();
}

#[test]
fn reindexing_replaces_instead_of_accumulating() {
    let mut i = index();
    add(&mut i, "a.md", "# Old\n[[x]] [[y]] #old oldword");
    add(&mut i, "a.md", "# New\n[[z]] #new");
    let info = i.note(&p("a.md")).unwrap().unwrap();
    assert_eq!(
        info.headings,
        [Heading {
            level: 1,
            text: "New".into()
        }]
    );
    assert_eq!(info.tags, ["new"]);
    assert_eq!(i.outgoing(&p("a.md")).unwrap().len(), 1);
    assert!(i.search("oldword", 5).unwrap().is_empty());
    assert_eq!(i.search("new", 5).unwrap().len(), 1);
}

// ---- resolution -------------------------------------------------------------------------------

#[test]
fn names_resolve_ignoring_case_and_md() {
    let i = vault();
    let from = "Back story.md";
    for target in [
        "Vallaki",
        "vallaki",
        "VALLAKI",
        "Vallaki.md",
        "vallaki.MD",
        "Locations/Vallaki",
        "locations/vallaki.md",
        "/Locations/Vallaki",
        "  Vallaki  ",
    ] {
        assert_eq!(
            resolve(&i, from, target).as_deref(),
            Some("Locations/Vallaki.md"),
            "{target:?}"
        );
    }
    assert_eq!(
        resolve(&i, from, "Back story").as_deref(),
        Some("Back story.md")
    );
    assert_eq!(
        resolve(&i, from, "Wizard's tower").as_deref(),
        Some("Locations/Wizard's tower.md")
    );
    assert_eq!(
        resolve(&i, from, "shortage of wine").as_deref(),
        Some("Quests/shortage of wine.md")
    );
}

#[test]
fn case_insensitivity_covers_non_ascii_names() {
    let i = vault();
    for target in ["Édward Black", "édward black", "ÉDWARD BLACK"] {
        assert_eq!(
            resolve(&i, "Back story.md", target).as_deref(),
            Some("Characters/Édward Black.md"),
            "{target}"
        );
    }
}

#[test]
fn a_name_must_match_the_end_of_the_path() {
    let i = vault();
    assert_eq!(
        resolve(&i, "Back story.md", "Characters/Vallaki"),
        None,
        "right name, wrong folder"
    );
    assert_eq!(resolve(&i, "Back story.md", "Vallaki/Locations"), None);
    assert_eq!(
        resolve(&i, "Back story.md", "ocations/Vallaki"),
        None,
        "a partial folder name is not a match"
    );
    assert_eq!(resolve(&i, "Back story.md", "Nowhere/Vallaki"), None);
}

#[test]
fn a_bare_name_means_a_note_but_an_extension_means_that_file() {
    let i = vault();
    // The real vault has both 2026-07-03.md and Transcripts/2026-07-03.txt
    assert_eq!(
        resolve(&i, "Back story.md", "2026-07-03").as_deref(),
        Some("2026-07-03.md")
    );
    assert_eq!(
        resolve(&i, "Back story.md", "2026-07-03.md").as_deref(),
        Some("2026-07-03.md")
    );
    assert_eq!(
        resolve(&i, "Back story.md", "2026-07-03.txt").as_deref(),
        Some("Transcripts/2026-07-03.txt")
    );
    assert_eq!(
        resolve(&i, "Back story.md", "Pasted image 20250723195845.png").as_deref(),
        Some("Images/Pasted image 20250723195845.png")
    );
    assert_eq!(
        resolve(&i, "Back story.md", "pasted IMAGE 20250723195845.PNG").as_deref(),
        Some("Images/Pasted image 20250723195845.png")
    );
    assert_eq!(
        resolve(&i, "Back story.md", "Pasted image 20250723195845"),
        None,
        "a bare name never picks an image"
    );
    assert_eq!(
        resolve(&i, "Back story.md", "Pasted image 20250723195845.png.md"),
        None,
        ".md means a note"
    );
}

#[test]
fn ambiguous_names_pick_the_shallowest_then_shortest_then_alphabetical() {
    let mut i = index();
    for path in [
        "Note.md",
        "A/Note.md",
        "B/Deep/Note.md",
        "X/Dup.md",
        "Y/Dup.md",
        "Z/Very/Deep/Only.md",
        "Z/Other/Only.md",
    ] {
        add(&mut i, path, "");
    }
    let r = |t: &str| resolve(&i, "Note.md", t);
    assert_eq!(
        r("Note").as_deref(),
        Some("Note.md"),
        "the root one is an exact path match"
    );
    assert_eq!(r("A/Note").as_deref(), Some("A/Note.md"));
    assert_eq!(
        r("Deep/Note").as_deref(),
        Some("B/Deep/Note.md"),
        "a partial path picks the files ending that way"
    );
    assert_eq!(
        r("Dup").as_deref(),
        Some("X/Dup.md"),
        "same depth and length: alphabetical"
    );
    assert_eq!(
        r("Only").as_deref(),
        Some("Z/Other/Only.md"),
        "fewer folders wins"
    );

    let mut j = index();
    add(&mut j, "A/Note.md", "");
    add(&mut j, "B/Deep/Note.md", "");
    assert_eq!(
        resolve(&j, "A/Note.md", "Note").as_deref(),
        Some("A/Note.md"),
        "no root note: the shallowest"
    );
}

#[test]
fn aliases_are_the_fallback_after_file_names() {
    let mut i = index();
    add(
        &mut i,
        "Characters/Strahd.md",
        "---\naliases: [Count von Zarovich, The Devil]\n---\n",
    );
    add(
        &mut i,
        "Other.md",
        "---\naliases: [Strahd, Other One]\n---\n",
    );
    add(&mut i, "Zed.md", "---\naliases: [The Devil]\n---\n");
    add(&mut i, "Characters/Abe.md", "---\naliases: [Twin]\n---\n");
    add(&mut i, "Characters/Bob.md", "---\naliases: [Twin]\n---\n");
    assert_eq!(
        resolve(&i, "Other.md", "count von zarovich").as_deref(),
        Some("Characters/Strahd.md")
    );
    assert_eq!(
        resolve(&i, "Other.md", "The Devil").as_deref(),
        Some("Zed.md"),
        "alias clash: the shallowest note wins"
    );
    assert_eq!(
        resolve(&i, "Other.md", "Twin").as_deref(),
        Some("Characters/Abe.md"),
        "same depth: alphabetical"
    );
    assert_eq!(
        resolve(&i, "Other.md", "Strahd").as_deref(),
        Some("Characters/Strahd.md"),
        "a real file name beats an alias"
    );
    assert_eq!(resolve(&i, "Other.md", "Nobody"), None);
}

#[test]
fn an_empty_target_is_the_same_note() {
    let i = vault();
    assert_eq!(
        resolve(&i, "Back story.md", "").as_deref(),
        Some("Back story.md")
    );
    assert_eq!(
        resolve(&i, "Back story.md", "   ").as_deref(),
        Some("Back story.md")
    );
}

#[test]
fn resolution_follows_the_vault_as_it_changes_with_no_reindexing_of_the_linking_note() {
    let mut i = index();
    add(&mut i, "a.md", "see [[b]] and [[Wanted]]");
    assert_eq!(resolve(&i, "a.md", "b"), None);

    add(&mut i, "folder/b.md", "");
    assert_eq!(resolve(&i, "a.md", "b").as_deref(), Some("folder/b.md"));
    assert_eq!(
        i.outgoing(&p("a.md")).unwrap()[0].resolved,
        Some(p("folder/b.md"))
    );

    add(&mut i, "b.md", ""); // a shallower b appears: links now prefer it
    assert_eq!(resolve(&i, "a.md", "b").as_deref(), Some("b.md"));

    i.remove(&p("b.md")).unwrap();
    i.remove(&p("folder/b.md")).unwrap();
    assert_eq!(resolve(&i, "a.md", "b"), None);
}

// ---- outgoing / unresolved --------------------------------------------------------------------

#[test]
fn outgoing_lists_links_in_order_with_where_they_point() {
    let mut i = vault();
    add(
        &mut i,
        "Quests/q.md",
        "[[Strahd|the count]] then [[Nobody#Part]] and ![[Pasted image 20250723195845.png]] and [[#Same]]",
    );
    let out = i.outgoing(&p("Quests/q.md")).unwrap();
    let got: Vec<_> = out
        .iter()
        .map(|r| {
            (
                r.link.target.as_str(),
                r.resolved.as_ref().map(|p| p.as_str()),
                r.link.embed,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("Strahd", Some("Characters/Strahd.md"), false),
            ("Nobody", None, false),
            (
                "Pasted image 20250723195845.png",
                Some("Images/Pasted image 20250723195845.png"),
                true
            ),
            ("", Some("Quests/q.md"), false),
        ]
    );
    assert_eq!(out[0].link.display.as_deref(), Some("the count"));
    assert_eq!(out[1].link.fragment, Some(Fragment::Heading("Part".into())));
    assert!(i.outgoing(&p("nonexistent.md")).unwrap().is_empty());
}

#[test]
fn unresolved_links_are_kept_and_listed() {
    let mut i = vault();
    add(&mut i, "a.md", "[[Vistani]] [[Strahd]] [[Ireena]]");
    add(&mut i, "b.md", "[[Vistani]] ![[missing.png]] [[#local]]");
    let missing: Vec<_> = i
        .unresolved()
        .unwrap()
        .into_iter()
        .map(|b| (b.from.to_string(), b.link.target))
        .collect();
    assert_eq!(
        missing,
        [
            ("a.md".into(), "Vistani".into()),
            ("a.md".into(), "Ireena".into()),
            ("b.md".into(), "Vistani".into()),
            ("b.md".into(), "missing.png".into())
        ]
    );
}

// ---- backlinks --------------------------------------------------------------------------------

#[test]
fn backlinks_find_every_link_that_resolves_to_a_note() {
    let mut i = index();
    add(
        &mut i,
        "Locations/Vallaki.md",
        "I link to [[Vallaki#Self]] myself",
    );
    add(&mut i, "a.md", "go to [[Vallaki]]");
    add(
        &mut i,
        "z.md",
        "[[vallaki|town]] and later [[Locations/Vallaki#Inn]]",
    );
    add(&mut i, "m.md", "[[Strahd]] only");

    let found = i.backlinks(&p("Locations/Vallaki.md")).unwrap();
    let got: Vec<_> = found
        .iter()
        .map(|b| {
            (
                b.from.as_str(),
                b.link.target.as_str(),
                b.link.display.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("a.md", "Vallaki", None),
            ("z.md", "vallaki", Some("town")),
            ("z.md", "Locations/Vallaki", None)
        ]
    );
    assert_eq!(
        found[2].link.fragment,
        Some(Fragment::Heading("Inn".into()))
    );
    assert!(
        !backlinks(&i, "Locations/Vallaki.md").contains(&"Locations/Vallaki.md".to_string()),
        "self links are not backlinks"
    );
    assert!(i.backlinks(&p("nope.md")).unwrap().is_empty());
}

#[test]
fn backlinks_carry_the_range_for_showing_context() {
    let mut i = index();
    add(&mut i, "t.md", "");
    let text = "Ünïcode lead-in, then [[t]] in the text.";
    add(&mut i, "a.md", text);
    let range = i.backlinks(&p("t.md")).unwrap()[0].link.range.clone();
    assert_eq!(&text[range], "[[t]]");
}

#[test]
fn backlinks_through_aliases() {
    let mut i = index();
    add(
        &mut i,
        "Strahd.md",
        "---\naliases: [Count von Zarovich]\n---\n",
    );
    add(
        &mut i,
        "a.md",
        "[[Count von Zarovich]] and [[Strahd]] and [[count von zarovich|him]]",
    );
    assert_eq!(backlinks(&i, "Strahd.md"), ["a.md", "a.md", "a.md"]);
}

#[test]
fn a_link_counts_only_for_the_note_it_resolves_to() {
    let mut i = index();
    for path in ["Note.md", "A/Note.md", "B/Deep/Note.md"] {
        add(&mut i, path, "");
    }
    add(&mut i, "from.md", "[[Note]] [[A/Note]] [[Deep/Note]]");
    assert_eq!(backlinks(&i, "Note.md"), ["from.md"]);
    assert_eq!(backlinks(&i, "A/Note.md"), ["from.md"]);
    assert_eq!(backlinks(&i, "B/Deep/Note.md"), ["from.md"]);
    // each of the three links appears for exactly one note, never two
    let total: usize = ["Note.md", "A/Note.md", "B/Deep/Note.md"]
        .iter()
        .map(|n| i.backlinks(&p(n)).unwrap().len())
        .sum();
    assert_eq!(total, 3);
}

#[test]
fn attachments_have_backlinks_from_embeds() {
    let mut i = vault();
    add(
        &mut i,
        "Quests/shortage of wine.md",
        "![[Pasted image 20250723195845.png]]",
    );
    add(
        &mut i,
        "Back story.md",
        "[[Pasted image 20250723195845.png|link not embed]]",
    );
    let found = i
        .backlinks(&p("Images/Pasted image 20250723195845.png"))
        .unwrap();
    let got: Vec<_> = found
        .iter()
        .map(|b| (b.from.as_str(), b.link.embed))
        .collect();
    assert_eq!(
        got,
        [
            ("Back story.md", false),
            ("Quests/shortage of wine.md", true)
        ]
    );
}

#[test]
fn backlinks_follow_the_vault_as_it_changes() {
    let mut i = index();
    add(&mut i, "a.md", "links to [[b]]");
    assert!(i.unresolved().unwrap().iter().any(|u| u.link.target == "b"));

    add(&mut i, "b.md", ""); // b appears; a.md is not touched
    assert_eq!(backlinks(&i, "b.md"), ["a.md"]);
    assert!(i.unresolved().unwrap().is_empty());

    i.remove(&p("b.md")).unwrap();
    assert!(i.unresolved().unwrap().iter().any(|u| u.link.target == "b"));

    // a rename is remove + add: links by name follow the note to its new folder
    add(&mut i, "old/b.md", "");
    i.remove(&p("old/b.md")).unwrap();
    add(&mut i, "new/b.md", "");
    assert_eq!(backlinks(&i, "new/b.md"), ["a.md"]);
}

// ---- search -----------------------------------------------------------------------------------

#[test]
fn search_finds_words_in_the_body() {
    let mut i = index();
    add(&mut i, "a.md", "The wizard visited the vineyard at dawn.");
    add(&mut i, "b.md", "Nothing relevant here.");
    let hits = i.search("vineyard", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path.as_str(), "a.md");
}

#[test]
fn search_covers_names_headings_tags_and_frontmatter() {
    let mut i = index();
    add(&mut i, "Characters/Rictavio.md", "plain body");
    add(&mut i, "h.md", "# Marvelous Heading\n\nbody");
    add(&mut i, "t.md", "text #uniquetag here");
    add(
        &mut i,
        "f.md",
        "---\nfaction: Keepers of the Feather\n---\nbody",
    );
    for (query, expected) in [
        ("rictavio", "Characters/Rictavio.md"),
        ("marvelous", "h.md"),
        ("uniquetag", "t.md"),
        ("feather", "f.md"),
        ("characters rictavio", "Characters/Rictavio.md"),
    ] {
        let hits: Vec<_> = i
            .search(query, 10)
            .unwrap()
            .into_iter()
            .map(|h| h.path.to_string())
            .collect();
        assert_eq!(hits, [expected], "{query}");
    }
}

#[test]
fn search_is_case_and_accent_insensitive_with_prefix_matching() {
    let mut i = index();
    add(&mut i, "a.md", "A quiet café near the Esmerlada vineyard");
    for query in [
        "CAFE",
        "café",
        "Café",
        "esmer",
        "vine",
        "quiet cafe near",
        "the esm",
    ] {
        assert_eq!(i.search(query, 10).unwrap().len(), 1, "{query}");
    }
    assert!(i.search("esmerladas", 10).unwrap().is_empty());
    assert!(
        i.search("quiet missingword", 10).unwrap().is_empty(),
        "words are ANDed"
    );
}

#[test]
fn search_input_is_text_never_query_syntax() {
    let mut i = index();
    add(&mut i, "a.md", "plain words and OR AND NOT here");
    for query in [
        "\"",
        "\"unbalanced",
        "a OR",
        "NOT",
        "(",
        ")",
        "*",
        "foo:bar",
        "col:",
        "^x",
        "a AND (b OR c",
        "-minus",
        "NEAR(a b)",
        "'; DROP TABLE notes; --",
        "\u{0}",
    ] {
        i.search(query, 10)
            .unwrap_or_else(|e| panic!("{query:?}: {e}"));
    }
    assert_eq!(
        i.search("OR AND NOT", 10).unwrap().len(),
        1,
        "operators are searched as words"
    );
    assert!(i.search("", 10).unwrap().is_empty());
    assert!(i.search("   ", 10).unwrap().is_empty());
}

#[test]
fn a_name_match_outranks_a_body_mention() {
    let mut i = index();
    add(
        &mut i,
        "mentions.md",
        "strahd strahd strahd strahd strahd is talked about at length here",
    );
    add(&mut i, "Characters/Strahd.md", "A vampire.");
    let hits = i.search("strahd", 10).unwrap();
    assert_eq!(hits[0].path.as_str(), "Characters/Strahd.md");
    assert_eq!(hits.len(), 2);
}

#[test]
fn snippets_mark_the_matches() {
    let mut i = index();
    add(
        &mut i,
        "a.md",
        "A long note about wine. The vineyard produces a red wine every autumn.",
    );
    let snippet = &i.search("vineyard", 10).unwrap()[0].snippet;
    assert!(
        snippet.contains(&format!("{HIGHLIGHT_START}vineyard{HIGHLIGHT_END}")),
        "{snippet:?}"
    );
}

#[test]
fn limit_and_removal() {
    let mut i = index();
    for n in 0..5 {
        add(&mut i, &format!("n{n}.md"), "shared word");
    }
    assert_eq!(i.search("shared", 3).unwrap().len(), 3);
    assert_eq!(i.search("shared", 100).unwrap().len(), 5);
    assert!(i.search("shared", 0).unwrap().is_empty());
    i.remove(&p("n0.md")).unwrap();
    assert_eq!(i.search("shared", 100).unwrap().len(), 4);
}

// ---- stored note information ------------------------------------------------------------------

#[test]
fn note_info_round_trips() {
    let mut i = index();
    add(
        &mut i,
        "a.md",
        "---\ntitle: Baron\nlevel: 5\ntags: [npc, Vallaki]\naliases: [The Baron]\n---\n# One\n\n## Two\n\ntext #inline #NPC",
    );
    let info = i.note(&p("a.md")).unwrap().unwrap();
    assert_eq!(
        info.frontmatter,
        Some(
            json!({"title": "Baron", "level": 5, "tags": ["npc", "Vallaki"], "aliases": ["The Baron"]})
        )
    );
    assert_eq!(info.frontmatter_error, None);
    assert_eq!(info.aliases, ["The Baron"]);
    assert_eq!(
        info.headings,
        [
            Heading {
                level: 1,
                text: "One".into()
            },
            Heading {
                level: 2,
                text: "Two".into()
            }
        ]
    );
    assert_eq!(
        info.tags.len(),
        3,
        "NPC and npc are one tag: {:?}",
        info.tags
    );
    assert!(i.note(&p("missing.md")).unwrap().is_none());
}

#[test]
fn bad_frontmatter_is_recorded_not_fatal() {
    let mut i = index();
    add(&mut i, "a.md", "---\nkey: [unclosed\n---\nbody [[b]]");
    let info = i.note(&p("a.md")).unwrap().unwrap();
    assert_eq!(info.frontmatter, None);
    assert!(info.frontmatter_error.is_some());
    assert_eq!(i.outgoing(&p("a.md")).unwrap().len(), 1);
}

#[test]
fn frontmatter_is_queryable_with_sqlite_json_functions() {
    let mut i = index();
    add(
        &mut i,
        "a.md",
        "---\nlevel: 5\nstats:\n  str: 10\nparty: [x, y]\n---\n",
    );
    add(&mut i, "b.md", "---\nlevel: 7\n---\n");
    add(&mut i, "c.md", "no frontmatter");
    let sql = |q: &str| -> Vec<String> {
        let mut s = i.conn.prepare(q).unwrap();
        s.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(
        sql("SELECT path FROM notes WHERE json_extract(frontmatter, '$.level') >= 5 ORDER BY path"),
        ["a.md", "b.md"]
    );
    assert_eq!(
        sql("SELECT path FROM notes WHERE json_extract(frontmatter, '$.stats.str') = 10"),
        ["a.md"]
    );
    assert_eq!(
        sql(
            "SELECT notes.path FROM notes, json_each(notes.frontmatter, '$.party') WHERE json_each.value = 'y'"
        ),
        ["a.md"]
    );
}

#[test]
fn tags_across_the_store() {
    let mut i = index();
    add(&mut i, "a.md", "#character and #Location");
    add(&mut i, "b.md", "---\ntags: [character]\n---\n#quest");
    add(&mut i, "c.md", "#CHARACTER");
    assert_eq!(
        names(i.notes_with_tag("character").unwrap()),
        ["a.md", "b.md", "c.md"]
    );
    assert_eq!(
        names(i.notes_with_tag("#Character").unwrap()),
        ["a.md", "b.md", "c.md"],
        "leading # and case are ignored"
    );
    assert_eq!(names(i.notes_with_tag("quest").unwrap()), ["b.md"]);
    assert!(i.notes_with_tag("nothing").unwrap().is_empty());
    let tags = i.tags().unwrap();
    let got: Vec<_> = tags.iter().map(|(t, n)| (t.to_lowercase(), *n)).collect();
    assert_eq!(
        got,
        [
            ("character".into(), 3),
            ("location".into(), 1),
            ("quest".into(), 1)
        ]
    );
}

// ---- the invariant that ties backlinks and resolution together --------------------------------

/// Every link that resolves to some file other than its own note is a backlink of exactly that file.
fn check_backlinks_agree_with_resolution(index: &Index) {
    let mut resolved_links = 0;
    for entry in index.entries().unwrap().iter().filter(|e| e.is_note) {
        for out in index.outgoing(&entry.path).unwrap() {
            if let Some(target) = &out.resolved
                && target != &entry.path
                && !out.link.target.is_empty()
            {
                resolved_links += 1;
                let back = index.backlinks(target).unwrap();
                assert!(
                    back.iter()
                        .any(|b| b.from == entry.path && b.link == out.link),
                    "{} -> {} ({:?}) is missing from the backlinks of {target}",
                    entry.path,
                    out.link.target,
                    out.link.range
                );
            }
        }
    }
    let total: usize = index
        .entries()
        .unwrap()
        .iter()
        .map(|e| index.backlinks(&e.path).unwrap().len())
        .sum();
    assert_eq!(
        total, resolved_links,
        "backlinks and resolved links must be the same set"
    );
}

proptest! {
    #[test]
    fn backlinks_and_resolution_always_agree(
        files in proptest::collection::btree_set((prop::sample::select(vec!["", "x/", "y/", "x/z/"]), prop::sample::select(vec!["a", "b", "c", "D"])), 1..8),
        link_choices in proptest::collection::vec(proptest::collection::vec(prop::sample::select(vec!["a", "b", "c", "d", "x/a", "y/b", "z/a", "a.md", "nope", "x/z/D"]), 0..5), 8),
    ) {
        let mut i = index();
        for (n, (folder, stem)) in files.iter().enumerate() {
            let links: String = link_choices[n % link_choices.len()].iter().map(|t| format!("[[{t}]] ")).collect();
            add(&mut i, &format!("{folder}{stem}.md"), &links);
        }
        check_backlinks_agree_with_resolution(&i);
    }
}

#[test]
fn the_same_invariant_on_a_realistic_vault() {
    let mut i = vault();
    add(
        &mut i,
        "Quests/q.md",
        "[[Strahd]] [[Vallaki]] [[Vistani]] [[Characters/Baron]] ![[Pasted image 20250723195845.png]] [[2026-07-03]] [[#h]]",
    );
    add(
        &mut i,
        "Characters/Strahd.md",
        "---\naliases: [Count]\n---\n[[Count]] [[Vallaki]] [[Baron]]",
    );
    add(
        &mut i,
        "Locations/Vallaki.md",
        "[[Strahd]] [[Count]] [[Quests/shortage of wine]]",
    );
    check_backlinks_agree_with_resolution(&i);
}

/// Run with `FLINT_SAMPLE_VAULT=/path/to/vault cargo test -p flint-index -- --ignored --nocapture`.
#[test]
#[ignore = "needs a real vault; set FLINT_SAMPLE_VAULT"]
fn indexes_a_real_vault() {
    let Some(root) = std::env::var_os("FLINT_SAMPLE_VAULT") else {
        panic!("set FLINT_SAMPLE_VAULT")
    };
    let root = std::path::PathBuf::from(root);
    let mut i = index();
    let mut stack = vec![root.clone()];
    let (mut notes, mut attachments) = (0, 0);
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().unwrap().to_string_lossy().starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative =
                StorePath::new(path.strip_prefix(&root).unwrap().to_str().unwrap()).unwrap();
            if path.extension().is_some_and(|e| e == "md") {
                i.upsert_note(&relative, &std::fs::read_to_string(&path).unwrap(), "s")
                    .unwrap();
                notes += 1;
            } else {
                i.upsert_attachment(&relative, "s").unwrap();
                attachments += 1;
            }
        }
    }
    check_backlinks_agree_with_resolution(&i);
    let unresolved = i.unresolved().unwrap();
    let mut distinct: Vec<_> = unresolved.iter().map(|u| u.link.target.clone()).collect();
    distinct.sort();
    distinct.dedup();
    println!(
        "{notes} notes, {attachments} other files, {} unresolved links to {} names: {distinct:?}",
        unresolved.len(),
        distinct.len()
    );
    println!("tags: {:?}", i.tags().unwrap());
    println!(
        "search 'vallaki': {:?}",
        i.search("vallaki", 5)
            .unwrap()
            .iter()
            .map(|h| h.path.to_string())
            .collect::<Vec<_>>()
    );
}
