use match_viewer_build::MatchViewer;
use std::env;
use std::path::PathBuf;

/// Builds the Bevy replay viewer and parks it in `OUT_DIR` for `main.rs` to
/// embed. Same helper the web crate uses, so the harness and the game always
/// show the same match.
fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let crate_dir = manifest_dir.join("..").join("..").join("src").join("match");
    let viewer_dir = out_dir.join("viewer");

    println!("cargo:rerun-if-changed=build.rs");
    MatchViewer::watch(&crate_dir);

    if !MatchViewer::skipped() {
        if let Err(reason) =
            MatchViewer::stage(&crate_dir, &out_dir.join("viewer-staging"), &viewer_dir)
        {
            println!("cargo:warning=match viewer not rebuilt: {}", reason);
        }
    }

    // `main.rs` reaches for these with `include_bytes!`, which has to resolve
    // even on a machine with no wasm toolchain. An empty file is the harness's
    // signal that there is no viewer to serve.
    MatchViewer::ensure_placeholders(&viewer_dir).expect("failed to stage match viewer");
}
