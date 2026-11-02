fn main() {
    // the icons are built into the binary, but tauri_build doesn't watch them
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
