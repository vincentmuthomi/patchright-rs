//! Build script for playwright-rs
//!
//! Assembles the Playwright Node.js driver from two artifacts — the
//! `playwright-core` npm tarball (registry.npmjs.org) and a pinned Node.js
//! binary (nodejs.org) — into the same `node` + `package/` layout the old
//! prebuilt CDN zip provided (see ADR 0006; the prebuilt zips were
//! discontinued when the azureedge CDN shut down). The runtime side
//! (`src/server/driver.rs`) picks the path up via compile-time
//! `option_env!()` lookups.
//!
//! By default the driver lands in the user cache, at the same version-keyed
//! path `playwright-rs install` writes and the runtime lookup probes
//! (`<cache>/playwright-rust/<version>/playwright-<version>-<platform>-node<node>`; see
//! ADR 0007). One download then serves every target directory, every
//! cargo-mutants copy, and every workspace on the machine, and a consumer
//! that has built once is offline-safe. The assembled files are watched, so
//! a cleaned cache reassembles on the next build. Two env knobs change it:
//!
//! - `PLAYWRIGHT_DRIVER_CACHE_DIR` puts the driver somewhere specific, for a
//!   CI job that caches an explicit path or a machine whose home is not
//!   the place for it.
//! - `PLAYWRIGHT_SKIP_DRIVER_DOWNLOAD` skips the download entirely for
//!   compile-only jobs (e.g. the MSRV `cargo check`) that never launch a
//!   browser, saving a ~90 MB fetch.
//!
//! A download that fails (offline, an outage) warns and builds without a
//! driver, so launch reports `ServerNotFound`. A download that does not
//! match its pinned digest fails the build.
//!
//! `$OUT_DIR` is the fallback for a process with no resolvable home
//! directory. Cargo's convention is that build scripts write only there;
//! this one writes to the cache on purpose, and the trade is recorded in
//! the ADR.

use std::env;
use std::path::{Path, PathBuf};

// PATCHRIGHT: the version of the `patchright-core` npm package (see
// `NPM_CORE_PACKAGE` in driver_urls.rs). Patchright follows Playwright's
// major.minor but numbers its own patch releases, and it can lag the
// playwright-rust release this crate was forked from by a minor version.
// Browsers come from this version's `browsers.json`, so `patchright-rs
// install chromium` must use this driver. `cargo xtask update-driver` moves
// this pin and everything that goes with it.
const PLAYWRIGHT_VERSION: &str = "1.64.3";

// PATCHRIGHT: the Playwright release `PLAYWRIGHT_VERSION` is built on. The
// build does not use it; `cargo xtask sync-protocol-spec` reads it to vendor
// `protocol-spec/` from the matching Playwright tag, since patchright
// publishes no protocol spec of its own. `cargo xtask update-driver` sets both.
#[allow(dead_code)]
const PLAYWRIGHT_UPSTREAM_VERSION: &str = "1.64.1";

