//! Links, headings, tags and frontmatter from one markdown note. Pure: no I/O, never fails.
//!
//! [`parse`] takes the note's text and returns a [`ParsedNote`]. It is built on `pulldown-cmark`
//! (CommonMark plus wikilinks and YAML metadata blocks) and `serde-saphyr` (YAML), so links inside
//! code spans and fences are not links, tables work, and malformed input degrades instead of failing.
//!
//! Recognised, in Obsidian's dialect:
//!
//! - wikilinks `[[Note]]`, `[[Note|alias]]`, `[[Note#Heading]]`, `[[Note#^block]]`, `[[#Heading]]`
//! - embeds `![[image.png]]`, `![[image.png|200]]`, `![alt](image.png)`
//! - markdown links to notes `[text](other%20note.md#heading)` (web and `mailto:` links are skipped)
//! - inline `#tags` (including `#nested/tags`) and `tags:` in frontmatter
//! - YAML frontmatter, kept as JSON so it can be searched later with SQLite's JSON functions
//! - `aliases:` in frontmatter, for link resolution
//!
//! Not done here: resolving a link to a file (that needs the list of files, so it belongs to
//! `flint-index`), `%%comments%%`, and links inside frontmatter values.

mod frontmatter;
mod links;
mod tags;

use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, MetadataBlockKind, Options, Parser, Tag, TagEnd};

pub use links::{Fragment, Link, LinkKind};

/// Everything extracted from one note.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedNote {
    /// The YAML frontmatter as JSON, if there is a valid mapping.
    pub frontmatter: Option<serde_json::Value>,
    /// Why the frontmatter was ignored, if it was present but not usable (bad YAML, not a mapping).
    pub frontmatter_error: Option<String>,
    /// Other names the note answers to, from `aliases:`/`alias:`.
    pub aliases: Vec<String>,
    /// Frontmatter and inline tags without the leading `#`, sorted and de-duplicated.
    pub tags: Vec<String>,
    pub headings: Vec<Heading>,
    /// Wikilinks, markdown links and embeds in document order. Unresolved targets are included.
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

/// Parses one note. Always succeeds; unusable parts are left out (see [`ParsedNote::frontmatter_error`]).
/// Link ranges are byte offsets into `source`.
pub fn parse(source: &str) -> ParsedNote {
    // A byte-order mark would stop the frontmatter fence from being recognised at offset 0.
    let (bom, text) = match source.strip_prefix('\u{feff}') {
        Some(rest) => ('\u{feff}'.len_utf8(), rest),
        None => (0, source),
    };
    let mut options = Options::ENABLE_WIKILINKS | Options::ENABLE_TABLES;
    // Like Obsidian, frontmatter only counts when the very first line is `---`. (pulldown-cmark
    // would also accept it after a blank line.)
    if text.starts_with("---") {
        options |= Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    }

    let mut note = ParsedNote::default();
    let mut inline_tags = tags::TagCollector::default();
    let mut link_stack: Vec<links::Pending> = Vec::new();
    let mut heading: Option<(u8, String)> = None;
    let mut yaml = String::new();
    let mut in_yaml = false;
    let mut in_code_block = false;

    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        let range: Range<usize> = range.start + bom..range.end + bom;
        let is_text_for_tags =
            matches!(event, Event::Text(_)) && !in_code_block && !in_yaml && link_stack.is_empty();
        if !is_text_for_tags {
            inline_tags.flush();
        }

        match event {
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)) => in_yaml = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_yaml = false,
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level_number(level), String::new()))
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, text)) = heading.take() {
                    note.headings.push(Heading { level, text });
                }
            }
            Event::Start(tag @ (Tag::Link { .. } | Tag::Image { .. })) => {
                if let Some(pending) = links::Pending::start(&tag, range) {
                    link_stack.push(pending);
                } else {
                    link_stack.push(links::Pending::ignored());
                }
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some(link) = link_stack.pop().and_then(links::Pending::finish) {
                    note.links.push(link);
                }
            }
            Event::Text(text) => {
                if in_yaml {
                    yaml.push_str(&text);
                    continue;
                }
                for pending in &mut link_stack {
                    pending.text.push_str(&text);
                }
                if let Some((_, heading_text)) = &mut heading {
                    heading_text.push_str(&text);
                }
                if is_text_for_tags {
                    inline_tags.push(&text);
                }
            }
            Event::Code(code) => {
                for pending in &mut link_stack {
                    pending.text.push_str(&code);
                }
                if let Some((_, heading_text)) = &mut heading {
                    heading_text.push_str(&code);
                }
            }
            _ => {}
        }
    }
    inline_tags.flush();

    let mut tags = inline_tags.into_tags();
    match frontmatter::parse(&yaml) {
        Ok(Some(front)) => {
            tags.extend(front.tags);
            note.aliases = front.aliases;
            note.frontmatter = Some(front.value);
        }
        Ok(None) => {}
        Err(error) => note.frontmatter_error = Some(error),
    }
    note.tags = tags.into_iter().collect();
    note
}

