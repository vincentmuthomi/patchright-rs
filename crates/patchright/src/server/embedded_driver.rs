//! The driver the `bundled-driver` feature compiles into the library.
//!
//! build.rs packs the assembled driver directory into a `.tar.gz` in
//! `OUT_DIR`, and this module embeds it with `include_bytes!`. A binary built
//! with the crate then carries its own driver: on a machine that never built
//! it, the first launch unpacks the archive into the user cache, at the same
//! path build.rs and `patchright-rs install` use, and every later launch
//! finds it there without unpacking again.
//!
//! The archive is empty when the feature is off, or when the build had no
//! driver to pack (download skipped or failed); [`extract`] then returns
//! `None` and resolution falls through to the npm fallbacks.

#[cfg(feature = "bundled-driver")]
use crate::Error;
use crate::Result;
#[cfg(feature = "bundled-driver")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "bundled-driver")]
static ARCHIVE: &[u8] = include_bytes!(env!("PATCHRIGHT_EMBEDDED_DRIVER"));

/// Unpack the embedded driver if it is not already on disk, and return its
/// `(node, cli.js)` paths. `None` when nothing is embedded.
///
/// # Errors
///
/// [`Error::LaunchFailed`] when there is an embedded driver but it could be
/// unpacked neither into the user cache nor into the temp directory.
pub(crate) fn extract() -> Result<Option<(PathBuf, PathBuf)>> {
    #[cfg(feature = "bundled-driver")]
    {
        if ARCHIVE.is_empty() {
            return Ok(None);
        }
        let (Some(version), Some(platform)) = (
            option_env!("PLAYWRIGHT_DRIVER_VERSION"),
            option_env!("PLAYWRIGHT_DRIVER_PLATFORM"),
        ) else {
            return Ok(None);
        };

        // The user cache is where a later launch and the other lookups look;
        // the temp dir is for a process with no resolvable home.
        let roots = dirs::cache_dir()
            .into_iter()
            .chain(std::iter::once(std::env::temp_dir()));
        let mut failures = Vec::new();
        for root in roots {
            let dir = crate::driver_urls::cached_driver_dir(&root, version, platform);
            match extract_into(ARCHIVE, &dir, platform) {
                Ok(paths) => return Ok(Some(paths)),
                Err(e) => failures.push(format!("{}: {e}", dir.display())),
            }
        }
        Err(Error::LaunchFailed(format!(
            "could not unpack the embedded Patchright driver: {}",
            failures.join("; ")
        )))
    }
    #[cfg(not(feature = "bundled-driver"))]
    {
        Ok(None)
    }
}

/// Unpack `archive` into `driver_dir` unless a complete driver is already
/// there.
///
/// Unpacks into a sibling temp directory and renames it into place, so a
/// killed process never leaves a half-written driver that a later launch
/// mistakes for a whole one, and of two processes unpacking at once the
/// second finds the first's result. A lock serializes threads of this
/// process, which would otherwise unpack the same 100 MB twice.
#[cfg(feature = "bundled-driver")]
fn extract_into(
    archive: &[u8],
    driver_dir: &Path,
    platform: &str,
) -> std::io::Result<(PathBuf, PathBuf)> {
    static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _guard = LOCK.lock();

    let node_exe = driver_dir.join(crate::driver_urls::node_exe_name(platform));
    let cli_js = driver_dir.join("package").join("cli.js");
    let complete = || node_exe.is_file() && cli_js.is_file();
    if complete() {
        return Ok((node_exe, cli_js));
    }

    let parent = driver_dir
        .parent()
        .ok_or_else(|| std::io::Error::other("driver dir has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let name = driver_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("driver");
    let tmp = parent.join(format!(".{name}.unpack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);

    let unpacked = (|| {
        std::fs::create_dir_all(&tmp)?;
        let gz = flate2::read::GzDecoder::new(archive);
        // `unpack` refuses entries that would land outside `tmp`.
        tar::Archive::new(gz).unpack(&tmp)?;
        let node = tmp.join(crate::driver_urls::node_exe_name(platform));
        if !node.is_file() || !tmp.join("package").join("cli.js").is_file() {
            return Err(std::io::Error::other(
                "the embedded archive does not hold node and package/cli.js",
            ));
        }
        Ok(())
    })();
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    // Another process may have finished first; its copy is as good. A
    // directory that exists but is incomplete (a file deleted by hand) is
    // replaced, since nothing can be running from it.
    if complete() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Ok((node_exe, cli_js));
    }
    if driver_dir.exists() {
        let _ = std::fs::remove_dir_all(driver_dir);
    }
    match std::fs::rename(&tmp, driver_dir) {
        Ok(()) => Ok((node_exe, cli_js)),
        Err(_) if complete() => {
            let _ = std::fs::remove_dir_all(&tmp);
            Ok((node_exe, cli_js))
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

#[cfg(all(test, feature = "bundled-driver"))]
mod tests {
    use super::*;

    /// A tiny driver-shaped archive, packed the way build.rs packs one.
    fn fake_archive(platform: &str) -> Vec<u8> {
        let src = tempfile::tempdir().unwrap();
        let node = crate::driver_urls::node_exe_name(platform);
        std::fs::write(src.path().join(node), b"node").unwrap();
        std::fs::create_dir_all(src.path().join("package")).unwrap();
        std::fs::write(src.path().join("package").join("cli.js"), b"cli").unwrap();

        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        tar.append_dir_all(".", src.path()).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn unpacks_into_place_and_reuses_it() {
        let platform = "win32_x64";
        let archive = fake_archive(platform);
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a").join("driver");

        let (node, cli) = extract_into(&archive, &dir, platform).unwrap();
        assert_eq!(std::fs::read(&node).unwrap(), b"node");
        assert_eq!(std::fs::read(&cli).unwrap(), b"cli");

        // A second call finds the first's driver and leaves no temp dirs.
        std::fs::write(&node, b"kept").unwrap();
        extract_into(&archive, &dir, platform).unwrap();
        assert_eq!(std::fs::read(&node).unwrap(), b"kept");
        let leftovers: Vec<_> = std::fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("driver")]);
    }

    #[test]
    fn replaces_an_incomplete_directory() {
        let platform = "linux";
        let archive = fake_archive(platform);
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("driver");
        std::fs::create_dir_all(dir.join("package")).unwrap();
        std::fs::write(dir.join("package").join("cli.js"), b"stale").unwrap();

        let (node, cli) = extract_into(&archive, &dir, platform).unwrap();
        assert!(node.is_file());
        assert_eq!(std::fs::read(&cli).unwrap(), b"cli");
    }

    #[test]
    fn rejects_an_archive_without_a_driver() {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let archive = tar::Builder::new(gz)
            .into_inner()
            .unwrap()
            .finish()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("driver");

        assert!(extract_into(&archive, &dir, "linux").is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn this_build_embeds_a_driver_when_one_was_assembled() {
        let assembled = option_env!("PLAYWRIGHT_DRIVER_DIR").is_some_and(|d| !d.is_empty());
        assert_eq!(!ARCHIVE.is_empty(), assembled);
    }
}
