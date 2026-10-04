use serde::de::DeserializeOwned;
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::models::*;

/// Starts the Kotlin plugin class.
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<Keystore<R>> {
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin("dev.flint.keystore", "KeystorePlugin")?;
    #[cfg(target_os = "ios")]
    compile_error!("the keystore plugin is Android-only");
    Ok(Keystore(handle))
}

/// Access to the Android Keystore through the Kotlin plugin. Calls block until the Kotlin side answers
/// (for `wrap` and `unwrap`, until the user has used the fingerprint sensor), so make them from a
/// worker thread, never the UI thread.
pub struct Keystore<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> Keystore<R> {
    pub fn status(&self) -> crate::Result<Status> {
        self.0.run_mobile_plugin("status", ()).map_err(Into::into)
    }

    /// Encrypts under the hardware key that needs no user action.
    pub fn seal(&self, request: DataRequest) -> crate::Result<Sealed> {
        self.0
            .run_mobile_plugin("seal", request)
            .map_err(Into::into)
    }

    pub fn open(&self, sealed: Sealed) -> crate::Result<Opened> {
        self.0.run_mobile_plugin("open", sealed).map_err(Into::into)
    }

    /// Shows the fingerprint prompt, then encrypts under the fingerprint-gated hardware key.
    pub fn wrap(&self, request: DataRequest) -> crate::Result<Sealed> {
        self.0
            .run_mobile_plugin("wrap", request)
            .map_err(Into::into)
    }

    /// Shows the fingerprint prompt, then decrypts.
    pub fn unwrap(&self, sealed: Sealed) -> crate::Result<Opened> {
        self.0
            .run_mobile_plugin("unwrap", sealed)
            .map_err(Into::into)
    }

    /// Deletes the fingerprint-gated key; everything wrapped under it is unreadable afterwards.
    pub fn forget(&self) -> crate::Result<()> {
        self.0.run_mobile_plugin("forget", ()).map_err(Into::into)
    }
}
