use tauri::{command, AppHandle, Runtime};

use crate::models::*;
use crate::KeystoreExt;
use crate::Result;

#[command]
pub(crate) async fn status<R: Runtime>(app: AppHandle<R>) -> Result<Status> {
  app.keystore().status()
}

#[command]
pub(crate) async fn wrap<R: Runtime>(app: AppHandle<R>, payload: WrapRequest) -> Result<Wrapped> {
  app.keystore().wrap(payload)
}

#[command]
pub(crate) async fn unwrap<R: Runtime>(app: AppHandle<R>, payload: UnwrapRequest) -> Result<Unwrapped> {
  app.keystore().unwrap(payload)
}

#[command]
pub(crate) async fn forget<R: Runtime>(app: AppHandle<R>) -> Result<()> {
  app.keystore().forget()
}
