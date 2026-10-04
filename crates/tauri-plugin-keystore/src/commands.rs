use tauri::{AppHandle, Runtime, command};

use crate::models::*;
use crate::{KeystoreExt, Result};

#[command]
pub(crate) async fn status<R: Runtime>(app: AppHandle<R>) -> Result<Status> {
    app.keystore().status()
}

#[command]
pub(crate) async fn seal<R: Runtime>(app: AppHandle<R>, payload: DataRequest) -> Result<Sealed> {
    app.keystore().seal(payload)
}

#[command]
pub(crate) async fn open<R: Runtime>(app: AppHandle<R>, payload: Sealed) -> Result<Opened> {
    app.keystore().open(payload)
}

#[command]
pub(crate) async fn wrap<R: Runtime>(app: AppHandle<R>, payload: DataRequest) -> Result<Sealed> {
    app.keystore().wrap(payload)
}

#[command]
pub(crate) async fn unwrap<R: Runtime>(app: AppHandle<R>, payload: Sealed) -> Result<Opened> {
    app.keystore().unwrap(payload)
}

#[command]
pub(crate) async fn forget<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.keystore().forget()
}
