use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use flint_app_service::{AppService, Connect, NewStore};
use flint_core::StorePath;
use flint_registry::{RemoteSettings, SecretStore, StoreRecord};
use thiserror::Error;

use crate::{Cli, Command, Input, RemoteArgs, output};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Service(#[from] flint_app_service::Error),

    #[error(transparent)]
    Registry(#[from] flint_registry::Error),

    #[error(transparent)]
    Path(#[from] flint_core::Error),

    #[error("no store called {0:?}; `flint stores` lists them")]
    NoSuchStore(String),

    #[error(
        "there is no active store: create one with `flint create-store`, or add one with `flint join`"
    )]
    NoActiveStore,

    #[error("the two passwords do not match")]
    PasswordsDiffer,

    #[error("cannot work out where to put the store folder; give --folder")]
    NoHome,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Everything a command needs from its surroundings.
pub struct Env<'a, S, C> {
    pub service: AppService<S, C>,
    pub input: &'a mut dyn Input,
    pub out: &'a mut dyn Write,
}

pub async fn run<S: SecretStore, C: Connect>(cli: Cli, env: &mut Env<'_, S, C>) -> Result<()> {
    let store = cli.store.as_deref();
    match cli.command {
        Command::CreateStore { name, remote } => {
            let s3_secret = env.input.secret("S3 secret access key")?;
            let password = env.input.secret("New password")?;
            if env.input.secret("Repeat password")? != password {
                return Err(Error::PasswordsDiffer);
            }
            let new = new_store(&name, remote, s3_secret, password)?;
            let id = env.service.create_store(new).await?;
            writeln!(env.out, "created store {name:?}")?;
            sync_and_print(env, &id).await
        }
        Command::Join { name, remote } => {
            let s3_secret = env.input.secret("S3 secret access key")?;
            let password = env.input.secret("Password")?;
            let new = new_store(&name, remote, s3_secret, password)?;
            let id = env.service.join_store(new).await?;
            writeln!(env.out, "joined store {name:?}")?;
            sync_and_print(env, &id).await
        }
        Command::Stores => {
            let active = env.service.registry().active().map(|s| s.id.clone());
            let ids: Vec<_> = env
                .service
                .registry()
                .list()
                .iter()
                .map(|s| s.id.clone())
                .collect();
            if ids.is_empty() {
                writeln!(env.out, "no stores yet")?;
            }
            for id in ids {
                let status = env.service.status(&id)?;
                output::status(env.out, &status, active.as_ref() == Some(&id))?;
            }
            Ok(())
        }
        Command::Switch { name } => {
            let id = find(env, Some(&name))?.id;
            env.service.registry_mut().set_active(&id)?;
            writeln!(env.out, "{name} is now the active store")?;
            Ok(())
        }
        Command::Unlock => {
            let record = find(env, store)?;
            let password = env.input.secret("Password")?;
            env.service.unlock(&record.id, &password).await?;
            writeln!(env.out, "{} is unlocked", record.name)?;
            Ok(())
        }
        Command::Lock => {
            let record = find(env, store)?;
            env.service.lock(&record.id)?;
            writeln!(env.out, "{} is locked", record.name)?;
            Ok(())
        }
        Command::Passwd => {
            let record = find(env, store)?;
            let old = env.input.secret("Current password")?;
            let new = env.input.secret("New password")?;
            if env.input.secret("Repeat new password")? != new {
                return Err(Error::PasswordsDiffer);
            }
            let report = env.service.change_password(&record.id, &old, &new).await?;
            if report.failed.is_empty() {
                writeln!(
                    env.out,
                    "password changed; other devices will pick it up when they sync"
                )?;
            } else {
                writeln!(
                    env.out,
                    "password changed here, but not uploaded yet (run `flint sync` when online)"
                )?;
            }
            Ok(())
        }
        Command::Adopt { now } => {
            let record = find(env, store)?;
            let min_age = if now {
                Duration::ZERO
            } else {
                Duration::from_secs(2)
            };
            let (adopted, _) = env.service.adopt(&record.id, min_age)?;
            writeln!(env.out, "encrypted {} file(s)", adopted.adopted.len())?;
            for path in &adopted.waiting {
                writeln!(env.out, "waiting (still being written?): {path}")?;
            }
            for (path, error) in &adopted.failed {
                writeln!(env.out, "FAILED {path}: {error}")?;
            }
            Ok(())
        }
        Command::Sync => {
            let record = find(env, store)?;
            sync_and_print(env, &record.id).await
        }
        Command::Status => {
            let record = find(env, store)?;
            let status = env.service.status(&record.id)?;
            let active = env.service.registry().active().map(|s| &s.id) == Some(&record.id);
            output::status(env.out, &status, active)?;
            Ok(())
        }
        Command::Resolve { path } => {
            let record = find(env, store)?;
            env.service
                .resolve(&record.id, &StorePath::new(&path)?)
                .await?;
            writeln!(
                env.out,
                "{path} is resolved; the next sync will carry your version"
            )?;
            Ok(())
        }
        Command::Forget { name } => {
            let record = find(env, Some(&name))?;
            env.service.registry_mut().remove(&record.id)?;
            writeln!(
                env.out,
                "forgot {name:?}; its folder and its objects on S3 were left alone"
            )?;
            Ok(())
        }
    }
}

async fn sync_and_print<S: SecretStore, C: Connect>(
    env: &mut Env<'_, S, C>,
    id: &flint_registry::StoreId,
) -> Result<()> {
    let outcome = env.service.sync(id).await?;
    output::sync_outcome(env.out, &outcome)?;
    Ok(())
}

fn find<S: SecretStore, C: Connect>(
    env: &Env<'_, S, C>,
    name: Option<&str>,
) -> Result<StoreRecord> {
    let registry = env.service.registry();
    match name {
        Some(name) => registry
            .find(name)
            .cloned()
            .ok_or_else(|| Error::NoSuchStore(name.to_owned())),
        None => registry.active().cloned().ok_or(Error::NoActiveStore),
    }
}

fn new_store(
    name: &str,
    remote: RemoteArgs,
    s3_secret: String,
    password: String,
) -> Result<NewStore> {
    let folder = match remote.folder {
        Some(folder) => folder,
        None => default_folder(name)?,
    };
    Ok(NewStore {
        name: name.to_owned(),
        folder,
        remote: RemoteSettings {
            bucket: remote.bucket,
            prefix: remote.prefix.unwrap_or_else(|| name.to_owned()),
            region: remote.region,
            endpoint: remote.endpoint,
            access_key_id: remote.access_key_id,
        },
        s3_secret,
        password,
    })
}

fn default_folder(name: &str) -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or(Error::NoHome)?
        .join("Flint")
        .join(name))
}
