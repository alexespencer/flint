fn main() {
    // `tauri::generate_context!` needs the frontend's output folder to exist, even before the frontend
    // has been built, so that `cargo build --workspace` works on a fresh checkout.
    let _ = std::fs::create_dir_all("../../app/dist");
    tauri_build::build();
}
