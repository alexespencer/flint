//! Android Keystore access for Flint, as a Tauri plugin (the Kotlin half is in `android/`).
//!
//! Two hardware-backed AES-256-GCM keys, neither of which ever leaves the secure hardware:
//!
//! - **seal / open**: a key that needs no user action, for secrets the app must read by itself, such as
//!   the S3 secret key.
//! - **wrap / unwrap**: a key that needs a fingerprint for every single use, for a store's data key.
//!
//! Only meaningful on Android. On other platforms every call returns [`Error::Unsupported`].

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

pub use models::*;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod commands;
mod error;
mod models;

pub use error::{Error, Result};

#[cfg(desktop)]
pub use desktop::Keystore;
#[cfg(mobile)]
pub use mobile::Keystore;

/// Extensions to [`tauri::App`], [`tauri::AppHandle`] and [`tauri::Window`] to reach the keystore.
pub trait KeystoreExt<R: Runtime> {
    fn keystore(&self) -> &Keystore<R>;
}

impl<R: Runtime, T: Manager<R>> KeystoreExt<R> for T {
    fn keystore(&self) -> &Keystore<R> {
        self.state::<Keystore<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("keystore")
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::seal,
            commands::open,
            commands::wrap,
            commands::unwrap,
            commands::forget
        ])
        .setup(|app, api| {
            #[cfg(mobile)]
            let keystore = mobile::init(app, api)?;
            #[cfg(desktop)]
            let keystore = desktop::init(app, api)?;
            app.manage(keystore);
            Ok(())
        })
        .build()
}
