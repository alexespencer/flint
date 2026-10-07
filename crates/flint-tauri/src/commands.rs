//! One function per thing the web view can ask for. Each only converts arguments, calls the app
//! service on its thread, and converts the answer: there is no logic here.

use flint_core::StorePath;
use flint_registry::StoreId;
use tauri::State;

use crate::dto::{BacklinkDto, CmdError, FolderDto, HitDto, NewStoreArgs, StoreDto, SyncDto};
use crate::{Service, Worker};

type Result<T> = std::result::Result<T, CmdError>;

fn store_id(id: &str) -> Result<StoreId> {
    StoreId::parse(id).ok_or_else(|| CmdError::other(format!("{id:?} is not a store id")))
}

fn path(path: &str) -> Result<StorePath> {
    Ok(StorePath::new(path)?)
}

#[tauri::command]
pub async fn list_stores(worker: State<'_, Worker<Service>>) -> Result<Vec<StoreDto>> {
    worker
        .call(|svc| {
            Box::pin(async move {
                let active = svc.registry().active().map(|s| s.id.clone());
                let ids: Vec<_> = svc.registry().list().iter().map(|s| s.id.clone()).collect();
                let mut stores = Vec::new();
                for id in ids {
                    let is_active = active.as_ref() == Some(&id);
                    let record = svc.registry().get(&id)?.clone();
                    stores.push(StoreDto::new(svc.status(&id)?, is_active, &record));
                }
                Ok::<_, flint_app_service::Error>(stores)
            })
        })
        .await
        .map_err(Into::into)
}

/// Where a store named `name` goes if the user does not choose.
#[tauri::command]
pub fn default_folder(app: tauri::AppHandle, name: String) -> Option<String> {
    crate::default_store_folder(&app, &name).map(|p| p.display().to_string())
}

#[tauri::command]
pub async fn create_store(
    worker: State<'_, Worker<Service>>,
    args: NewStoreArgs,
) -> Result<String> {
    worker
        .call(move |svc| {
            Box::pin(async move {
                let new = with_saved_secret(svc, args)?;
                svc.create_store(new).await.map(|id| id.to_string())
            })
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn join_store(worker: State<'_, Worker<Service>>, args: NewStoreArgs) -> Result<String> {
    worker
        .call(move |svc| {
            Box::pin(async move {
                let new = with_saved_secret(svc, args)?;
                svc.join_store(new).await.map(|id| id.to_string())
            })
        })
        .await
        .map_err(Into::into)
}

/// Fills in the S3 secret key from another store's saved one, if the user chose to reuse an account.
fn with_saved_secret(
    svc: &Service,
    args: NewStoreArgs,
) -> std::result::Result<flint_app_service::NewStore, flint_app_service::Error> {
    let from = args.reuse_secret_from.clone();
    let mut new: flint_app_service::NewStore = args.into();
    if let Some(from) = from {
        let id = StoreId::parse(&from).ok_or_else(|| {
            flint_app_service::Error::Registry(flint_registry::Error::Secrets(format!(
                "{from:?} is not a store id"
            )))
        })?;
        new.s3_secret = svc.reuse_s3_secret(&id)?;
    }
    Ok(new)
}

#[tauri::command]
pub async fn unlock(
    worker: State<'_, Worker<Service>>,
    id: String,
    password: String,
) -> Result<()> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { svc.unlock_noting(&id, &password).await }))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn lock(worker: State<'_, Worker<Service>>, id: String) -> Result<()> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { svc.lock(&id) }))
        .await
        .map_err(Into::into)
}

/// Locks every store: what the app's Lock button, auto-lock and going to the background all do.
#[tauri::command]
pub async fn lock_all(worker: State<'_, Worker<Service>>) -> Result<()> {
    worker
        .call(|svc| Box::pin(async move { svc.lock_all() }))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn switch_store(worker: State<'_, Worker<Service>>, id: String) -> Result<()> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { svc.registry_mut().set_active(&id) }))
        .await
        .map_err(Into::into)
}

/// Takes a store off this device (its folder is renamed, not deleted; everything is still on S3). Unless
/// `force`, the store is synced first and the removal is refused if anything would be left only on this
/// device. Returns where the folder went.
#[tauri::command]
pub async fn remove_store(
    app: tauri::AppHandle,
    worker: State<'_, Worker<Service>>,
    id: String,
    force: bool,
) -> Result<Option<String>> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| {
            Box::pin(async move {
                let removed = svc.remove_store(&id, force).await?;
                crate::biometric::forget(&app, svc, &id);
                Ok::<_, flint_app_service::Error>(
                    removed.folder_kept_as.map(|p| p.display().to_string()),
                )
            })
        })
        .await
        .map_err(Into::into)
}

/// Deletes a store from S3 for every device, then takes it off this one. `name` must be typed exactly.
/// Returns how many objects were deleted.
#[tauri::command]
pub async fn delete_store(
    app: tauri::AppHandle,
    worker: State<'_, Worker<Service>>,
    id: String,
    name: String,
) -> Result<usize> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| {
            Box::pin(async move {
                let deleted = svc.delete_store(&id, &name).await?;
                crate::biometric::forget(&app, svc, &id);
                Ok::<_, flint_app_service::Error>(deleted)
            })
        })
        .await
        .map_err(Into::into)
}