// Download + assembly logic shared with the cli binary (`src/bin/
// patchright_rs.rs`); pulls in the pure URL/platform mapping from
// `driver_urls.rs`, which the lib test suite unit-tests.
include!("src/build_support/driver_assembly.rs");

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/build_support/driver_assembly.rs");
    println!("cargo:rerun-if-changed=src/build_support/driver_urls.rs");
    println!("cargo:rerun-if-env-changed=PLAYWRIGHT_DRIVER_CACHE_DIR");
    println!("cargo:rerun-if-env-changed=PLAYWRIGHT_SKIP_DRIVER_DOWNLOAD");

    let platform = detect_platform();

    // `include_bytes!` in the lib needs a file on every build path (skipped,
    // failed, feature off): an empty one means nothing is embedded. A build
    // with a driver points the variable at the real archive instead.
    let empty_archive = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by Cargo"))
        .join("patchright-driver-none.tar.gz");
    if let Err(e) = std::fs::write(&empty_archive, []) {
        println!(
            "cargo:warning=Could not write {}: {e}",
            empty_archive.display()
        );
    }
    println!(
        "cargo:rustc-env=PATCHRIGHT_EMBEDDED_DRIVER={}",
        empty_archive.display()
    );

    // Skip the download on docs.rs (no network) and for compile-only jobs
    // (e.g. MSRV `cargo check`, mutation testing) that never launch a browser.
    if env::var_os("DOCS_RS").is_some() || env::var_os("PLAYWRIGHT_SKIP_DRIVER_DOWNLOAD").is_some()
    {
        set_absent_env_vars(platform, "skipped");
        return;
    }

    let versioned = driver_dir_name(PLAYWRIGHT_VERSION, platform);

    // The source is recorded so the runtime can say what became of the driver
    // when a launch finds none, and so tests assert a layout only where it
    // applies.
    let (driver_dir, source) = match env::var_os("PLAYWRIGHT_DRIVER_CACHE_DIR") {
        Some(dir) => (PathBuf::from(dir).join(&versioned), "cache_dir"),
        None => match dirs::cache_dir() {
            Some(root) => (
                cached_driver_dir(&root, PLAYWRIGHT_VERSION, platform),
                "user_cache",
            ),
            None => {
                let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by Cargo"));
                (
                    out_dir.join("patchright-driver").join(&versioned),
                    "out_dir",
                )
            }
        },
    };

    let node_exe = driver_dir.join(node_exe_name(platform));
    let cli_js = driver_dir.join("package").join("cli.js");

    // Old driver versions linger beside the current one; a bump of
    // PLAYWRIGHT_VERSION changes build.rs and reruns this, so the new version
    // is always written, but prior versions are not garbage-collected.
    if node_exe.exists() && cli_js.exists() {
        set_present_env_vars(&driver_dir, platform, source);
        watch(&[&node_exe, &cli_js]);
        embed(&driver_dir);
        return;
    }

    println!(
        "cargo:warning=Assembling Patchright driver {PLAYWRIGHT_VERSION} (Node {NODE_VERSION}) for {platform}..."
    );

    match assemble_driver(&driver_dir, PLAYWRIGHT_VERSION, platform) {
        Ok(()) => {
            println!(
                "cargo:warning=Patchright driver assembled at {}",
                driver_dir.display()
            );
            predate_invocation(&[&node_exe, &cli_js]);
            set_present_env_vars(&driver_dir, platform, source);
            watch(&[&node_exe, &cli_js]);
            embed(&driver_dir);
        }
        Err(AssemblyError::Pin(message)) => {
            println!("cargo::error=Playwright driver not assembled: {message}");
            println!(
                "cargo::error=If you supply your own driver (PLAYWRIGHT_DRIVER_PATH, or playwright from npm), set PLAYWRIGHT_SKIP_DRIVER_DOWNLOAD=1 to skip this download."
            );
        }
        Err(AssemblyError::Other(e)) => {
            // Compile anyway (same shape as the skip path): the runtime
            // resolution chain can still find a driver via PLAYWRIGHT_DRIVER_PATH
            // or an npm-installed playwright, and a build without one fails at
            // launch with ServerNotFound instead of a cryptic missing-env-var
            // compile error in downstream crates. Nothing is watched on this
            // path, so the failure is sticky until a knob changes or
            // `cargo clean -p playwright-rs`; watching files that do not exist
            // would rerun this script, and retry the download, on every cargo
            // invocation.
            println!("cargo:warning=Failed to assemble Playwright driver: {e}");
            println!(
                "cargo:warning=Set PLAYWRIGHT_DRIVER_PATH to a driver directory, or install one via npm."
            );
            set_absent_env_vars(platform, "failed");
        }
    }
}

/// Watch the two files the runtime needs. The driver lives outside the
/// target directory, where nothing else would notice it going; cargo treats
/// a missing watched path as changed, so a cleaned cache reruns this script
/// and reassembles on the next build. Only called once the files exist.
fn watch(files: &[&Path]) {
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
}

/// Give freshly assembled files the mtime of cargo's invocation stamp.
///
/// Cargo decides whether a watched file changed by comparing its mtime to
/// `invoked.timestamp`, which it writes beside `OUT_DIR` before this script
/// starts. A file written during the run is newer than that stamp, so
/// without this the very next build reruns the script and recompiles the
/// crate and everything above it once more. The tar path preserves the
/// archive's old mtimes, but the zip path (Windows) writes fresh files, so
/// both are set. The stamp is a cargo internal: if it is not where expected,
/// nothing is changed and the cost is that one extra rebuild.
fn predate_invocation(files: &[&Path]) {
    let Ok(out_dir) = env::var("OUT_DIR") else {
        return;
    };
    let stamp = Path::new(&out_dir)
        .parent()
        .map(|dir| dir.join("invoked.timestamp"));
    let Some(mtime) = stamp
        .and_then(|stamp| std::fs::metadata(stamp).ok())
        .and_then(|meta| meta.modified().ok())
    else {
        return;
    };
    for file in files {
        if let Ok(handle) = std::fs::File::options().write(true).open(file) {
            let _ = handle.set_modified(mtime);
        }
    }
}

