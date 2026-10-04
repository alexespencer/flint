use std::ops::Range;
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use pulldown_cmark::{LinkType, Tag};
use regex::Regex;

/// A link, or an embed, from one note to a target that may or may not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// `![[x]]` or `![alt](x)`: the target is shown inline instead of linked.
    pub embed: bool,
    /// The note or file named, as written but trimmed and percent-decoded: `Note`, `Locations/Vallaki`,
    /// `pic.png`, `other note.md`. Empty for a link to a heading in the same note (`[[#Heading]]`).
    pub target: String,
    pub fragment: Option<Fragment>,
    /// The text after the `|` in a wikilink (an alias, or the size of an embedded image), or the
    /// visible text of a markdown link.
    pub display: Option<String>,
    /// Byte range of the whole link in the source text, e.g. for showing context in a backlinks pane.
    pub range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `[[...]]`
    Wiki,
    /// `[text](...)`
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fragment {
    /// `#Heading` (nested headings stay as written: `#H1#H2`)
    Heading(String),
    /// `#^block-id`
    Block(String),
}

/// A link that has started but whose text has not been seen yet.
pub(crate) struct Pending {
    link: Option<Link>,
    has_display: bool,
    pub text: String,
}

// RFC 3986 scheme: `https:`, `mailto:`, `obsidian:`, ...
static SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").expect("valid regex"));

impl Pending {
    /// A link we do not record (web links, autolinks, empty targets) still needs a stack slot so
    /// its closing event pops the right entry.
    pub fn ignored() -> Self {
        Self {
            link: None,
            has_display: false,
            text: String::new(),
        }
    }

    pub fn start(tag: &Tag<'_>, range: Range<usize>) -> Option<Self> {
        let (link_type, dest, embed) = match tag {
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => (*link_type, dest_url.as_ref(), false),
            Tag::Image {
                link_type,
                dest_url,
                ..
            } => (*link_type, dest_url.as_ref(), true),
            _ => return None,
        };
        let (kind, raw, has_display) = match link_type {
            LinkType::WikiLink { has_pothole } => {
                // An escaped pipe inside a table cell (`[[Note\|alias]]`) leaves the backslash on the target.
                (LinkKind::Wiki, dest.trim_end_matches('\\'), has_pothole)
            }
            LinkType::Autolink | LinkType::Email => return None,
            _ => {
                if dest.is_empty() || SCHEME.is_match(dest) {
                    return None;
                }
                (LinkKind::Markdown, dest, true)
            }
        };
        // Obsidian does not treat a "link" that spans lines as one.
        if raw.contains(['\n', '\r']) {
            return None;
        }
        let (target, fragment) = split(raw, kind);
        if target.is_empty() && fragment.is_none() {
            return None;
        }
        Some(Self {
            link: Some(Link {
                kind,
                embed,
                target,
                fragment,
                display: None,
                range,
            }),
            has_display,
            text: String::new(),
        })
    }

    pub fn finish(self) -> Option<Link> {
        let mut link = self.link?;
        if self.has_display && !self.text.is_empty() {
            link.display = Some(self.text);
        }
        Some(link)
    }
}

fn split(raw: &str, kind: LinkKind) -> (String, Option<Fragment>) {
    let decode = |s: &str| match kind {
        LinkKind::Wiki => s.trim().to_owned(),
        LinkKind::Markdown => percent_decode_str(s).decode_utf8_lossy().trim().to_owned(),
    };
    let (target, fragment) = match raw.split_once('#') {
        Some((target, fragment)) => (target, Some(fragment)),
        None => (raw, None),
    };
    let fragment =
        fragment
            .map(decode)
            .filter(|f| !f.is_empty())
            .map(|f| match f.strip_prefix('^') {
                Some(block) => Fragment::Block(block.to_owned()),
                None => Fragment::Heading(f),
            });
    (decode(target), fragment)
}
