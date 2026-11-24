fn main() {
    // `relay::build_label` names the build after the branch `run.sh` built
    // it from, so a build from another branch must be a rebuild.
    println!("cargo:rerun-if-env-changed=VITE_PARALLELIST_BRANCH");
    tauri_build::build()
}
