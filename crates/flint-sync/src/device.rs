use std::fmt;

const MAX_LEN: usize = 24;
const FALLBACK: &str = "device";

/// A device's name as it appears in conflict-copy file names. Whatever the device calls itself
/// ("Alex's MacBook Pro", "Pixel 8") is turned into something safe in a file name on every platform
/// and in an S3 key: letters and digits (including non-ASCII ones) are kept, every other run of
/// characters becomes a single `-`, and the result is trimmed and capped at 24 characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceName(String);

impl DeviceName {
    pub fn new(name: &str) -> Self {
        let mut clean = String::new();
        let mut pending_dash = false;
        for c in name.chars() {
            if c.is_alphanumeric() {
                if pending_dash && !clean.is_empty() {
                    clean.push('-');
                }
                pending_dash = false;
                clean.push(c);
            } else {
                pending_dash = true;
            }
        }
        let clean: String = clean.chars().take(MAX_LEN).collect();
        let clean = clean.trim_end_matches('-').to_owned();
        Self(if clean.is_empty() {
            FALLBACK.to_owned()
        } else {
            clean
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn typical_device_names() {
        for (input, expected) in [
            ("Alex's MacBook Pro", "Alex-s-MacBook-Pro"),
            ("Pixel 8", "Pixel-8"),
            ("galaxy-s24", "galaxy-s24"),
            ("  spaced   out  ", "spaced-out"),
            ("Zoë's iPhone", "Zoë-s-iPhone"),
            ("日本語の端末", "日本語の端末"),
            ("a/b\\c:d*e?f\"g<h>i|j", "a-b-c-d-e-f-g-h-i-j"),
            ("../../etc/passwd", "etc-passwd"),
            ("under_score.and.dots", "under-score-and-dots"),
        ] {
            assert_eq!(DeviceName::new(input).as_str(), expected, "{input:?}");
        }
    }

    #[test]
    fn nothing_usable_falls_back() {
        for input in ["", "   ", "---", "///", "!!!", "\u{0}\u{1}"] {
            assert_eq!(DeviceName::new(input).as_str(), "device", "{input:?}");
        }
    }

    #[test]
    fn long_names_are_capped_without_a_trailing_dash() {
        let name = DeviceName::new(&"abcdefghij ".repeat(10));
        assert!(name.as_str().chars().count() <= 24);
        assert!(!name.as_str().ends_with('-'));
        assert_eq!(DeviceName::new("a".repeat(100).as_str()).as_str().len(), 24);
    }

    proptest! {
        #[test]
        fn always_safe_in_a_file_name(input in "\\PC{0,60}") {
            let name = DeviceName::new(&input);
            let s = name.as_str();
            prop_assert!(!s.is_empty() && s.chars().count() <= 24);
            prop_assert!(s.chars().all(|c| c.is_alphanumeric() || c == '-'), "{s:?}");
            prop_assert!(!s.starts_with('-') && !s.ends_with('-') && !s.contains("--"), "{s:?}");
            // idempotent: cleaning a clean name changes nothing
            prop_assert_eq!(DeviceName::new(s), name.clone());
        }
    }
}
