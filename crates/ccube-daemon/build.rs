//! Build script: ensure the embedded frontend (`include_dir!` in `http.rs`) is
//! re-embedded whenever the SvelteKit `build/` output changes. Without this,
//! Cargo has no idea the embedded directory changed and would serve a stale UI.

fn main() {
    println!("cargo:rerun-if-changed=../../build");
}
