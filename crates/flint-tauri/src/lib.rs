//! The Flint desktop app. Everything of substance is in the crates below; this one wires the app
//! service to the web view.

mod commands;
mod dto;
mod worker;

use flint_app_service::{AppService, S3Connect};
use flint_registry::Registry;
use flint_sync::DeviceName;

pub use worker::Worker;

#[cfg(target_os = "macos")]
type Secrets = flint_registry::KeychainSecrets;
#[cfg(not(target_os = "macos"))]
type Secrets = flint_registry::MemorySecrets;

/// The app service, as the worker thread holds it.
pub type Service = AppService<Secrets, S3Connect>;

#[cfg(target_os = "macos")]
fn secrets() -> Secrets {
    // The same keychain service as the CLI, so the two see the same stores and secrets.
    Secrets::new("dev.flint")
}

#[cfg(not(target_os = "macos"))]
fn secrets() -> Secrets {
    Secrets::new()
}

/// Where the list of stores is kept (shared with the CLI).
fn config_dir() -> std::path::PathBuf {
    dirs::config_dir().expect("a config folder").join("Flint")
}

fn open_service() -> Service {
    let registry = Registry::open(config_dir(), secrets()).expect("the store list could be opened");
    let device = DeviceName::new(&gethostname::gethostname().to_string_lossy());
    AppService::new(registry, S3Connect, device)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Worker::start(open_service))
        .invoke_handler(tauri::generate_handler![
            commands::list_stores,
            commands::default_folder,
            commands::create_store,
            commands::join_store,
            commands::unlock,
            commands::lock,
            commands::switch_store,
            commands::forget_store,
            commands::change_password,
            commands::sync,
            commands::resolve,
            commands::files,
            commands::read_note,
            commands::save_note,
            commands::search,
            commands::backlinks,
            commands::resolve_link,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Flint");
}
