// Keep the console window away on Windows release builds; harmless elsewhere.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    flint_tauri::run();
}
