fn main() {
    // `tauri::generate_context!` needs the frontend's output folder to exist, even before the frontend
    // has been built, so that `cargo build --workspace` works on a fresh checkout.
    let _ = std::fs::create_dir_all("../../app/dist");
    // The icons are embedded in the binary at compile time, so changing one must rebuild it.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build();
}
