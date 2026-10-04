use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

// A `#` that is not part of a word, followed by tag characters. The leading group stands in for a
// look-behind, which `regex` does not have: the `#` must start the text or follow something that is
// not a letter, digit, `_`, `/`, `#` or `&`.
static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^\p{L}\p{N}_/#&])#([\p{L}\p{N}_/-]+)").expect("valid regex")
});

/// Finds inline `#tags` in running text. The parser splits text at every stray bracket, so text is
/// gathered here and scanned in whole runs.
#[derive(Default)]
pub(crate) struct TagCollector {
    pending: String,
    tags: BTreeSet<String>,
}

impl TagCollector {
    pub fn push(&mut self, text: &str) {
        self.pending.push_str(text);
    }

    pub fn flush(&mut self) {
        for captures in TAG.captures_iter(&self.pending) {
            let tag = captures[1].trim_end_matches('/');
            // Obsidian: a tag needs at least one non-digit, so `#1984` is not a tag but `#1984a` is.
            if !tag.is_empty() && tag.chars().any(|c| !c.is_ascii_digit()) {
                self.tags.insert(tag.to_owned());
            }
        }
        self.pending.clear();
    }

    pub fn into_tags(self) -> BTreeSet<String> {
        self.tags
    }
}
