use serde::de::DeserializeOwned;
use tauri::{
  plugin::{PluginApi, PluginHandle},
  AppHandle, Runtime,
};

use crate::models::*;

// initializes the Kotlin plugin class
pub fn init<R: Runtime, C: DeserializeOwned>(
  _app: &AppHandle<R>,
  api: PluginApi<R, C>,
) -> crate::Result<Keystore<R>> {
  #[cfg(target_os = "android")]
  let handle = api.register_android_plugin("dev.flint.keystore", "KeystorePlugin")?;
  #[cfg(target_os = "ios")]
  compile_error!("the keystore spike is Android-only");
  Ok(Keystore(handle))
}

/// Access to the Android Keystore through the Kotlin plugin.
pub struct Keystore<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> Keystore<R> {
  pub fn status(&self) -> crate::Result<Status> {
    self.0.run_mobile_plugin("status", ()).map_err(Into::into)
  }

  /// Shows the fingerprint prompt, then encrypts `data` under the hardware key.
  pub fn wrap(&self, request: WrapRequest) -> crate::Result<Wrapped> {
    self.0.run_mobile_plugin("wrap", request).map_err(Into::into)
  }

  /// Shows the fingerprint prompt, then decrypts.
  pub fn unwrap(&self, request: UnwrapRequest) -> crate::Result<Unwrapped> {
    self.0.run_mobile_plugin("unwrap", request).map_err(Into::into)
  }

  pub fn forget(&self) -> crate::Result<()> {
    self.0.run_mobile_plugin("forget", ()).map_err(Into::into)
  }
}