/// Point the lib's `include_bytes!` at a packed copy of `driver_dir`, when
/// the `bundled-driver` feature is on.
///
/// The archive sits beside the driver, as `<driver_dir>.tar.gz`, so it is
/// packed once per machine like the driver itself: packing ~110 MB runs
/// unoptimized in a build script and takes most of a minute. It is watched,
/// so a deleted archive is repacked. A packing failure warns and leaves the
/// empty archive in place; the build machine still finds its driver on disk.
fn embed(driver_dir: &Path) {
    if env::var_os("CARGO_FEATURE_BUNDLED_DRIVER").is_none() {
        return;
    }
    let Some(name) = driver_dir.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let archive = driver_dir.with_file_name(format!("{name}.tar.gz"));
    if !archive.is_file() {
        remove_stale_partials(&archive);
        if let Err(e) = pack_driver(driver_dir, &archive) {
            println!("cargo:warning=Could not embed the Patchright driver: {e}");
            return;
        }
        predate_invocation(&[&archive]);
    }
    println!(
        "cargo:rustc-env=PATCHRIGHT_EMBEDDED_DRIVER={}",
        archive.display()
    );
    watch(&[&archive]);
}

/// Delete partial archives a killed build left behind (an IDE restarting
/// its `cargo check` mid-pack is the usual one). Only those untouched for
/// 15 minutes, so a concurrent build's file in progress is left alone.
fn remove_stale_partials(archive: &Path) {
    let (Some(dir), Some(name)) = (
        archive.parent(),
        archive.file_stem().and_then(|n| n.to_str()),
    ) else {
        return;
    };
    let prefix = format!("{name}.partial-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(15 * 60));
        if stale && entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Pack `driver_dir`'s contents, relative to it, into a gzipped tar. Tar
/// keeps the Unix mode bits, so `node` unpacks executable.
fn pack_driver(driver_dir: &Path, archive: &Path) -> std::io::Result<()> {
    // Written beside the target and renamed in, so a killed build never
    // leaves a truncated archive that later builds would embed.
    let tmp = archive.with_extension(format!("partial-{}", std::process::id()));
    let packed = (|| {
        let file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        tar.follow_symlinks(true);
        tar.append_dir_all(".", driver_dir)?;
        let mut file = tar.into_inner()?.finish()?;
        std::io::Write::flush(&mut file)
    })();
    // A concurrent build that renamed its archive in first wins; renaming
    // onto an existing file fails on Windows, and its copy is as good.
    let result = packed.and_then(|()| match std::fs::rename(&tmp, archive) {
        Err(_) if archive.is_file() => Ok(()),
        other => other,
    });
    let _ = std::fs::remove_file(&tmp);
    result
}

/// The Playwright platform identifier of the *target*: the embedded driver
/// runs where the binary runs, which differs from the host when
/// cross-compiling.
fn detect_platform() -> &'static str {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| env::consts::OS.to_string());
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| env::consts::ARCH.to_string());
    let (os, arch) = (os.as_str(), arch.as_str());
    playwright_platform(os, arch).unwrap_or_else(|| {
        println!("cargo:warning=Unsupported platform: {os} {arch}");
        println!("cargo:warning=Defaulting to linux platform");
        "linux"
    })
}

/// Env vars for builds that have no driver on disk (skipped or failed
/// download): the crate still compiles, and the runtime falls back through
/// its resolution chain. `reason` is what the runtime reports if that chain
/// also comes up empty.
fn set_absent_env_vars(platform: &str, reason: &str) {
    println!("cargo:rustc-env=PLAYWRIGHT_DRIVER_DIR=");
    println!("cargo:rustc-env=PLAYWRIGHT_DRIVER_VERSION={PLAYWRIGHT_VERSION}");
    println!("cargo:rustc-env=PLAYWRIGHT_DRIVER_PLATFORM={platform}");
    println!("cargo:rustc-env=PLAYWRIGHT_DRIVER_DIR_SOURCE={reason}");
}

/// Env vars for a build with a driver on disk. `source` names which default
/// or knob chose the directory.
fn set_present_env_vars(driver_dir: &Path, platform: &str, source: &str) {
    println!(
        "cargo:rustc-env=PLAYWRIGHT_DRIVER_DIR={}",
        driver_dir.display()
    );
    println!("cargo:rustc-env=PLAYWRIGHT_DRIVER_VERSION={PLAYWRIGHT_VERSION}");
    println!("cargo:rustc-env=PLAYWRIGHT_DRIVER_PLATFORM={platform}");
    println!("cargo:rustc-env=PLAYWRIGHT_DRIVER_DIR_SOURCE={source}");
    // `links` metadata: a dependent's build script reads these as
    // DEP_PATCHRIGHT_DRIVER_DIR / _VERSION, which is how a Tauri app finds
    // the driver to copy into its bundle resources.
    println!("cargo::metadata=DIR={}", driver_dir.display());
    println!("cargo::metadata=VERSION={PLAYWRIGHT_VERSION}");
    // Deliberately not PLAYWRIGHT_NODE_EXE / PLAYWRIGHT_CLI_JS: those names are
    // the user's runtime overrides, and cargo injects every rustc-env value
    // into the environment of this crate's own test and bin processes, so
    // emitting them here would make the bundled paths look like overrides.
    // The runtime derives both files from PLAYWRIGHT_DRIVER_DIR instead.
}
