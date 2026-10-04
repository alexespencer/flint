use std::path::{Path, PathBuf};

use flint_core::StorePath;
use flint_md_parse::{Fragment, Heading, Link, LinkKind};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use zeroize::Zeroizing;

use crate::names::{file_keys, is_note, leaf_key};
use crate::schema::{SCHEMA, SCHEMA_VERSION};
use crate::{Error, Result};

/// The local index of one store: a SQLCipher database holding what was extracted from the notes.
///
/// It is derived data. It is encrypted with the store's index key, never synced, and safe to delete:
/// [`Index::open`] throws away a file it cannot read (wrong key, damaged, older schema) and starts
/// empty, and the caller then fills it from the notes ([`Index::entries`] says what it already holds).
pub struct Index {
    pub(crate) conn: Connection,
}

/// What the index knows about a file, so the caller can tell what needs re-indexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub path: StorePath,
    /// The marker given when the file was indexed.
    pub stamp: String,
    pub is_note: bool,
}

/// What was extracted from one note, minus its links (see [`Index::outgoing`]).
#[derive(Debug, Clone, PartialEq)]
pub struct NoteInfo {
    pub frontmatter: Option<serde_json::Value>,
    pub frontmatter_error: Option<String>,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub headings: Vec<Heading>,
}

