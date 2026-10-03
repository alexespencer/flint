//! Shared types and errors for Flint. No I/O.
//!
//! `Note` and `Link` will land here once `flint-md-parse` and `flint-index` define them.

mod error;
mod hash;
mod path;

pub use error::{Error, Result};
pub use hash::ContentHash;
pub use path::StorePath;
