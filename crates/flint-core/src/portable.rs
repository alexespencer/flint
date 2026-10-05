//! What a file name may be on the systems Flint runs on. macOS, Linux and Android allow nearly anything;
//! Windows does not, so a note named on one of those can have a name Windows cannot store.

/// Why Windows would refuse `name` (one name, not a path), or `None` if it would accept it.
pub fn windows_name_problem(name: &str) -> Option<&'static str> {
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') || c < ' ')
    {
        return Some(
            "contains a character Windows does not allow in file names (< > : \" | ? * or a control character)",
        );
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("ends with a dot or a space, which Windows does not allow");
    }
    // devices answer to these names with any extension: `nul.txt` is the NUL device
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        });
    if reserved {
        return Some("is a name Windows reserves for devices (CON, PRN, AUX, NUL, COM1-9, LPT1-9)");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorePath;

    #[test]
    fn ordinary_names_are_fine() {
        for name in [
            "note.md",
            "Meeting notes 2026.md",
            "日本語.md",
            ".hidden",
            "a.b.c",
            "console.md", // starts like CON but is not it
            "com10.md",
            "com.md",
            "auxiliary",
        ] {
            assert_eq!(windows_name_problem(name), None, "{name}");
        }
    }

    #[test]
    fn names_windows_refuses_are_caught() {
        for name in [
            "what?.md",
            "a:b.md",
            "a|b",
            "a*b",
            "<a>",
            "say \"hi\".md",
            "tab\there",
            "ends.",
            "ends ",
            "CON",
            "con.md",
            "Nul.txt",
            "AUX",
            "prn",
            "COM1",
            "lpt9.md",
            "com5.tar.gz",
        ] {
            assert!(windows_name_problem(name).is_some(), "{name}");
        }
    }

    #[test]
    fn a_path_reports_the_first_name_in_it_that_is_a_problem() {
        let fine = StorePath::new("a/b/note.md").unwrap();
        assert_eq!(fine.windows_problem(), None);
        let bad = StorePath::new("Projects/Q&A: draft/notes.md").unwrap();
        assert_eq!(
            bad.windows_problem().map(|(name, _)| name),
            Some("Q&A: draft")
        );
        let device = StorePath::new("ok/nul/x.md").unwrap();
        assert_eq!(device.windows_problem().map(|(name, _)| name), Some("nul"));
    }
}
