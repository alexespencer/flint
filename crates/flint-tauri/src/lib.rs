//! The Flint desktop app. Everything of substance is in the crates below; this one wires the app
//! service to the web view.

#[cfg(target_os = "android")]
mod android_secrets;
mod biometric;
mod commands;
mod dto;
#[cfg(target_os = "macos")]
mod touchid;
mod worker;

use flint_app_service::{AppService, S3Connect};
use flint_registry::Registry;
use flint_sync::DeviceName;

pub use worker::Worker;

#[cfg(target_os = "macos")]
type Secrets = flint_registry::KeychainSecrets;
#[cfg(target_os = "android")]
type Secrets = android_secrets::AndroidSecrets;
#[cfg(not(any(target_os = "macos", target_os = "android")))]
type Secrets = flint_registry::MemorySecrets;

/// The app service, as the worker thread holds it.
pub type Service = AppService<Secrets, S3Connect>;

#[cfg(target_os = "macos")]
fn secrets(_: &tauri::AppHandle) -> Secrets {
    // The same keychain service as the CLI, so the two see the same stores and secrets.
    Secrets::new("dev.flint")
}

#[cfg(target_os = "android")]
fn secrets(app: &tauri::AppHandle) -> Secrets {
    Secrets::new(app.clone(), config_dir(app))
}

#[cfg(not(any(target_os = "macos", target_os = "android")))]
fn secrets(_: &tauri::AppHandle) -> Secrets {
    Secrets::new()
}

/// Where the list of stores is kept. On the desktop that is the platform's config folder, shared with
/// the CLI; on a phone, the app's private storage.
pub(crate) fn config_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    #[cfg(mobile)]
    {
        use tauri::Manager;
        app.path()
            .app_data_dir()
            .expect("an app data folder")
            .join("config")
    }
    #[cfg(not(mobile))]
    {
        let _ = app;
        dirs::config_dir().expect("a config folder").join("Flint")
    }
}

/// Where a new store's folder goes unless the user chooses: `~/Flint/<name>` on the desktop, and in the
/// app's private storage on a phone (where there is no folder picker, and other apps cannot read it).
pub(crate) fn default_store_folder(
    app: &tauri::AppHandle,
    name: &str,
) -> Option<std::path::PathBuf> {
    #[cfg(mobile)]
    {
        use tauri::Manager;
        app.path()
            .app_data_dir()
            .ok()
            .map(|d| d.join("stores").join(name))
    }
    #[cfg(not(mobile))]
    {
        let _ = app;
        dirs::home_dir().map(|h| h.join("Flint").join(name))
    }
}

fn open_service(app: &tauri::AppHandle) -> Service {
    let registry =
        Registry::open(config_dir(app), secrets(app)).expect("the store list could be opened");
    // A phone's host name is not something a person would recognise in a conflict file's name.
    let device = if cfg!(target_os = "android") {
        DeviceName::new("Phone")
    } else {
        DeviceName::new(&gethostname::gethostname().to_string_lossy())
    };
    AppService::new(registry, S3Connect, device)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(tauri_plugin_keystore::init());
    builder
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::Manager::manage(app, Worker::start(move || open_service(&handle)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_stores,
            commands::default_folder,
            commands::create_store,
            commands::join_store,
            commands::unlock,
            commands::lock,
            commands::lock_all,
            commands::switch_store,
            commands::forget_store,
            commands::change_password,
            commands::sync,
            commands::resolve,
            commands::files,
            commands::folders,
            commands::create_folder,
            commands::delete_path,
            commands::move_path,
            commands::read_note,
            commands::save_note,
            commands::search,
            commands::backlinks,
            commands::resolve_link,
            biometric::biometric_state,
            biometric::biometric_enable,
            biometric::biometric_unlock,
            biometric::biometric_disable,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Flint");
}
