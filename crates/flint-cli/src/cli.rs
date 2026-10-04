use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Encrypted notes, synced through your own S3 bucket.
#[derive(Debug, Parser)]
#[command(name = "flint", version)]
pub struct Cli {
    /// Which store to act on, by name. Defaults to the active one (see `flint switch`).
    #[arg(long, global = true)]
    pub store: Option<String>,

    /// Read secrets (S3 secret key, passwords) from standard input, one per line in the order they
    /// would be asked, instead of prompting. For scripts.
    #[arg(long, global = true)]
    pub password_stdin: bool,

    /// Where the list of stores is kept. Defaults to the platform's config folder.
    #[arg(long, global = true, env = "FLINT_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Make a brand-new store: asks for the S3 secret key and a new password, then does a first sync.
    CreateStore {
        name: String,
        #[command(flatten)]
        remote: RemoteArgs,
    },
    /// Add a store that already exists on S3 (made on another device): asks for the S3 secret key and
    /// the store's password, then does a first sync.
    Join {
        name: String,
        #[command(flatten)]
        remote: RemoteArgs,
    },
    /// List the stores on this device.
    Stores,
    /// Make a store the active one.
    Switch { name: String },
    /// Open a store with its password, so it can be used until it is locked.
    Unlock,
    /// Forget the store's keys on this device.
    Lock,
    /// Change the store's password (it is asked for, not given here). Other devices keep working.
    Passwd,
    /// Encrypt files dropped into the store folder, without syncing.
    Adopt {
        /// Do not wait for files to have been untouched for a moment.
        #[arg(long)]
        now: bool,
    },
    /// Encrypt anything new in the folder, then sync with S3.
    Sync,
    /// Show the store's state: locked or not, files waiting to be encrypted, conflicts.
    Status,
    /// Tell sync you have dealt with a conflicted note (merged it, or chosen a side).
    Resolve { path: String },
    /// Forget a store on this device. Its folder and its objects on S3 are left alone.
    Forget { name: String },
}

#[derive(Debug, Args)]
pub struct RemoteArgs {
    #[arg(long)]
    pub bucket: String,
    /// The folder within the bucket. One store is one bucket + prefix. Defaults to the store's name.
    #[arg(long)]
    pub prefix: Option<String>,
    #[arg(long, default_value = "auto")]
    pub region: String,
    /// The S3 endpoint, for services other than AWS (Cloudflare R2, MinIO, ...). Must be https.
    #[arg(long)]
    pub endpoint: Option<String>,
    #[arg(long)]
    pub access_key_id: String,
    /// The store's folder on this device. Defaults to ~/Flint/<name>.
    #[arg(long)]
    pub folder: Option<PathBuf>,
}
