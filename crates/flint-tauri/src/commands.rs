//! One function per thing the web view can ask for. Each only converts arguments, calls the app
//! service on its thread, and converts the answer: there is no logic here.

use flint_core::StorePath;
use flint_registry::StoreId;
use tauri::State;

use crate::dto::{BacklinkDto, CmdError, HitDto, NewStoreArgs, StoreDto, SyncDto};
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
        .call(move |svc| Box::pin(async move { svc.unlock(&id, &password).await }))
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

#[tauri::command]
pub async fn switch_store(worker: State<'_, Worker<Service>>, id: String) -> Result<()> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { svc.registry_mut().set_active(&id) }))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn forget_store(worker: State<'_, Worker<Service>>, id: String) -> Result<()> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { svc.registry_mut().remove(&id) }))
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
            Box::pin(async move { svc.change_password(&id, &old, &new).await.map(|_| ()) })
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
