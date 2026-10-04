use std::time::{SystemTime, UNIX_EPOCH};

use flint_core::StorePath;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::DeviceName;

/// Where the other side's version of `path` is written when the two sides disagree, next to the
/// original and Dropbox/Obsidian style: `dir/Note.md` becomes `dir/Note.conflict-<device>-<yyyymmdd-hhmm>.md`
/// (UTC), and a file without an extension just gets the suffix. The caller adds a counter if that name
/// is already taken.
pub fn conflict_copy_path(path: &StorePath, device: &DeviceName, now: SystemTime) -> StorePath {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let stamp = OffsetDateTime::from_unix_timestamp(seconds)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(format_description!("[year][month][day]-[hour][minute]"))
        .expect("a fixed format always formats");

    let p = path.as_path();
    let stem = p.file_stem().unwrap_or("");
    let suffix = format!("conflict-{device}-{stamp}");
    let name = match p.extension() {
        Some(ext) => format!("{stem}.{suffix}.{ext}"),
        None => format!("{stem}.{suffix}"),
    };
    let full = match p.parent().filter(|d| !d.as_str().is_empty()) {
        Some(dir) => format!("{dir}/{name}"),
        None => name,
    };
    StorePath::new(full).expect("a valid path with a suffix inserted is still a valid path")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn name(path: &str, secs: u64) -> String {
        conflict_copy_path(
            &StorePath::new(path).unwrap(),
            &DeviceName::new("Pixel 8"),
            at(secs),
        )
        .to_string()
    }

    #[test]
    fn names_follow_the_documented_shape() {
        // 2026-10-04 12:34:56 UTC
        let t = 1_791_117_296;
        assert_eq!(name("Note.md", t), "Note.conflict-Pixel-8-20261004-1234.md");
        assert_eq!(
            name("Characters/Baron.md", t),
            "Characters/Baron.conflict-Pixel-8-20261004-1234.md"
        );
        assert_eq!(
            name("a/b/c/deep note.md", t),
            "a/b/c/deep note.conflict-Pixel-8-20261004-1234.md"
        );
        assert_eq!(name("README", t), "README.conflict-Pixel-8-20261004-1234");
        assert_eq!(
            name("archive.tar.gz", t),
            "archive.tar.conflict-Pixel-8-20261004-1234.gz"
        );
        assert_eq!(
            name("Images/pic.PNG", t),
            "Images/pic.conflict-Pixel-8-20261004-1234.PNG"
        );
    }

    #[test]
    fn the_time_is_utc_and_minute_resolution() {
        assert_eq!(name("a.md", 0), "a.conflict-Pixel-8-19700101-0000.md");
        assert_eq!(name("a.md", 59), "a.conflict-Pixel-8-19700101-0000.md");
        assert_eq!(name("a.md", 60), "a.conflict-Pixel-8-19700101-0001.md");
        assert_eq!(name("a.md", 86_399), "a.conflict-Pixel-8-19700101-2359.md");
        assert_eq!(name("a.md", 86_400), "a.conflict-Pixel-8-19700102-0000.md");
    }

    #[test]
    fn a_clock_before_1970_does_not_panic() {
        let before = UNIX_EPOCH - Duration::from_secs(10);
        let path = conflict_copy_path(
            &StorePath::new("a.md").unwrap(),
            &DeviceName::new("d"),
            before,
        );
        assert_eq!(path.as_str(), "a.conflict-d-19700101-0000.md");
    }

    #[test]
    fn the_copy_sits_next_to_the_original_and_is_a_different_file() {
        for original in [
            "a.md",
            "dir/a.md",
            "dir/sub/a b.md",
            "noext",
            ".hidden",
            "dir/.hidden.md",
        ] {
            let original = StorePath::new(original).unwrap();
            let copy = conflict_copy_path(&original, &DeviceName::new("Mac"), at(1_791_117_296));
            assert_ne!(copy, original);
            assert_eq!(
                copy.as_path().parent(),
                original.as_path().parent(),
                "{original}"
            );
        }
    }
}
