fn main() {
    // Cargo does not watch frontendDist by default — without these, the
    // desktop binary can keep serving a stale baked HTML (e.g. 3 presets)
    // while http://127.0.0.1:1420 already shows the updated UI.
    println!("cargo:rerun-if-changed=../src/index.html");
    println!("cargo:rerun-if-changed=../src/app.js");
    println!("cargo:rerun-if-changed=../src/styles.css");
    println!("cargo:rerun-if-changed=../src/lib.js");
    println!("cargo:rerun-if-changed=../src/seed.js");
    tauri_build::build()
}
