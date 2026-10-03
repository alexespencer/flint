use std::fmt;
use std::str::FromStr;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};

use crate::{Error, Result};

/// A file or folder inside a store: relative, UTF-8, `/`-separated, and free of `.` and `..`.
///
/// The same string is the file's S3 key (under the store's prefix), so equal paths always have
/// equal spellings. Names are opaque: there is no Unicode or case normalisation, so two names that
/// differ only in normalisation or case are different paths (APFS may treat them as the same file).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StorePath(Utf8PathBuf);

impl StorePath {
    /// Accepts any relative path made of ordinary names. Doubled slashes, a trailing slash and
    /// interior `.` are dropped, so `a//b/./c/` becomes `a/b/c`.
    pub fn new(path: impl AsRef<str>) -> Result<Self> {
        let path = path.as_ref();
        let invalid = |reason| Error::InvalidStorePath {
            path: path.to_owned(),
            reason,
        };

        let mut canonical = Utf8PathBuf::new();
        for component in Utf8Path::new(path).components() {
            match component {
                Utf8Component::Normal(name) => canonical.push(name),
                Utf8Component::RootDir | Utf8Component::Prefix(_) => {
                    return Err(invalid("must be relative"));
                }
                Utf8Component::ParentDir => return Err(invalid("must not contain `..`")),
                Utf8Component::CurDir => return Err(invalid("must not start with `.`")),
            }
        }
        if canonical.as_str().is_empty() {
            return Err(invalid("must not be empty"));
        }
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The underlying path, for `file_name()`, `extension()`, `parent()` and the like.
    pub fn as_path(&self) -> &Utf8Path {
        &self.0
    }
}

impl fmt::Display for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for StorePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for StorePath {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn accepts_ordinary_relative_paths() {
        for (input, canonical) in [
            ("note.md", "note.md"),
            ("folder/note.md", "folder/note.md"),
            ("a/b/c/d.md", "a/b/c/d.md"),
            ("Ünïcode/日本語 note.md", "Ünïcode/日本語 note.md"),
            (".hidden/file", ".hidden/file"),
            ("a..b/c", "a..b/c"),
            ("a//b", "a/b"),
            ("a/./b", "a/b"),
            ("a/b/", "a/b"),
        ] {
            let path = StorePath::new(input).unwrap_or_else(|e| panic!("{input:?}: {e}"));
            assert_eq!(path.as_str(), canonical, "input {input:?}");
        }
    }

    #[test]
    fn rejects_paths_that_could_escape_or_are_not_relative() {
        for input in [
            "",
            "/",
            "/abs/note.md",
            "..",
            "../note.md",
            "a/../b",
            "a/..",
            "./note.md",
            ".",
        ] {
            assert!(
                matches!(StorePath::new(input), Err(Error::InvalidStorePath { .. })),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn error_names_the_input_and_the_reason() {
        let err = StorePath::new("a/../b").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"invalid store path "a/../b": must not contain `..`"#
        );
    }

    #[test]
    fn names_are_opaque_so_case_and_normalisation_differ() {
        assert_ne!(
            StorePath::new("Note.md").unwrap(),
            StorePath::new("note.md").unwrap()
        );
        // "é" as one code point versus "e" + combining accent
        assert_ne!(
            StorePath::new("caf\u{e9}.md").unwrap(),
            StorePath::new("cafe\u{301}.md").unwrap()
        );
    }

    #[test]
    fn exposes_path_parts_through_camino() {
        let path: StorePath = "projects/flint/plan.md".parse().unwrap();
        assert_eq!(path.as_path().file_name(), Some("plan.md"));
        assert_eq!(path.as_path().extension(), Some("md"));
        assert_eq!(
            path.as_path().parent().map(|p| p.as_str()),
            Some("projects/flint")
        );
        assert_eq!(path.to_string(), "projects/flint/plan.md");
    }

    proptest! {
        // Whatever the input, an accepted path is relative, has no `.`/`..` components, and is a
        // fixed point of `new` (so equal paths always have equal spellings).
        #[test]
        fn accepted_paths_are_canonical(input in ".{0,40}") {
            if let Ok(path) = StorePath::new(&input) {
                prop_assert_eq!(StorePath::new(path.as_str()).unwrap(), path.clone());
                prop_assert!(!path.as_str().is_empty());
                prop_assert!(!path.as_str().starts_with('/'));
                prop_assert!(path.as_str().split('/').all(|c| !c.is_empty() && c != "." && c != ".."));
            }
        }

        #[test]
        fn dotdot_anywhere_is_rejected(prefix in "[a-z]{0,5}", suffix in "[a-z]{0,5}") {
            let input = format!("{prefix}/../{suffix}");
            prop_assert!(StorePath::new(input).is_err());
        }
    }
}
