//! Compiles the Bevy replay viewer to WebAssembly and stages it for embedding.
//!
//! Meant to be called from a `build.rs`; every entry point reports failure as a
//! human-readable string rather than panicking, because a machine without the
//! wasm target should still be able to build the rest of the project.

use flate2::Compression;
use flate2::write::GzEncoder;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct MatchViewer;

impl MatchViewer {
    pub const TARGET: &'static str = "wasm32-unknown-unknown";
    pub const OUT_NAME: &'static str = "match_viewer";
    /// Set to skip the (slow) wasm build and keep whatever is already staged.
    pub const SKIP_ENV: &'static str = "OPEN_FOOTBALL_SKIP_MATCH_VIEWER";

    pub fn script_file() -> String {
        format!("{}.js.gz", Self::OUT_NAME)
    }

    pub fn wasm_file() -> String {
        format!("{}_bg.wasm.gz", Self::OUT_NAME)
    }

    /// Tells cargo which viewer sources should trigger a rebuild.
    pub fn watch(crate_dir: &Path) {
        println!("cargo:rerun-if-changed={}", crate_dir.join("src").display());
        println!(
            "cargo:rerun-if-changed={}",
            crate_dir.join("Cargo.toml").display()
        );
        // The viewer's one path dependency, and the only source outside its
        // own directory that ends up inside the wasm. Left off this list it
        // would go stale in exactly the way it exists to prevent: the server
        // would rebuild against a new palette and the viewer would keep
        // shipping the old one, and every player would change colour on the
        // way to the pitch.
        if let Some(workspace) = crate_dir.parent() {
            println!(
                "cargo:rerun-if-changed={}",
                workspace.join("shared").join("src").display()
            );
        }
        println!(
            "cargo:rerun-if-changed={}",
            crate_dir.join(".cargo").join("config.toml").display()
        );
        println!("cargo:rerun-if-env-changed={}", Self::SKIP_ENV);
    }

    pub fn skipped() -> bool {
        env::var_os(Self::SKIP_ENV).is_some()
    }

    /// Builds the viewer and writes `match_viewer.js.gz` and
    /// `match_viewer_bg.wasm.gz` into `assets_dir`.
    ///
    /// `staging` is scratch space for the raw wasm-bindgen output — put it
    /// under `OUT_DIR`.
    ///
    /// Cheap when nothing moved. The nested cargo build settles in well under a
    /// second once its target dir is warm, but wasm-bindgen and a level-9 gzip
    /// over ~30 MB of Bevy cost seconds every time, and the build script re-runs
    /// for reasons that have nothing to do with the viewer — a stylesheet edit
    /// is enough. So the compiled wasm is fingerprinted and the rest of the
    /// pipeline is skipped whole when that fingerprint matches what is already
    /// staged.
    pub fn stage(crate_dir: &Path, staging: &Path, assets_dir: &Path) -> Result<(), String> {
        let wasm = Self::compile(crate_dir)?;
        let fingerprint = Self::fingerprint(&wasm)?;

        if Self::staged(assets_dir) && Self::stamp(staging).as_deref() == Some(&fingerprint) {
            return Ok(());
        }

        Self::bindgen(&wasm, staging)?;
        Self::compress(staging, assets_dir)?;
        Self::stamp_write(staging, &fingerprint)
    }

    /// Content hash of the compiled wasm, used as the staging cache key.
    fn fingerprint(wasm: &Path) -> Result<String, String> {
        let bytes = fs::read(wasm)
            .map_err(|error| format!("could not read {}: {}", wasm.display(), error))?;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Ok(format!("{:x}", hasher.finish()))
    }

    fn stamp_path(staging: &Path) -> PathBuf {
        staging.join("input-fingerprint")
    }

    fn stamp(staging: &Path) -> Option<String> {
        fs::read_to_string(Self::stamp_path(staging))
            .ok()
            .map(|stamp| stamp.trim().to_string())
    }