/// Puts this device's copy of a store back onto S3 after the store was deleted there.
#[tauri::command]
pub async fn reupload_store(worker: State<'_, Worker<Service>>, id: String) -> Result<SyncDto> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { svc.reupload_store(&id).await.map(SyncDto::from) }))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn change_password(
    worker: State<'_, Worker<Service>>,
    id: String,
    old: String,
    new: String,
) -> Result<()> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| {
            Box::pin(async move {
                svc.change_password_noting(&id, &old, &new)
                    .await
                    .map(|_| ())
            })
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn sync(worker: State<'_, Worker<Service>>, id: String) -> Result<SyncDto> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { svc.sync(&id).await.map(SyncDto::from) }))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resolve(worker: State<'_, Worker<Service>>, id: String, path: String) -> Result<()> {
    let (id, path) = (store_id(&id)?, self::path(&path)?);
    worker
        .call(move |svc| Box::pin(async move { svc.resolve(&id, &path).await }))
        .await
        .map_err(Into::into)
}

/// Every note and attachment, as store paths.
#[tauri::command]
pub async fn files(worker: State<'_, Worker<Service>>, id: String) -> Result<Vec<String>> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| {
            Box::pin(async move {
                svc.files(&id)
                    .map(|files| files.iter().map(ToString::to_string).collect())
            })
        })
        .await
        .map_err(Into::into)
}

/// Every folder, empty ones included, with when each was created (milliseconds since 1970).
#[tauri::command]
pub async fn folders(worker: State<'_, Worker<Service>>, id: String) -> Result<Vec<FolderDto>> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| {
            Box::pin(async move {
                svc.folders_created(&id).map(|dirs| {
                    dirs.into_iter()
                        .map(|(path, created)| FolderDto {
                            path: path.to_string(),
                            created_ms: created
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| d.as_millis() as u64),
                        })
                        .collect()
                })
            })
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_folder(
    worker: State<'_, Worker<Service>>,
    id: String,
    path: String,
) -> Result<()> {
    let (id, path) = (store_id(&id)?, self::path(&path)?);
    worker
        .call(move |svc| Box::pin(async move { svc.create_folder(&id, &path) }))
        .await
        .map_err(Into::into)
}

/// Moves a file or a folder to the trash; returns how many files went.
#[tauri::command]
pub async fn delete_path(
    worker: State<'_, Worker<Service>>,
    id: String,
    path: String,
) -> Result<usize> {
    let (id, path) = (store_id(&id)?, self::path(&path)?);
    worker
        .call(move |svc| Box::pin(async move { svc.delete_path(&id, &path) }))
        .await
        .map_err(Into::into)
}

/// Renames or moves a file or a folder; returns how many files moved.
#[tauri::command]
pub async fn move_path(
    worker: State<'_, Worker<Service>>,
    id: String,
    from: String,
    to: String,
) -> Result<usize> {
    let (id, from, to) = (store_id(&id)?, self::path(&from)?, self::path(&to)?);
    worker
        .call(move |svc| Box::pin(async move { svc.move_path(&id, &from, &to) }))
        .await
        .map_err(Into::into)
}

/// The decrypted bytes of a file (an image, say), sent to the web view as raw binary, not as JSON numbers.
#[tauri::command]
pub async fn read_file(
    worker: State<'_, Worker<Service>>,
    id: String,
    path: String,
) -> std::result::Result<tauri::ipc::Response, CmdError> {
    let (id, path) = (store_id(&id)?, self::path(&path)?);
    worker
        .call(move |svc| Box::pin(async move { svc.read_file(&id, &path) }))
        .await
        .map(tauri::ipc::Response::new)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn read_note(
    worker: State<'_, Worker<Service>>,
    id: String,
    path: String,
) -> Result<String> {
    let (id, path) = (store_id(&id)?, self::path(&path)?);
    worker
        .call(move |svc| Box::pin(async move { svc.read_note(&id, &path) }))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn save_note(
    worker: State<'_, Worker<Service>>,
    id: String,
    path: String,
    text: String,
) -> Result<()> {
    let (id, path) = (store_id(&id)?, self::path(&path)?);
    worker
        .call(move |svc| Box::pin(async move { svc.save_note(&id, &path, &text) }))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn search(
    worker: State<'_, Worker<Service>>,
    id: String,
    query: String,
) -> Result<Vec<HitDto>> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| {
            Box::pin(async move {
                svc.search(&id, &query, 50).map(|hits| {
                    hits.into_iter()
                        .map(|h| HitDto {
                            path: h.path.to_string(),
                            snippet: h.snippet,
                        })
                        .collect()
                })
            })
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn backlinks(
    worker: State<'_, Worker<Service>>,
    id: String,
    path: String,
) -> Result<Vec<BacklinkDto>> {
    let (id, path) = (store_id(&id)?, self::path(&path)?);
    worker
        .call(move |svc| {
            Box::pin(async move {
                svc.backlinks(&id, &path).map(|links| {
                    links
                        .into_iter()
                        .map(|b| BacklinkDto {
                            from: b.from.to_string(),
                        })
                        .collect()
                })
            })
        })
        .await
        .map_err(Into::into)
}

/// The file a `[[link]]` in `from` points to, if any.
#[tauri::command]
pub async fn resolve_link(
    worker: State<'_, Worker<Service>>,
    id: String,
    from: String,
    target: String,
) -> Result<Option<String>> {
    let (id, from) = (store_id(&id)?, path(&from)?);
    worker
        .call(move |svc| {
            Box::pin(async move {
                svc.resolve_link(&id, &from, &target)
                    .map(|p| p.map(|p| p.to_string()))
            })
        })
        .await
        .map_err(Into::into)
}
