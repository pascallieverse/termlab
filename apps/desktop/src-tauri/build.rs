fn main() {
    // `tauri::generate_context!` embeds the built frontend. On a fresh checkout `../dist`
    // doesn't exist yet, which would make `cargo build`/`clippy` fail before anyone has run
    // `npm run build`. Create a placeholder page so the Rust side always compiles; the real
    // frontend replaces it (`npm run build`, or `npm run tauri dev`).
    let dist = std::path::Path::new("../dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).expect("create ../dist");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>Termlab</title><p>Frontend not built. Run <code>npm run build</code> in apps/desktop.</p>",
        )
        .expect("write placeholder index.html");
    }
    tauri_build::build()
}
