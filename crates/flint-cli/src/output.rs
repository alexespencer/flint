use std::io::{self, Write};

use flint_app_service::{Status, SyncOutcome};
use flint_sync::{ConflictOutcome, Refusal};

pub(crate) fn sync_outcome(out: &mut dyn Write, outcome: &SyncOutcome) -> io::Result<()> {
    let report = &outcome.report;
    let adopted = outcome.adopted.adopted.len();
    if adopted > 0 {
        writeln!(out, "encrypted {adopted} new file(s) from the folder")?;
    }
    for path in &outcome.adopted.waiting {
        writeln!(out, "waiting (still being written?): {path}")?;
    }
    let counts = [
        ("uploaded", report.uploaded.len()),
        ("downloaded", report.downloaded.len()),
        ("deleted on S3", report.deleted_remote.len()),
        ("moved to .trash", report.trashed_local.len()),
    ];
    let done: Vec<String> = counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(what, n)| format!("{n} {what}"))
        .collect();
    if done.is_empty() && report.conflicts.is_empty() {
        writeln!(out, "everything is in sync")?;
    } else if !done.is_empty() {
        writeln!(out, "{}", done.join(", "))?;
    }
    for conflict in &report.conflicts {
        match &conflict.outcome {
            ConflictOutcome::Diverged { copy } => writeln!(
                out,
                "CONFLICT {}: the other version is in {copy}; merge them, then `flint resolve {}`",
                conflict.path, conflict.path
            )?,
            ConflictOutcome::EditedHereDeletedThere => writeln!(
                out,
                "CONFLICT {}: edited here, deleted on the other side; keep it and `flint resolve {}`, or delete it",
                conflict.path, conflict.path
            )?,
            ConflictOutcome::RestoredEditedRemotely => writeln!(
                out,
                "{}: you deleted it here but it was edited on the other side, so it was restored",
                conflict.path
            )?,
        }
    }
    for (path, why) in &report.refused {
        writeln!(out, "left alone {path}: {}", refusal(why))?;
    }
    for (path, error) in &report.failed {
        writeln!(out, "FAILED {path}: {error}")?;
    }
    for (path, why) in &outcome.indexed.failed {
        writeln!(out, "not indexed {path}: {why}")?;
    }
    Ok(())
}

fn refusal(why: &Refusal) -> String {
    match why {
        Refusal::RemoteNotCiphertext => "the object on S3 is not encrypted by Flint".into(),
        Refusal::LocalNotCiphertext => "not encrypted yet".into(),
        Refusal::LocalNotDecryptable => {
            "this file was damaged or edited outside Flint and no longer opens; it was not uploaded, and the last good version is still on S3".into()
        }
        Refusal::RemoteNotDecryptable => {
            "the copy on S3 does not open (damaged there, or at the wrong name); it was not downloaded".into()
        }
        Refusal::HashMismatch { .. } => "the download did not match its recorded hash".into(),
        Refusal::LocalChangedDuringSync => "it changed while syncing; next time".into(),
        Refusal::KeyFileDiverged => {
            "the password was changed on two devices at once; unlock with the newer password".into()
        }
        Refusal::Contended => "S3 kept changing under it; next time".into(),
    }
}

pub(crate) fn status(out: &mut dyn Write, status: &Status, active: bool) -> io::Result<()> {
    writeln!(
        out,
        "{}{}  {}",
        status.name,
        if active { " (active)" } else { "" },
        status.folder.display()
    )?;
    writeln!(
        out,
        "  {}",
        if status.unlocked {
            "unlocked"
        } else {
            "locked"
        }
    )?;
    writeln!(out, "  {} file(s)", status.files)?;
    if !status.awaiting_encryption.is_empty() {
        writeln!(
            out,
            "  {} file(s) not encrypted yet (`flint sync` or `flint adopt`):",
            status.awaiting_encryption.len()
        )?;
        for path in &status.awaiting_encryption {
            writeln!(out, "    {path}")?;
        }
    }
    for conflict in &status.conflicts {
        match &conflict.copy {
            Some(copy) => writeln!(out, "  CONFLICT {} (other version: {copy})", conflict.path)?,
            None => writeln!(
                out,
                "  CONFLICT {} (deleted on the other side)",
                conflict.path
            )?,
        }
    }
    Ok(())
}
