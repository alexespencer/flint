//! M0 spike: a Rust command writes and reads a file in the app's private storage.
//! On Android that is `/data/user/0/<identifier>/files`; on macOS `~/Library/Application Support/<identifier>`.

use std::fs;

use tauri::Manager;

#[tauri::command]
fn storage_probe(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join("probe.md");
    let body = format!("# probe\nwritten by Rust at {:?}\n", std::time::SystemTime::now());
    fs::write(&file, &body).map_err(|e| e.to_string())?;
    let back = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    if back != body {
        return Err("read-back differs from what was written".into());
    }
    Ok(format!("ok: {} ({} bytes)", file.display(), back.len()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_keystore::init())
        .invoke_handler(tauri::generate_handler![storage_probe])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