    /// Written last, so a run that dies half way through re-does the work
    /// rather than declaring a partial staging directory good.
    fn stamp_write(staging: &Path, fingerprint: &str) -> Result<(), String> {
        let path = Self::stamp_path(staging);
        fs::write(&path, fingerprint)
            .map_err(|error| format!("could not write {}: {}", path.display(), error))
    }

    /// Creates empty stand-ins so a consumer that reaches for the staged files
    /// unconditionally — `include_bytes!`, say — still compiles on a machine
    /// with no wasm toolchain. [`MatchViewer::staged`] reports these as absent.
    pub fn ensure_placeholders(assets_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(assets_dir)
            .map_err(|error| format!("could not create {}: {}", assets_dir.display(), error))?;
        for name in [Self::script_file(), Self::wasm_file()] {
            let path = assets_dir.join(name);
            if !path.exists() {
                fs::write(&path, [])
                    .map_err(|error| format!("could not create {}: {}", path.display(), error))?;
            }
        }
        Ok(())
    }

    /// True when a real viewer — not a placeholder — is sitting in `assets_dir`.
    pub fn staged(assets_dir: &Path) -> bool {
        [Self::script_file(), Self::wasm_file()]
            .iter()
            .all(|name| fs::metadata(assets_dir.join(name)).is_ok_and(|meta| meta.len() > 0))
    }

    /// Short content hash of the staged viewer, for cache busting. `"none"`
    /// when nothing is staged.
    pub fn version(assets_dir: &Path) -> String {
        let Ok(bytes) = fs::read(assets_dir.join(Self::wasm_file())) else {
            return "none".to_string();
        };
        if bytes.is_empty() {
            return "none".to_string();
        }
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = format!("{:x}", hasher.finish());
        hash[..8.min(hash.len())].to_string()
    }

    fn compile(crate_dir: &Path) -> Result<PathBuf, String> {
        if !crate_dir.join("Cargo.toml").exists() {
            return Err(format!("{} is missing", crate_dir.display()));
        }
        if !Self::target_installed() {
            return Err(format!(
                "the {target} target is not installed — run `rustup target add {target}`",
                target = Self::TARGET
            ));
        }

        let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut command = Command::new(cargo);
        command
            .current_dir(crate_dir)
            .args(["build", "--release", "--target", Self::TARGET]);

        // A build script inherits the outer build's cargo environment whole:
        // the outer target directory, the outer rustflags, the calling
        // package's features, and — the one that actually bites — the host's
        // `CARGO_CFG_*` view of the world. wgpu's build script reads
        // `CARGO_CFG_WINDOWS` to decide whether to enable its Vulkan backend,
        // so leaving it in place makes a WebAssembly build reach for Vulkan and
        // fail to compile. Cargo sets the right values for the nested build
        // itself; the job here is purely to stop the wrong ones from surviving.
        for (key, _) in env::vars() {
            if key.starts_with("CARGO_CFG_")
                || key.starts_with("CARGO_FEATURE_")
                || key.starts_with("CARGO_PKG_")
                || key.starts_with("DEP_")
            {
                command.env_remove(key);
            }
        }
        for leaked in [
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTFLAGS",
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "CARGO_BUILD_TARGET_DIR",
            "CARGO_TARGET_DIR",
            "CARGO_MAKEFLAGS",
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_MANIFEST_LINKS",
            "RUSTC_WORKSPACE_WRAPPER",
            "OUT_DIR",
            "TARGET",
            "HOST",
            "PROFILE",
            "DEBUG",
            "OPT_LEVEL",
            "NUM_JOBS",
        ] {
            command.env_remove(leaked);
        }

        let output = command
            .output()
            .map_err(|error| format!("could not run cargo: {}", error))?;

        if !output.status.success() {
            // Cargo hides a build script's own stderr, so the nested compiler
            // errors have to be re-emitted as warnings or they vanish.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = stderr.lines().rev().take(25).collect();
            for line in tail.into_iter().rev() {
                println!("cargo:warning=  {}", line);
            }
            return Err("the viewer crate failed to compile".to_string());
        }

        let wasm = crate_dir
            .join("target")
            .join(Self::TARGET)
            .join("release")
            .join(format!("{}.wasm", Self::OUT_NAME));
        if !wasm.exists() {
            return Err(format!("{} was not produced", wasm.display()));
        }
        Ok(wasm)
    }

