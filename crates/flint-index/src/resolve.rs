//! Link resolution, done when a link is looked up and not when it is indexed, so adding, removing
//! or renaming a note never requires re-indexing the notes that link to it.
//!
//! The rule is Obsidian's, as seen in a real vault:
//!
//! 1. A name ending in `.md`, or with no extension, names a note. Matching ignores case and `.md`.
//!    Any other extension (`pic.png`) names an attachment by its exact file name.
//! 2. A name with folders (`Locations/Vallaki`) must match the end of the path.
//! 3. If several files match: an exact path from the store root first, then notes before attachments,
//!    then the fewest folders, then the shortest path, then alphabetical order (so it is deterministic).
//! 4. If no file matches, a note whose frontmatter `aliases` include the name.
//! 5. Otherwise the link is unresolved. It is kept: the note may be created later.
//!
//! An empty target (`[[#Heading]]`) is a link within the same note.

use flint_core::StorePath;
use flint_md_parse::Link;
use rusqlite::{Connection, params};

use crate::index::{Index, LINK_COLUMNS, link_from_row};
use crate::names::{leaf_key, strip_md};
use crate::{Error, Result};

/// A link and where it points now.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLink {
    pub link: Link,
    /// `None` if nothing in the store matches (yet).
    pub resolved: Option<StorePath>,
}

/// A link found in `from` that resolves to the file asked about.
#[derive(Debug, Clone, PartialEq)]
pub struct Backlink {
    pub from: StorePath,
    pub link: Link,
}

impl Index {
    /// Where a link written in `from` with this target points, or `None` if it is unresolved.
    pub fn resolve(&self, from: &StorePath, target: &str) -> Result<Option<StorePath>> {
        resolve(&self.conn, from, target)
    }

    /// Every link in a note, in document order, each with where it points now.
    pub fn outgoing(&self, from: &StorePath) -> Result<Vec<ResolvedLink>> {
        let sql = format!(
            "SELECT {LINK_COLUMNS} FROM links l JOIN notes n ON n.id = l.note_id WHERE n.path = ?1 ORDER BY l.ord"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let links = statement
            .query_map([from.as_str()], |row| link_from_row(row, 0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        links
            .into_iter()
            .map(|link| {
                Ok(ResolvedLink {
                    resolved: resolve(&self.conn, from, &link.target)?,
                    link,
                })
            })
            .collect()
    }

    /// Links in other notes that resolve to `target` (a note or an attachment), ordered by note then
    /// position. A note's links to itself are not included; embeds are, marked by [`Link::embed`].
    pub fn backlinks(&self, target: &StorePath) -> Result<Vec<Backlink>> {
        let Some(name_key) = self
            .conn
            .query_row(
                "SELECT name_key FROM files WHERE path = ?1",
                [target.as_str()],
                |row| row.get::<_, String>(0),
            )
            .ok()
        else {
            return Ok(Vec::new());
        };
        // Cheap candidate filter on the link's last name segment; resolution then decides.
        let mut leaves = vec![name_key];
        let mut statement = self.conn.prepare(
            "SELECT a.alias_key FROM aliases a JOIN notes n ON n.id = a.note_id WHERE n.path = ?1",
        )?;
        for alias in statement.query_map([target.as_str()], |row| row.get::<_, String>(0))? {
            leaves.push(leaf_key(&alias?));
        }
        leaves.sort();
        leaves.dedup();

        let sql = format!(
            "SELECT n.path, {LINK_COLUMNS} FROM links l JOIN notes n ON n.id = l.note_id
             WHERE l.leaf_key = ?1 AND l.target <> '' AND n.path <> ?2 ORDER BY n.path, l.ord"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let mut found = Vec::new();
        for leaf in &leaves {
            let rows = statement.query_map(params![leaf, target.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, link_from_row(row, 1)?))
            })?;
            for row in rows {
                let (from, link) = row?;
                let from = StorePath::new(from)?;
                if resolve(&self.conn, &from, &link.target)?.as_ref() == Some(target) {
                    found.push(Backlink { from, link });
                }
            }
        }
        found.sort_by(|a, b| (&a.from, a.link.range.start).cmp(&(&b.from, b.link.range.start)));
        found.dedup_by(|a, b| a.from == b.from && a.link.range == b.link.range);
        Ok(found)
    }

    /// Links that currently point at nothing, across the whole store, for an "unresolved links" view.
    pub fn unresolved(&self) -> Result<Vec<Backlink>> {
        let sql = format!(
            "SELECT n.path, {LINK_COLUMNS} FROM links l JOIN notes n ON n.id = l.note_id WHERE l.target <> '' ORDER BY n.path, l.ord"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, link_from_row(row, 1)?))
        })?;
        let mut missing = Vec::new();
        for row in rows {
            let (from, link) = row?;
            let from = StorePath::new(from)?;
            if resolve(&self.conn, &from, &link.target)?.is_none() {
                missing.push(Backlink { from, link });
            }
        }
        Ok(missing)
    }
}

/// A file that matched a name, with what the tie-break needs.
struct Candidate {
    path: String,
    is_note: bool,
    exact: bool,
    depth: i64,
}

fn resolve(conn: &Connection, from: &StorePath, target: &str) -> Result<Option<StorePath>> {
    let target = target.trim();
    if target.is_empty() {
        return Ok(Some(from.clone()));
    }
    let lower = target.to_lowercase();
    let wants_note_extension = lower.ends_with(".md");
    // "/Locations/Vallaki" means from the store root; the leading slash is only a hint.
    let key = strip_md(&lower).trim_start_matches('/');
    let leaf = leaf_key(key);

    let mut statement =
        conn.prepare("SELECT path, is_note, path_key, depth FROM files WHERE name_key = ?1")?;
    let mut candidates = Vec::new();
    for row in statement.query_map([&leaf], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, bool>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })? {
        let (path, is_note, path_key, depth) = row?;
        if wants_note_extension && !is_note {
            continue;
        }
        let exact = path_key == key;
        if key.contains('/') && !exact && !path_key.ends_with(&format!("/{key}")) {
            continue;
        }
        candidates.push(Candidate {
            path,
            is_note,
            exact,
            depth,
        });
    }
    if let Some(best) = best(candidates) {
        return Ok(Some(StorePath::new(best)?));
    }

    // No file by that name: a note that lists it as an alias.
    let mut statement = conn.prepare(
        "SELECT f.path, f.depth FROM aliases a JOIN notes n ON n.id = a.note_id JOIN files f ON f.path = n.path WHERE a.alias_key = ?1",
    )?;
    let by_alias = statement
        .query_map([&lower], |row| {
            Ok(Candidate {
                path: row.get(0)?,
                is_note: true,
                exact: false,
                depth: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    best(by_alias)
        .map(|path| StorePath::new(path).map_err(Error::from))
        .transpose()
}

fn best(candidates: Vec<Candidate>) -> Option<String> {
    candidates
        .into_iter()
        .min_by(|a, b| {
            (!a.exact, !a.is_note, a.depth, a.path.len(), &a.path).cmp(&(
                !b.exact,
                !b.is_note,
                b.depth,
                b.path.len(),
                &b.path,
            ))
        })
        .map(|c| c.path)
}
