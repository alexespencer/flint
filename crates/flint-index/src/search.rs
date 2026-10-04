use flint_core::StorePath;
use rusqlite::params;

use crate::Result;
use crate::index::Index;

/// Wraps matched words in [`SearchHit::snippet`]. Control characters, so they cannot be confused
/// with anything in a note.
pub const HIGHLIGHT_START: char = '\u{1}';
pub const HIGHLIGHT_END: char = '\u{2}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: StorePath,
    /// A short excerpt with the matches between [`HIGHLIGHT_START`] and [`HIGHLIGHT_END`].
    pub snippet: String,
}

impl Index {
    /// Full-text search over note names, headings, tags and text (frontmatter included), best match
    /// first. Words are ANDed; the last word also matches as a prefix, so results appear as you type.
    /// Case and accents are ignored. The query is plain words: operators in it are not interpreted.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(fts_query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        // Columns: name, headings, tags, body. A match in the name counts most.
        let mut statement = self.conn.prepare(
            "SELECT n.path, snippet(note_fts, -1, ?2, ?3, '…', 12)
             FROM note_fts JOIN notes n ON n.id = note_fts.rowid
             WHERE note_fts MATCH ?1
             ORDER BY bm25(note_fts, 8.0, 4.0, 2.0, 1.0), n.path
             LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![
                fts_query,
                HIGHLIGHT_START.to_string(),
                HIGHLIGHT_END.to_string(),
                limit as i64
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        rows.map(|row| {
            let (path, snippet) = row?;
            Ok(SearchHit {
                path: StorePath::new(path)?,
                snippet,
            })
        })
        .collect()
    }
}

/// `café au  lait` becomes `"café" "au" "lait"*`. Every word is quoted, so FTS operators and
/// punctuation in what the user typed are just text.
fn fts_query(query: &str) -> Option<String> {
    // A NUL inside the query makes FTS5 fail with "unterminated string"; treat it as a separator.
    let query = query.replace('\0', " ");
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect();
    let (last, rest) = words.split_last()?;
    let mut parts = rest.to_vec();
    parts.push(format!("{last}*"));
    Some(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_safe_query() {
        assert_eq!(
            fts_query("café au  lait").as_deref(),
            Some("\"café\" \"au\" \"lait\"*")
        );
        assert_eq!(fts_query("single").as_deref(), Some("\"single\"*"));
        assert_eq!(
            fts_query(r#"say "hi" AND NOT (x)"#).as_deref(),
            Some(r#""say" """hi""" "AND" "NOT" "(x)"*"#)
        );
        assert_eq!(fts_query(""), None);
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query("a\0b").as_deref(), Some("\"a\" \"b\"*"));
        assert_eq!(fts_query("\0"), None);
    }
}