    fn bindgen(wasm: &Path, staging: &Path) -> Result<(), String> {
        fs::create_dir_all(staging)
            .map_err(|error| format!("could not create {}: {}", staging.display(), error))?;

        let mut bindgen = wasm_bindgen_cli_support::Bindgen::new();
        bindgen
            .input_path(wasm)
            .out_name(Self::OUT_NAME)
            .typescript(false)
            .debug(false)
            .keep_debug(false)
            .remove_name_section(true)
            .remove_producers_section(true);
        bindgen
            .web(true)
            .map_err(|error| format!("wasm-bindgen: {}", error))?;
        bindgen
            .generate(staging)
            .map_err(|error| format!("wasm-bindgen: {}", error))
    }

    /// Moves the wasm-bindgen output into `assets_dir`, gzipped.
    ///
    /// The viewer is ~30 MB of Bevy, wgpu and naga. Storing it pre-compressed
    /// cuts that to about a quarter both in the binary and on the wire, and the
    /// servers pass the bytes through untouched under `Content-Encoding: gzip`
    /// — nothing ever holds the inflated copy.
    fn compress(staging: &Path, assets_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(assets_dir)
            .map_err(|error| format!("could not create {}: {}", assets_dir.display(), error))?;

        let staged = [Self::script_file(), Self::wasm_file()];

        // Anything else in here gets embedded too, and an earlier build's
        // uncompressed copy would double the binary for nothing. Swept rather
        // than solved with `remove_dir_all` so the two files that belong here
        // keep their timestamps — see `write_if_changed`.
        Self::sweep(assets_dir, &staged);

        for name in [
            format!("{}.js", Self::OUT_NAME),
            format!("{}_bg.wasm", Self::OUT_NAME),
        ] {
            let source = staging.join(&name);
            let bytes = fs::read(&source)
                .map_err(|error| format!("could not read {}: {}", source.display(), error))?;

            let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
            encoder
                .write_all(&bytes)
                .and_then(|()| encoder.finish())
                .map_err(|error| format!("could not compress {}: {}", name, error))
                .and_then(|compressed| {
                    Self::write_if_changed(&assets_dir.join(format!("{}.gz", name)), &compressed)
                        .map_err(|error| format!("could not stage {}: {}", name, error))
                })?;
        }
        Ok(())
    }

    /// Deletes everything in `dir` that is not one of `keep`.
    fn sweep(dir: &Path, keep: &[String]) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if keep.iter().any(|kept| name == kept.as_str()) {
                continue;
            }
            let path = entry.path();
            let _ = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
        }
    }

    /// Writes only when the bytes actually differ.
    ///
    /// These files are embedded by `rust-embed`, which puts them in the
    /// consuming crate's dependency list. Rewriting one with identical
    /// contents still moves its mtime, and that alone is enough to recompile
    /// the web crate and re-link the binary behind it — two minutes of LTO to
    /// arrive back at the same bytes.
    fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if fs::read(path).is_ok_and(|existing| existing == bytes) {
            return Ok(());
        }
        fs::write(path, bytes)
    }

    fn target_installed() -> bool {
        let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
        Command::new(rustc)
            .args(["--print", "target-libdir", "--target", Self::TARGET])
            .output()
            .map(|output| {
                output.status.success()
                    && Path::new(String::from_utf8_lossy(&output.stdout).trim()).exists()
            })
            .unwrap_or(false)
    }
}
