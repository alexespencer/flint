const COMMANDS: &[&str] = &["status", "seal", "open", "wrap", "unwrap", "forget"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
