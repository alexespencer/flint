//! The lowercase keys the index matches on. SQLite's own `lower()` only handles ASCII, so every key
//! is computed here with Rust's Unicode-aware `to_lowercase` and stored.

use flint_core::StorePath;

pub(crate) struct FileKeys {
    pub is_note: bool,
    /// Lowercase path; for notes, without the `.md`.
    pub path_key: String,
    /// The last segment of `path_key`: a note's name, or an attachment's file name with extension.
    pub name_key: String,
    /// Folders above the file; shallower files win when a name is ambiguous.
    pub depth: i64,
}

pub(crate) fn is_note(path: &StorePath) -> bool {
    path.as_path()
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

pub(crate) fn file_keys(path: &StorePath) -> FileKeys {
    let is_note = is_note(path);
    let lower = path.as_str().to_lowercase();
    let path_key = if is_note {
        strip_md(&lower).to_owned()
    } else {
        lower
    };
    let name_key = last_segment(&path_key).to_owned();
    FileKeys {
        is_note,
        path_key,
        name_key,
        depth: path.as_str().matches('/').count() as i64,
    }
}

/// What a link target is compared on: lowercase, `.md` dropped, last path segment.
pub(crate) fn leaf_key(target: &str) -> String {
    let lower = target.trim().to_lowercase();
    last_segment(strip_md(&lower)).to_owned()
}

pub(crate) fn strip_md(lowercase: &str) -> &str {
    lowercase.strip_suffix(".md").unwrap_or(lowercase)
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> StorePath {
        StorePath::new(s).unwrap()
    }

    #[test]
    fn note_and_attachment_keys() {
        let note = file_keys(&p("Characters/Édward Black.md"));
        assert!(note.is_note);
        assert_eq!(
            (note.path_key.as_str(), note.name_key.as_str(), note.depth),
            ("characters/édward black", "édward black", 1)
        );

        let image = file_keys(&p("Images/Pasted image 1.PNG"));
        assert!(!image.is_note);
        assert_eq!(
            (image.path_key.as_str(), image.name_key.as_str()),
            ("images/pasted image 1.png", "pasted image 1.png")
        );

        assert!(
            file_keys(&p("A.MD")).is_note,
            "extension match is case-insensitive"
        );
        assert!(!file_keys(&p("Transcripts/2026-07-03.txt")).is_note);
    }

    #[test]
    fn leaf_keys() {
        assert_eq!(leaf_key("Locations/Vallaki"), "vallaki");
        assert_eq!(leaf_key("  Vallaki.md "), "vallaki");
        assert_eq!(leaf_key("pic.PNG"), "pic.png");
        assert_eq!(leaf_key("ÉDWARD"), "édward");
        assert_eq!(leaf_key(""), "");
    }
}