impl Index {
    /// Opens (or creates) the index at `path` with `key` (the store's index key).
    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self> {
        if let Some(conn) = try_open(Some(path), key) {
            return Ok(Self { conn });
        }
        // Unreadable with this key, or an old schema: it is only a cache, so start over.
        for file in sqlite_files(path) {
            match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        let conn = try_open(Some(path), key)
            .ok_or_else(|| Error::Corrupt("a fresh index file could not be opened".into()))?;
        Ok(Self { conn })
    }

    pub fn open_in_memory(key: &[u8; 32]) -> Result<Self> {
        let conn = try_open(None, key)
            .ok_or_else(|| Error::Corrupt("an in-memory index could not be created".into()))?;
        Ok(Self { conn })
    }

    /// Every indexed file, sorted by path.
    pub fn entries(&self) -> Result<Vec<IndexEntry>> {
        let mut statement = self
            .conn
            .prepare("SELECT path, stamp, is_note FROM files ORDER BY path")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (path, stamp, is_note) = row?;
            Ok(IndexEntry {
                path: StorePath::new(path)?,
                stamp,
                is_note,
            })
        })
        .collect()
    }

    /// Indexes (or re-indexes) a note from its text. Replaces everything previously stored for it.
    pub fn upsert_note(&mut self, path: &StorePath, source: &str, stamp: &str) -> Result<()> {
        if !is_note(path) {
            return Err(Error::WrongKind(path.clone()));
        }
        let parsed = flint_md_parse::parse(source);
        let keys = file_keys(path);
        let tx = self.conn.transaction()?;

        delete_note(&tx, path.as_str())?;
        put_file(&tx, path, stamp)?;
        tx.execute(
            "INSERT INTO notes (path, frontmatter, frontmatter_error) VALUES (?1, ?2, ?3)",
            params![
                path.as_str(),
                parsed.frontmatter.as_ref().map(|v| v.to_string()),
                parsed.frontmatter_error
            ],
        )?;
        let id = tx.last_insert_rowid();

        for (ord, heading) in parsed.headings.iter().enumerate() {
            tx.execute(
                "INSERT INTO headings (note_id, ord, level, text) VALUES (?1, ?2, ?3, ?4)",
                params![id, ord as i64, heading.level, heading.text],
            )?;
        }
        for tag in &parsed.tags {
            // two tags that differ only in case are one tag
            tx.execute(
                "INSERT OR IGNORE INTO tags (note_id, tag, tag_key) VALUES (?1, ?2, ?3)",
                params![id, tag, tag.to_lowercase()],
            )?;
        }
        for (ord, alias) in parsed.aliases.iter().enumerate() {
            tx.execute(
                "INSERT INTO aliases (note_id, ord, alias, alias_key) VALUES (?1, ?2, ?3, ?4)",
                params![id, ord as i64, alias, alias.to_lowercase()],
            )?;
        }
        for (ord, link) in parsed.links.iter().enumerate() {
            let (fragment_kind, fragment) = match &link.fragment {
                None => (None, None),
                Some(Fragment::Heading(text)) => (Some(0), Some(text.as_str())),
                Some(Fragment::Block(id)) => (Some(1), Some(id.as_str())),
            };
            tx.execute(
                "INSERT INTO links (note_id, ord, kind, embed, target, fragment_kind, fragment, display, range_start, range_end, leaf_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    ord as i64,
                    matches!(link.kind, LinkKind::Markdown),
                    link.embed,
                    link.target,
                    fragment_kind,
                    fragment,
                    link.display,
                    link.range.start as i64,
                    link.range.end as i64,
                    leaf_key(&link.target)
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO note_fts (rowid, name, headings, tags, body) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                keys.path_key,
                parsed
                    .headings
                    .iter()
                    .map(|h| h.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                parsed.tags.join(" "),
                source
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Records a non-note file (an image, a PDF) so links to it resolve. Not searchable.
    pub fn upsert_attachment(&mut self, path: &StorePath, stamp: &str) -> Result<()> {
        if is_note(path) {
            return Err(Error::WrongKind(path.clone()));
        }
        let tx = self.conn.transaction()?;
        put_file(&tx, path, stamp)?;
        tx.commit()?;
        Ok(())
    }

    /// Forgets a file. Links that pointed at it become unresolved the next time they are looked up;
    /// the notes containing them need no re-indexing.
    pub fn remove(&mut self, path: &StorePath) -> Result<()> {
        let tx = self.conn.transaction()?;
        delete_note(&tx, path.as_str())?;
        tx.execute("DELETE FROM files WHERE path = ?1", [path.as_str()])?;
        tx.commit()?;
        Ok(())
    }

    /// What was extracted from a note, or `None` if it is not an indexed note.
    pub fn note(&self, path: &StorePath) -> Result<Option<NoteInfo>> {
        let Some((id, frontmatter, frontmatter_error)) = self
            .conn
            .query_row(
                "SELECT id, frontmatter, frontmatter_error FROM notes WHERE path = ?1",
                [path.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let frontmatter = frontmatter
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|e| Error::Corrupt(format!("frontmatter of {path}: {e}")))
            })
            .transpose()?;
        let strings = |sql: &str| -> Result<Vec<String>> {
            let mut statement = self.conn.prepare(sql)?;
            let rows = statement.query_map([id], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let mut statement = self
            .conn
            .prepare("SELECT level, text FROM headings WHERE note_id = ?1 ORDER BY ord")?;
        let headings = statement
            .query_map([id], |row| {
                Ok(Heading {
                    level: row.get(0)?,
                    text: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(NoteInfo {
            frontmatter,
            frontmatter_error,
            aliases: strings("SELECT alias FROM aliases WHERE note_id = ?1 ORDER BY ord")?,
            tags: strings("SELECT tag FROM tags WHERE note_id = ?1 ORDER BY tag_key")?,
            headings,
        }))
    }

    /// Notes carrying `tag` (case-insensitive, without the `#`), sorted by path.
    pub fn notes_with_tag(&self, tag: &str) -> Result<Vec<StorePath>> {
        let mut statement = self
            .conn
            .prepare("SELECT n.path FROM tags t JOIN notes n ON n.id = t.note_id WHERE t.tag_key = ?1 ORDER BY n.path")?;
        let rows = statement.query_map([tag.trim_start_matches('#').to_lowercase()], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| Ok(StorePath::new(row?)?)).collect()
    }

    /// Every tag in the store with how many notes use it, sorted by name.
    pub fn tags(&self) -> Result<Vec<(String, usize)>> {
        let mut statement = self
            .conn
            .prepare("SELECT min(tag), count(*) FROM tags GROUP BY tag_key ORDER BY tag_key")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, to_usize(row.get::<_, i64>(1)?)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// Opens the file and checks the key and schema. `None` means "unusable": the caller starts fresh.
fn try_open(path: Option<&Path>, key: &[u8; 32]) -> Option<Connection> {
    let conn = match path {
        Some(path) => Connection::open(path),
        None => Connection::open_in_memory(),
    }
    .ok()?;

    // The raw-key form skips SQLCipher's password KDF: the index key is already 256 random bits.
    let key_literal = Zeroizing::new(format!("x'{}'", hex::encode(key)));
    conn.pragma_update(None, "key", key_literal.as_str()).ok()?;

    // The first real read fails if the key is wrong or the file is not a database.
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .ok()?;
    let has_tables: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .ok()?;
    match (version, has_tables) {
        (SCHEMA_VERSION, _) => {}
        (0, 0) => {
            conn.execute_batch(SCHEMA).ok()?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)
                .ok()?;
        }
        _ => return None,
    }
    conn.pragma_update(None, "foreign_keys", true).ok()?;
    if path.is_some() {
        conn.pragma_update(None, "journal_mode", "WAL").ok()?;
        conn.pragma_update(None, "synchronous", "NORMAL").ok()?;
    }
    Some(conn)
}

/// The database and its write-ahead-log companions.
fn sqlite_files(path: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm", "-journal"]
        .iter()
        .map(|suffix| {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        })
        .collect()
}

fn put_file(tx: &Transaction<'_>, path: &StorePath, stamp: &str) -> Result<()> {
    let keys = file_keys(path);
    tx.execute(
        "INSERT INTO files (path, stamp, is_note, path_key, name_key, depth) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(path) DO UPDATE SET stamp = ?2, is_note = ?3, path_key = ?4, name_key = ?5, depth = ?6",
        params![path.as_str(), stamp, keys.is_note, keys.path_key, keys.name_key, keys.depth],
    )?;
    Ok(())
}

/// Deletes a note's rows (the full-text row by hand; the rest cascade from `notes`).
fn delete_note(tx: &Transaction<'_>, path: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM note_fts WHERE rowid IN (SELECT id FROM notes WHERE path = ?1)",
        [path],
    )?;
    tx.execute("DELETE FROM notes WHERE path = ?1", [path])?;
    Ok(())
}

/// The columns [`link_from_row`] reads, in order.
pub(crate) const LINK_COLUMNS: &str =
    "l.kind, l.embed, l.target, l.fragment_kind, l.fragment, l.display, l.range_start, l.range_end";

/// Rebuilds a [`Link`] from [`LINK_COLUMNS`] starting at column `offset`.
pub(crate) fn link_from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Link> {
    let fragment_kind: Option<i64> = row.get(offset + 3)?;
    let fragment: Option<String> = row.get(offset + 4)?;
    Ok(Link {
        kind: if row.get::<_, bool>(offset)? {
            LinkKind::Markdown
        } else {
            LinkKind::Wiki
        },
        embed: row.get(offset + 1)?,
        target: row.get(offset + 2)?,
        fragment: match (fragment_kind, fragment) {
            (Some(0), Some(text)) => Some(Fragment::Heading(text)),
            (Some(1), Some(id)) => Some(Fragment::Block(id)),
            _ => None,
        },
        display: row.get(offset + 5)?,
        range: to_usize(row.get(offset + 6)?)?..to_usize(row.get(offset + 7)?)?,
    })
}

fn to_usize(value: i64) -> rusqlite::Result<usize> {
    usize::try_from(value).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Integer, Box::new(e))
    })
}
