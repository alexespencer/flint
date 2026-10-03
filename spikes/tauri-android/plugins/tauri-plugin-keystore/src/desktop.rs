use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::models::*;

pub fn init<R: Runtime, C: DeserializeOwned>(
  app: &AppHandle<R>,
  _api: PluginApi<R, C>,
) -> crate::Result<Keystore<R>> {
  Ok(Keystore(app.clone()))
}

/// Desktop stub: macOS uses the keychain + LAContext (see spikes/keychain), not this plugin.
pub struct Keystore<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Keystore<R> {
  pub fn status(&self) -> crate::Result<Status> {
    Err(crate::Error::Unsupported)
  }

  pub fn wrap(&self, _request: WrapRequest) -> crate::Result<Wrapped> {
    Err(crate::Error::Unsupported)
  }

  pub fn unwrap(&self, _request: UnwrapRequest) -> crate::Result<Unwrapped> {
    Err(crate::Error::Unsupported)
  }

  pub fn forget(&self) -> crate::Result<()> {
    Err(crate::Error::Unsupported)
  }
}
