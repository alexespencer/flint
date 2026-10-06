use std::path::{Path, PathBuf};

use flint_core::{ContentHash, StorePath};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Baseline, Conflict, Error, LocalStamp, Result, SeenVersion};

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE baselines (
    path     TEXT PRIMARY KEY,
    hash     TEXT NOT NULL,        -- hex SHA-256 of the stored bytes at the last agreed sync
    etag     TEXT,                 -- the remote's ETag then (opaque)
    size     INTEGER,              -- the local file's size and mtime when hashed: a skip-rehash hint only
    mtime_ns INTEGER
);
CREATE TABLE conflicts (
    path      TEXT PRIMARY KEY,    -- a conflict can exist with no baseline (same new name on both devices)
    copy_path TEXT,           -- NULL when there is no other version to keep
    seen_hash TEXT,           -- the remote version that is in the copy (what the user saw) ...
    seen_etag TEXT            -- ... which becomes the baseline when the conflict is resolved
);
";

/// The sync database of one store.
pub struct SyncState {
    conn: Connection,
}

impl SyncState {
    /// Opens (or creates) `.store/sync.db`. A file that is damaged, or from a newer version, is an error
    /// and is left exactly as it is; see [`reset`](Self::reset).
    pub fn open(path: &Path) -> Result<Self> {
        let state = Self::init(Connection::open(path)?)?;
        // Only after the schema check: switching journal mode rewrites the file header, which must not
        // happen to a database that turns out not to be ours.
        state.conn.pragma_update(None, "journal_mode", "WAL")?;
        // FULL: this is the record of what was synced, and the file is small and rarely written.
        state.conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(state)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let tables: i64 =
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
        match (version, tables) {
            (SCHEMA_VERSION, _) => {}
            (0, 0) => {
                conn.execute_batch(SCHEMA)?;
                conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            (found, _) => {
                return Err(Error::UnsupportedVersion {
                    found,
                    expected: SCHEMA_VERSION,
                });
            }
        }
        Ok(Self { conn })
    }

    /// Deletes the database file (and its WAL companions) so the next [`open`](Self::open) starts empty.
    /// Every file that then differs between the two sides will be treated as a conflict, so this is a last
    /// resort, never automatic.
    pub fn reset(path: &Path) -> Result<()> {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            match std::fs::remove_file(PathBuf::from(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }

    // ---- baselines ----------------------------------------------------------------------------

    pub fn baseline(&self, path: &StorePath) -> Result<Option<Baseline>> {
        self.conn
            .query_row(
                "SELECT path, hash, etag, size, mtime_ns FROM baselines WHERE path = ?1",
                [path.as_str()],
                baseline_from_row,
            )
            .optional()?
            .transpose()
    }

    /// Every baseline, sorted by path.
    pub fn baselines(&self) -> Result<Vec<Baseline>> {
        let mut statement = self
            .conn
            .prepare("SELECT path, hash, etag, size, mtime_ns FROM baselines ORDER BY path")?;
        let rows = statement.query_map([], baseline_from_row)?;
        rows.map(|row| row?).collect()
    }

    /// Records that local and remote now agree on this file, replacing any earlier baseline. Call it
    /// only after the upload, download or comparison that established the agreement has succeeded.
    pub fn record(&mut self, baseline: &Baseline) -> Result<()> {
        put_baseline(&self.conn, baseline)
    }

    /// [`record`](Self::record) for many files in one transaction, all or none. A first sync of thousands
    /// of files should not pay for a durable commit per file.
    pub fn record_many(&mut self, baselines: &[Baseline]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for baseline in baselines {
            put_baseline(&tx, baseline)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Refreshes only the skip-rehash hint of an existing baseline. Returns whether there was one.
    pub fn touch(&mut self, path: &StorePath, stamp: &LocalStamp) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE baselines SET size = ?2, mtime_ns = ?3 WHERE path = ?1",
            params![path.as_str(), stamp.size as i64, stamp.mtime_ns],
        )?;
        Ok(changed > 0)
    }

    /// Forgets a file that is gone from both sides.
    pub fn forget(&mut self, path: &StorePath) -> Result<()> {
        self.conn
            .execute("DELETE FROM baselines WHERE path = ?1", [path.as_str()])?;
        Ok(())
    }

    /// Forgets everything this device knows about what was synced: every baseline and every conflict flag.
    /// Used to put a device's copy back onto S3 after the store was deleted there: every local file is then
    /// new to the next sync, which uploads it. Conflict copies are ordinary files and are uploaded as well.
    pub fn clear_all(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM baselines", [])?;
        tx.execute("DELETE FROM conflicts", [])?;
        tx.commit()?;
        Ok(())
    }

    // ---- conflicts ----------------------------------------------------------------------------

    /// Holds `path` back from sync until [`resolve`](Self::resolve) or [`clear_conflict`](Self::clear_conflict).
    /// `copy` is where the other side's version was written. Marking an already-flagged path updates the copy.
    pub fn mark_conflict(
        &mut self,
        path: &StorePath,
        copy: Option<&StorePath>,
        seen: Option<&SeenVersion>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO conflicts (path, copy_path, seen_hash, seen_etag) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET copy_path = ?2, seen_hash = ?3, seen_etag = ?4",
            params![
                path.as_str(),
                copy.map(|c| c.as_str()),
                seen.map(|v| v.hash.to_string()),
                seen.and_then(|v| v.etag.clone())
            ],
        )?;
        Ok(())
    }

    pub fn conflict(&self, path: &StorePath) -> Result<Option<Conflict>> {
        self.conn
            .query_row(
                "SELECT path, copy_path, seen_hash, seen_etag FROM conflicts WHERE path = ?1",
                [path.as_str()],
                conflict_from_row,
            )
            .optional()?
            .transpose()
    }

    /// Every flagged path, sorted.
    pub fn conflicts(&self) -> Result<Vec<Conflict>> {
        let mut statement = self
            .conn
            .prepare("SELECT path, copy_path, seen_hash, seen_etag FROM conflicts ORDER BY path")?;
        let rows = statement.query_map([], conflict_from_row)?;
        rows.map(|row| row?).collect()
    }

    /// The user is done with a conflict: records the new baseline and lifts the flag in one transaction,
    /// so there is never a moment with the flag gone but the old baseline still in place.
    pub fn resolve(&mut self, baseline: &Baseline) -> Result<()> {
        let tx = self.conn.transaction()?;
        put_baseline(&tx, baseline)?;
        tx.execute(
            "DELETE FROM conflicts WHERE path = ?1",
            [baseline.path.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Settles a conflict by dropping the baseline too, so the file is treated as new on whichever side still
    /// has it. One transaction.
    pub fn release(&mut self, path: &StorePath) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM baselines WHERE path = ?1", [path.as_str()])?;
        tx.execute("DELETE FROM conflicts WHERE path = ?1", [path.as_str()])?;
        tx.commit()?;
        Ok(())
    }

    /// Lifts a flag without touching the baseline (for example, the file no longer exists on either side).
    pub fn clear_conflict(&mut self, path: &StorePath) -> Result<()> {
        self.conn
            .execute("DELETE FROM conflicts WHERE path = ?1", [path.as_str()])?;
        Ok(())
    }
}

fn put_baseline(conn: &Connection, baseline: &Baseline) -> Result<()> {
    conn.execute(
        "INSERT INTO baselines (path, hash, etag, size, mtime_ns) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET hash = ?2, etag = ?3, size = ?4, mtime_ns = ?5",
        params![
            baseline.path.as_str(),
            baseline.hash.to_string(),
            baseline.etag,
            baseline.stamp.map(|s| s.size as i64),
            baseline.stamp.map(|s| s.mtime_ns)
        ],
    )?;
    Ok(())
}

fn baseline_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Baseline>> {
    let (path, hash, etag, size, mtime_ns): (
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
    ) = (
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    );
    Ok((|| {
        let corrupt = |what: &str| Error::Corrupt(format!("baseline for {path:?}: {what}"));
        let stamp = match (size, mtime_ns) {
            (Some(size), Some(mtime_ns)) => Some(LocalStamp {
                size: u64::try_from(size).map_err(|_| corrupt("negative size"))?,
                mtime_ns,
            }),
            _ => None,
        };
        Ok(Baseline {
            path: StorePath::new(&path).map_err(|e| corrupt(&e.to_string()))?,
            hash: hash
                .parse::<ContentHash>()
                .map_err(|e| corrupt(&e.to_string()))?,
            etag,
            stamp,
        })
    })())
}

fn conflict_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Conflict>> {
    let (path, copy, seen_hash, seen_etag): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
    Ok((|| {
        let corrupt = |what: String| Error::Corrupt(format!("conflict for {path:?}: {what}"));
        Ok(Conflict {
            path: StorePath::new(&path).map_err(|e| corrupt(e.to_string()))?,
            copy: copy
                .map(|c| StorePath::new(&c).map_err(|e| corrupt(e.to_string())))
                .transpose()?,
            seen: seen_hash
                .map(|h| {
                    h.parse::<ContentHash>()
                        .map(|hash| SeenVersion {
                            hash,
                            etag: seen_etag,
                        })
                        .map_err(|e| corrupt(e.to_string()))
                })
                .transpose()?,
        })
    })())
}

#[cfg(test)]
mod tests;
