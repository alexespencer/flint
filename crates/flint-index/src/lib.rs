//! The local search and link index for one store.
//!
//! A SQLCipher database (encrypted with the store's index key, never synced, safe to delete) holding
//! what was extracted from the notes: files, links, headings, tags, aliases, frontmatter (as JSON) and
//! a full-text index. It answers backlinks, link resolution and search.
//!
//! Feed it with [`Index::upsert_note`], [`Index::upsert_attachment`] and [`Index::remove`]; compare
//! [`Index::entries`] with the folder to see what is out of date. Links are resolved when asked
//! (see the rules in [`resolve`]), so renaming or adding a note never means re-indexing others.

mod error;
mod index;
mod names;
pub mod resolve;
mod schema;
mod search;

pub use error::{Error, Result};
pub use index::{Index, IndexEntry, NoteInfo};
pub use resolve::{Backlink, ResolvedLink};
pub use search::{HIGHLIGHT_END, HIGHLIGHT_START, SearchHit};

#[cfg(test)]
mod tests;
