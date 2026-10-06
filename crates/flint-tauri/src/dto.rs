//! What crosses the boundary to the web view: plain data, camelCase, no domain types.

use std::path::PathBuf;

use flint_app_service::{ConflictInfo, NewStore, Status, SyncOutcome};
use flint_registry::{RemoteSettings, StoreRecord};
use flint_sync::{ConflictOutcome, Refusal};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreDto {
    /// The S3 account, so a new store can reuse it. The secret key is never sent.
    pub bucket: String,
    pub endpoint: Option<String>,
    pub region: String,
    pub access_key_id: String,
    pub id: String,
    pub name: String,
    pub folder: String,
    pub active: bool,
    pub unlocked: bool,
    pub files: usize,
    pub awaiting_encryption: Vec<String>,
    pub conflicts: Vec<ConflictDto>,
}

impl StoreDto {
    pub fn new(status: Status, active: bool, record: &StoreRecord) -> Self {
        Self {
            bucket: record.remote.bucket.clone(),
            endpoint: record.remote.endpoint.clone(),
            region: record.remote.region.clone(),
            access_key_id: record.remote.access_key_id.clone(),
            id: status.id.to_string(),
            name: status.name,
            folder: status.folder.display().to_string(),
            active,
            unlocked: status.unlocked,
            files: status.files,
            awaiting_encryption: status
                .awaiting_encryption
                .iter()
                .map(ToString::to_string)
                .collect(),
            conflicts: status
                .conflicts
                .into_iter()
                .map(ConflictDto::from)
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDto {
    pub path: String,
    /// Where the other version was written; absent if it was deleted on the other side.
    pub copy: Option<String>,
}

impl From<ConflictInfo> for ConflictDto {
    fn from(c: ConflictInfo) -> Self {
        Self {
            path: c.path.to_string(),
            copy: c.copy.map(|p| p.to_string()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDto {
    pub adopted: usize,
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted_remote: usize,
    pub trashed_local: usize,
    /// One line per new conflict, ready to show.
    pub conflicts: Vec<String>,
    pub refused: Vec<String>,
    pub failed: Vec<String>,
}

impl From<SyncOutcome> for SyncDto {
    fn from(outcome: SyncOutcome) -> Self {
        let report = outcome.report;
        let conflicts = report
            .conflicts
            .iter()
            .map(|c| match &c.outcome {
                ConflictOutcome::Diverged { .. } => {
                    format!("{} was changed on two devices", c.path)
                }
                ConflictOutcome::EditedHereDeletedThere => {
                    format!("{} was edited here and deleted elsewhere", c.path)
                }
                ConflictOutcome::RestoredEditedRemotely => {
                    format!(
                        "{} was deleted here but edited elsewhere, so it came back",
                        c.path
                    )
                }
            })
            .collect();
        Self {
            adopted: outcome.adopted.adopted.len(),
            uploaded: report.uploaded.len(),
            downloaded: report.downloaded.len(),
            deleted_remote: report.deleted_remote.len(),
            trashed_local: report.trashed_local.len(),
            conflicts,
            refused: report
                .refused
                .iter()
                .map(|(path, why)| format!("{path}: {}", refusal(why)))
                .collect(),
            failed: report
                .failed
                .iter()
                .map(|(path, error)| format!("{path}: {error}"))
                .collect(),
        }
    }
}

fn refusal(why: &Refusal) -> &'static str {
    match why {
        Refusal::RemoteNotCiphertext => "the object on S3 is not encrypted by Flint",
        Refusal::LocalNotCiphertext => "not encrypted yet",
        Refusal::LocalNotDecryptable => {
            "damaged or edited outside Flint, so it no longer opens; not uploaded (the last good version is still on S3)"
        }
        Refusal::RemoteNotDecryptable => "the copy on S3 does not open; not downloaded",
        Refusal::HashMismatch { .. } => "the download did not match its recorded hash",
        Refusal::LocalChangedDuringSync => "it changed while syncing; it will go next time",
        Refusal::KeyFileDiverged => "the password was changed on two devices at once",
        Refusal::Contended => "S3 kept changing under it; it will go next time",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitDto {
    pub path: String,
    /// The matching excerpt; matches are wrapped in `\u{1}` and `\u{2}`.
    pub snippet: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDto {
    pub path: String,
    pub created_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkDto {
    pub from: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewStoreArgs {
    pub name: String,
    pub folder: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: String,
    /// Empty when `reuse_secret_from` names a store whose saved secret key should be used.
    pub s3_secret: String,
    pub reuse_secret_from: Option<String>,
    pub password: String,
}

impl From<NewStoreArgs> for NewStore {
    fn from(a: NewStoreArgs) -> Self {
        NewStore {
            name: a.name,
            folder: PathBuf::from(a.folder),
            remote: RemoteSettings {
                bucket: a.bucket,
                prefix: a.prefix,
                region: a.region,
                endpoint: a.endpoint.filter(|e| !e.trim().is_empty()),
                access_key_id: a.access_key_id,
            },
            s3_secret: a.s3_secret,
            password: a.password,
        }
    }
}

/// An error as the web view sees it: a stable `kind` to branch on, and a message to show.
#[derive(Serialize)]
pub struct CmdError {
    pub kind: &'static str,
    pub message: String,
}

impl CmdError {
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            kind: "other",
            message: message.into(),
        }
    }
}

impl From<flint_app_service::Error> for CmdError {
    fn from(error: flint_app_service::Error) -> Self {
        use flint_app_service::Error as E;
        let kind = match &error {
            E::Locked(_) => "locked",
            E::WrongPassword => "wrongPassword",
            E::StoreExists => "storeExists",
            E::NoSuchStore => "noSuchStore",
            E::PasswordChangedElsewhere => "passwordChangedElsewhere",
            E::InConflict(_) => "inConflict",
            E::Local(flint_store_fs::Error::AlreadyExists(_)) => "alreadyExists",
            _ => "other",
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

impl From<flint_registry::Error> for CmdError {
    fn from(error: flint_registry::Error) -> Self {
        Self::other(error.to_string())
    }
}

impl From<flint_core::Error> for CmdError {
    fn from(error: flint_core::Error) -> Self {
        Self::other(error.to_string())
    }
}