fn level_number(level: HeadingLevel) -> u8 {
    level as u8
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    fn links(source: &str) -> Vec<Link> {
        parse(source).links
    }

    fn only_link(source: &str) -> Link {
        let mut found = links(source);
        assert_eq!(found.len(), 1, "{source:?} -> {found:?}");
        found.remove(0)
    }

    fn tags(source: &str) -> Vec<String> {
        parse(source).tags
    }

    // ---- wikilinks -------------------------------------------------------------------------

    #[test]
    fn plain_wikilink() {
        let link = only_link("see [[Vallaki]] for more");
        assert_eq!(link.kind, LinkKind::Wiki);
        assert!(!link.embed);
        assert_eq!(link.target, "Vallaki");
        assert_eq!(link.fragment, None);
        assert_eq!(link.display, None);
    }

    #[test]
    fn wikilink_forms() {
        let cases: [(&str, &str, Option<Fragment>, Option<&str>); 8] = [
            ("[[Note|alias]]", "Note", None, Some("alias")),
            (
                "[[Note#Heading]]",
                "Note",
                Some(Fragment::Heading("Heading".into())),
                None,
            ),
            (
                "[[Note#Heading|shown]]",
                "Note",
                Some(Fragment::Heading("Heading".into())),
                Some("shown"),
            ),
            (
                "[[Note#^abc123]]",
                "Note",
                Some(Fragment::Block("abc123".into())),
                None,
            ),
            (
                "[[#Same note heading]]",
                "",
                Some(Fragment::Heading("Same note heading".into())),
                None,
            ),
            (
                "[[Note#H1#H2]]",
                "Note",
                Some(Fragment::Heading("H1#H2".into())),
                None,
            ),
            ("[[Locations/Vallaki]]", "Locations/Vallaki", None, None),
            ("[[  Padded  ]]", "Padded", None, None),
        ];
        for (source, target, fragment, display) in cases {
            let link = only_link(source);
            assert_eq!(link.target, target, "{source}");
            assert_eq!(link.fragment, fragment, "{source}");
            assert_eq!(link.display.as_deref(), display, "{source}");
        }
    }

    #[test]
    fn unresolved_looking_targets_are_still_links() {
        let found = links("[[Vistani]] and [[Blood Spear of Kavan]] and [[Esmerlada D'avenir]]");
        let targets: Vec<_> = found.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(
            targets,
            ["Vistani", "Blood Spear of Kavan", "Esmerlada D'avenir"]
        );
    }

    #[test]
    fn links_come_in_document_order_across_blocks() {
        let found = links(
            "# H\n\n[[A]] text [[B]]\n\n- item [[C]]\n- [[D]]\n\n> quote [[E]]\n\n| t |\n|---|\n| [[F]] |\n",
        );
        let targets: Vec<_> = found.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["A", "B", "C", "D", "E", "F"]);
    }

    #[test]
    fn a_table_cell_may_escape_the_pipe() {
        let found = links("| a | b |\n|---|---|\n| [[Note\\|alias]] | [[Z]] |\n");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].target, "Note");
        assert_eq!(found[0].display.as_deref(), Some("alias"));
    }

    // ---- embeds ----------------------------------------------------------------------------

    #[test]
    fn embeds() {
        let image = only_link("![[Pasted image 20250723195845.png]]");
        assert!(image.embed);
        assert_eq!(image.target, "Pasted image 20250723195845.png");

        let sized = only_link("![[pic.png|200]]");
        assert!(sized.embed);
        assert_eq!(sized.display.as_deref(), Some("200"));

        let note_embed = only_link("![[Other note#Section]]");
        assert!(note_embed.embed);
        assert_eq!(
            note_embed.fragment,
            Some(Fragment::Heading("Section".into()))
        );

        let markdown = only_link("![a picture](images/my%20pic.png)");
        assert!(markdown.embed);
        assert_eq!(
            (markdown.kind, markdown.target.as_str()),
            (LinkKind::Markdown, "images/my pic.png")
        );
        assert_eq!(markdown.display.as_deref(), Some("a picture"));
    }

    // ---- markdown links --------------------------------------------------------------------

    #[test]
    fn markdown_links_to_notes() {
        let link = only_link("[the inn](Locations/Blue%20Water%20Inn.md#Rooms)");
        assert_eq!(link.kind, LinkKind::Markdown);
        assert_eq!(link.target, "Locations/Blue Water Inn.md");
        assert_eq!(link.fragment, Some(Fragment::Heading("Rooms".into())));
        assert_eq!(link.display.as_deref(), Some("the inn"));

        assert_eq!(only_link("[x](<with space.md>)").target, "with space.md");
        assert_eq!(only_link("[x](#Top)").target, "");
        assert_eq!(
            only_link("[x](#Top)").fragment,
            Some(Fragment::Heading("Top".into()))
        );
        assert_eq!(
            only_link("[ref link][r]\n\n[r]: Some%20Note.md").target,
            "Some Note.md"
        );
    }

    #[test]
    fn web_links_and_other_schemes_are_not_note_links() {
        for source in [
            "[web](https://example.com/a)",
            "[web](http://example.com)",
            "<https://example.com>",
            "<me@example.com>",
            "[mail](mailto:me@example.com)",
            "[app](obsidian://open?vault=x)",
            "[empty]()",
            "[nothing defined][nope]",
        ] {
            assert!(links(source).is_empty(), "{source}");
        }
    }

    // ---- what is not a link ----------------------------------------------------------------

    #[test]
    fn code_is_not_scanned() {
        assert!(links("`[[in code]]`").is_empty());
        assert!(links("```\n[[in fence]]\n```\n").is_empty());
        assert!(links("~~~md\n[[in tilde fence]]\n~~~\n").is_empty());
        assert!(links("    [[indented code]]\n").is_empty());
        assert_eq!(links("`[[no]]` but [[yes]]").len(), 1);
    }

    #[test]
    fn malformed_wikilinks_are_skipped() {
        for source in [
            "[[ ]]",
            "[[]]",
            "[[unclosed",
            "[[a\nb]]",
            "[[|alias]]",
            "[[#]]",
            "[ [x] ]",
        ] {
            assert!(links(source).is_empty(), "{source:?}");
        }
    }

    // ---- ranges ----------------------------------------------------------------------------

    #[test]
    fn ranges_slice_back_to_the_link_text() {
        let source = "Ünïcode before [[Note|alias]] then ![[pic.png]] and [x](y.md) end";
        let found = links(source);
        let texts: Vec<_> = found.iter().map(|l| &source[l.range.clone()]).collect();
        assert_eq!(texts, ["[[Note|alias]]", "![[pic.png]]", "[x](y.md)"]);
    }

    #[test]
    fn ranges_account_for_a_byte_order_mark() {
        let source = "\u{feff}---\ntitle: x\n---\nbody [[Note]]";
        let found = links(source);
        assert_eq!(&source[found[0].range.clone()], "[[Note]]");
    }

    // ---- tags ------------------------------------------------------------------------------

    #[test]
    fn inline_tags() {
        assert_eq!(
            tags("#character"),
            ["character"],
            "tag at the very start, no newline"
        );
        assert_eq!(
            tags("text #location and #quest/side, more"),
            ["location", "quest/side"]
        );
        assert_eq!(tags("#b #a #b"), ["a", "b"], "sorted and de-duplicated");
        assert_eq!(tags("(#paren) and \"#quoted\""), ["paren", "quoted"]);
        assert_eq!(tags("café #café-night #日本語"), ["café-night", "日本語"]);
        assert_eq!(tags("## Heading #in-heading"), ["in-heading"]);
        assert_eq!(tags("- list #item\n- other #other"), ["item", "other"]);
        assert_eq!(tags("**bold #tag**"), ["tag"]);
    }

    #[test]
    fn things_that_look_like_tags_but_are_not() {
        for source in [
            "not#tag",
            "a/#b",
            "http://example.com/page#anchor",
            "&#39;",
            "#1984",
            "# Heading with space",
            "price is #",
            "`#code`",
            "```\n#fenced\n```\n",
            "[[Note#Heading]]",
            "[[#Same]]",
            "[x](other.md#frag)",
        ] {
            assert!(tags(source).is_empty(), "{source:?} -> {:?}", tags(source));
        }
        assert_eq!(tags("#1984a"), ["1984a"], "a tag only needs one non-digit");
    }

    #[test]
    fn a_tag_split_by_a_stray_bracket_in_the_text_is_still_found() {
        assert_eq!(tags("see [this] and [that] #tag after brackets"), ["tag"]);
    }

    // ---- headings --------------------------------------------------------------------------

    #[test]
    fn headings() {
        let note = parse(
            "# One\n\ntext\n\n## Two with `code` and *emphasis*\n\nSetext\n======\n\n###### Six\n",
        );
        let got: Vec<_> = note
            .headings
            .iter()
            .map(|h| (h.level, h.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (1, "One"),
                (2, "Two with code and emphasis"),
                (1, "Setext"),
                (6, "Six")
            ]
        );
    }

    // ---- frontmatter -----------------------------------------------------------------------

    #[test]
    fn frontmatter_is_kept_as_json() {
        let note = parse(
            "---\ntitle: Baron\nlevel: 5\nalive: true\nhp: 12.5\ndate: 2026-07-03\nparty:\n  - Meryn\n  - Reena\nstats:\n  str: 10\n---\n\n# Body\n",
        );
        assert_eq!(note.frontmatter_error, None);
        assert_eq!(
            note.frontmatter,
            Some(json!({
                "title": "Baron", "level": 5, "alive": true, "hp": 12.5, "date": "2026-07-03",
                "party": ["Meryn", "Reena"], "stats": {"str": 10}
            }))
        );
        assert_eq!(
            note.headings.len(),
            1,
            "the body after the frontmatter is still parsed"
        );
    }

    #[test]
    fn frontmatter_tags_and_aliases() {
        let list = parse(
            "---\ntags: [a, \"#b\", c]\naliases:\n  - The Baron\n  - Baron Vallaki\n---\n#d\n",
        );
        assert_eq!(list.tags, ["a", "b", "c", "d"]);
        assert_eq!(list.aliases, ["The Baron", "Baron Vallaki"]);

        let strings = parse("---\ntags: one, two three\nalias: Smith, John\n---\n");
        assert_eq!(strings.tags, ["one", "three", "two"]);
        assert_eq!(
            strings.aliases,
            ["Smith, John"],
            "an alias string is one name, commas included"
        );

        let numbers = parse("---\ntags: [2026, 1984a]\naliases: [42]\n---\n");
        assert_eq!(
            numbers.tags,
            ["1984a", "2026"],
            "frontmatter tags are taken as written"
        );
        assert_eq!(numbers.aliases, ["42"]);

        assert_eq!(parse("---\ntag: single\n---\n").tags, ["single"]);
    }

    #[test]
    fn bad_frontmatter_is_reported_and_the_rest_still_parsed() {
        for yaml in [
            "title: [unclosed",
            "a: b: c: d",
            "key: 'unterminated",
            "\t- tab indented",
        ] {
            let note = parse(&format!("---\n{yaml}\n---\nbody [[Link]] #tag\n"));
            assert_eq!(note.frontmatter, None, "{yaml}");
            assert!(
                note.frontmatter_error.is_some(),
                "{yaml}: no error reported"
            );
            assert_eq!(note.links.len(), 1, "{yaml}");
            assert_eq!(note.tags, ["tag"], "{yaml}");
        }
    }

    #[test]
    fn frontmatter_that_is_not_a_mapping_is_an_error() {
        for yaml in ["- a\n- b", "just a string", "42"] {
            let note = parse(&format!("---\n{yaml}\n---\n"));
            assert_eq!(note.frontmatter, None, "{yaml}");
            assert_eq!(
                note.frontmatter_error.as_deref(),
                Some("frontmatter must be a mapping of keys to values"),
                "{yaml}"
            );
        }
    }

    #[test]
    fn empty_frontmatter_is_not_an_error() {
        for source in [
            "---\n---\nbody",
            "---\n\n---\nbody",
            "---\n# only a comment\n---\nbody",
        ] {
            let note = parse(source);
            assert_eq!(note.frontmatter, None, "{source:?}");
            assert_eq!(note.frontmatter_error, None, "{source:?}");
        }
    }

    #[test]
    fn frontmatter_only_counts_at_the_very_start() {
        for source in [
            "\n---\ntitle: x\n---\n",
            "text\n\n---\ntitle: x\n---\n",
            " ---\ntitle: x\n---\n",
        ] {
            let note = parse(source);
            assert_eq!(note.frontmatter, None, "{source:?}");
        }
        assert_eq!(
            parse("\u{feff}---\ntitle: x\n---\n").frontmatter,
            Some(json!({"title": "x"}))
        );
    }

    #[test]
    fn links_and_tags_inside_the_frontmatter_block_are_not_body_content() {
        let note = parse("---\nrelated: \"[[Linked]]\"\nnote: see #notatag\n---\nbody\n");
        assert!(note.links.is_empty());
        assert!(note.tags.is_empty());
        assert_eq!(note.frontmatter.unwrap()["related"], "[[Linked]]");
    }

    #[test]
    fn windows_line_endings_work() {
        let note = parse("---\r\ntags: [a]\r\n---\r\n# Head\r\n\r\n[[Note]] #b\r\n");
        assert_eq!(note.tags, ["a", "b"]);
        assert_eq!(note.links.len(), 1);
        assert_eq!(note.headings.len(), 1);
    }

    #[test]
    fn a_hostile_yaml_document_does_not_hang_or_blow_up() {
        // "billion laughs": nested anchors that expand exponentially
        let bomb = "---\na: &a [x,x,x,x,x,x,x,x,x]\nb: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a]\nc: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b]\nd: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c]\ne: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d]\nf: &f [*e,*e,*e,*e,*e,*e,*e,*e,*e]\n---\nbody [[Still]]\n";
        let started = std::time::Instant::now();
        let note = parse(bomb);
        assert!(
            started.elapsed().as_secs() < 5,
            "took {:?}",
            started.elapsed()
        );
        assert_eq!(note.links.len(), 1, "the body still parses");
        assert_eq!(note.frontmatter, None);
        let error = note.frontmatter_error.expect("the expansion is refused");
        assert!(error.contains("budget"), "{error}");
        assert!(!error.contains('\n'), "the message is a single line");
    }

    // ---- robustness ------------------------------------------------------------------------

    #[test]
    fn empty_and_trivial_input() {
        for source in ["", "\n", "   ", "---", "---\n", "#", "[[", "]]", "![["] {
            let _ = parse(source);
        }
        assert_eq!(parse(""), ParsedNote::default());
    }

    proptest! {
        #[test]
        fn never_panics_and_ranges_are_valid(source in "\\PC{0,200}|(\\[|\\]|!|#|\\||\\^|-|`|\n|[a-z ])*") {
            let note = parse(&source);
            for link in &note.links {
                prop_assert!(link.range.start < link.range.end && link.range.end <= source.len());
                let text = source.get(link.range.clone());
                prop_assert!(text.is_some(), "range {:?} is not on char boundaries", link.range);
                let text = text.unwrap();
                match link.kind {
                    LinkKind::Wiki => prop_assert!(text.starts_with("[[") || text.starts_with("![["), "{text:?}"),
                    LinkKind::Markdown => prop_assert!(text.starts_with('[') || text.starts_with("!["), "{text:?}"),
                }
                prop_assert!(!(link.target.is_empty() && link.fragment.is_none()));
            }
            prop_assert!(note.tags.windows(2).all(|w| w[0] < w[1]), "tags sorted and unique");
        }

        #[test]
        fn every_wikilink_we_generate_is_found(target in "[A-Za-z][A-Za-z0-9 ']{0,20}") {
            let target = target.trim().to_owned();
            let found = links(&format!("before [[{target}]] after"));
            prop_assert_eq!(found.len(), 1);
            prop_assert_eq!(&found[0].target, &target);
        }
    }

    /// Run with `FLINT_SAMPLE_VAULT=/path/to/vault cargo test -p flint-md-parse -- --ignored --nocapture`
    /// to parse every note in a real vault and print what was found.
    #[test]
    #[ignore = "needs a real vault; set FLINT_SAMPLE_VAULT"]
    fn parses_a_real_vault() {
        let Some(root) = std::env::var_os("FLINT_SAMPLE_VAULT") else {
            panic!("set FLINT_SAMPLE_VAULT to a folder of notes");
        };
        let (mut notes, mut wiki, mut embeds, mut markdown, mut headings) = (0, 0, 0, 0, 0);
        let mut all_tags = std::collections::BTreeSet::new();
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let hidden = path.file_name().unwrap().to_string_lossy().starts_with('.');
                if hidden {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "md") {
                    let note = parse(&std::fs::read_to_string(&path).unwrap());
                    notes += 1;
                    headings += note.headings.len();
                    all_tags.extend(note.tags);
                    for link in note.links {
                        match (link.kind, link.embed) {
                            (LinkKind::Wiki, false) => wiki += 1,
                            (_, true) => embeds += 1,
                            (LinkKind::Markdown, false) => markdown += 1,
                        }
                    }
                }
            }
        }
        println!(
            "{notes} notes, {headings} headings, {wiki} wikilinks, {embeds} embeds, {markdown} markdown links, tags: {all_tags:?}"
        );
    }
}
