use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::models::*;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Keystore<R>> {
    Ok(Keystore(app.clone()))
}

/// Desktop stub: macOS uses the login keychain, not this plugin.
pub struct Keystore<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Keystore<R> {
    pub fn status(&self) -> crate::Result<Status> {
        Err(crate::Error::Unsupported)
    }

    pub fn seal(&self, _request: DataRequest) -> crate::Result<Sealed> {
        Err(crate::Error::Unsupported)
    }

    pub fn open(&self, _sealed: Sealed) -> crate::Result<Opened> {
        Err(crate::Error::Unsupported)
    }

    pub fn wrap(&self, _request: DataRequest) -> crate::Result<Sealed> {
        Err(crate::Error::Unsupported)
    }

    pub fn unwrap(&self, _sealed: Sealed) -> crate::Result<Opened> {
        Err(crate::Error::Unsupported)
    }

    pub fn forget(&self) -> crate::Result<()> {
        Err(crate::Error::Unsupported)
    }
}
