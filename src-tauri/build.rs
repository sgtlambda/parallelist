fn main() {
    link_clang_runtime();
    // `relay::build_label` names the build after the branch `run.sh` built
    // it from, so a build from another branch must be a rebuild.
    println!("cargo:rerun-if-env-changed=VITE_PARALLELIST_BRANCH");
    tauri_build::build()
}

/// ONNX Runtime's prebuilt static library (screen search, through
/// fastembed) checks the macOS version with `@available`, which compiles to
/// a call to `__isPlatformVersionAtLeast`. Without LTO, Rust's std provides
/// that symbol; with the release profile's `lto = true` it is internalised
/// and dropped, since no Rust code calls it, and the link fails. Clang's own
/// runtime has it too, so it is linked from there.
fn link_clang_runtime() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Ok(out) = std::process::Command::new("xcrun")
        .args(["clang", "-print-runtime-dir"])
        .output()
    else {
        return;
    };
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && std::path::Path::new(&dir).join("libclang_rt.osx.a").exists() {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }
}
