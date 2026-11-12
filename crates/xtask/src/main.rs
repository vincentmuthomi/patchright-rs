//! Build tooling for the workspace, run as `cargo xtask <command>` (the
//! alias lives in `.cargo/config.toml`). Not published.
//!
//! - `sync-protocol-spec` vendors the protocol spec of the Playwright
//!   release the pinned driver is built on into `protocol-spec/`, so a
//!   driver bump shows its wire-contract delta as a diff. `--check`
//!   verifies the vendored copy offline.
//!
//! - `update-driver` moves the pin to the latest stable `patchright-core`
//!   (or a named one): the version, its npm integrity, the Playwright
//!   release it is built on, and the vendored spec, in one step.
//!
//! Ported from playwright-rust's xtask. PATCHRIGHT: the driver is
//! `patchright-core`, which numbers its own patch releases and publishes no
//! protocol spec, so the spec is fetched at the Playwright tag named by
//! `PLAYWRIGHT_UPSTREAM_VERSION` rather than at `PLAYWRIGHT_VERSION`.

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "xtask", about = "Workspace build tooling")]
enum Cmd {
    /// Vendor the protocol spec of the pinned driver into `protocol-spec/`,
    /// or verify the vendored copy with `--check` (offline).
    SyncProtocolSpec {
        /// Verify instead of fetching: fail if the vendored spec is not the
        /// pinned driver's, or does not match `protocol-spec/MANIFEST`.
        #[arg(long)]
        check: bool,
    },
    /// Move the pinned driver to the latest stable `patchright-core` on npm
    /// and re-vendor the protocol spec to match. Nothing is committed: the
    /// result is a working-tree diff to review.
    UpdateDriver {
        /// Pin this `patchright-core` version instead of the latest.
        #[arg(long)]
        version: Option<String>,
        /// The Playwright release that version is built on, when the one
        /// read from patchright's releases is not the one wanted.
        #[arg(long)]
        playwright: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cmd::parse() {
        Cmd::SyncProtocolSpec { check } => sync_protocol_spec(check),
        Cmd::UpdateDriver {
            version,
            playwright,
        } => update_driver(version, playwright),
    }
}

/// Where the versions are pinned.
const BUILD_SCRIPT: &str = "crates/patchright/build.rs";
/// Where the driver tarball's npm integrity is pinned.
const DRIVER_URLS: &str = "crates/patchright/src/build_support/driver_urls.rs";
/// A test that names the pinned driver version.
const VERSION_TEST: &str = "crates/patchright/tests/playwright_version_test.rs";
const DRIVER_MARKER: &str = "const PLAYWRIGHT_VERSION: &str = ";
const UPSTREAM_MARKER: &str = "const PLAYWRIGHT_UPSTREAM_VERSION: &str = ";
const INTEGRITY_MARKER: &str = "const NPM_CORE_INTEGRITY: (&str, &str) = (";
const VERSION_TEST_MARKER: &str = "assert_eq!(PLAYWRIGHT_VERSION, ";
/// Where the pinned driver's protocol spec is vendored.
const SPEC_DIR: &str = "protocol-spec";
/// Records which Playwright release the vendored spec was taken from.
const SPEC_STAMP: &str = "protocol-spec/DRIVER_VERSION";
/// Digest of every vendored file, so the offline check sees an edited or
/// half-written tree rather than a plausible-looking one.
const SPEC_MANIFEST: &str = "protocol-spec/MANIFEST";

/// The Playwright release whose spec describes the pinned driver: the
/// `PLAYWRIGHT_UPSTREAM_VERSION` constant in the build script, checked
/// against the `patchright-core` version beside it.
fn read_spec_version(root: &Path) -> Result<String> {
    let path = root.join(BUILD_SCRIPT);
    let driver = read_version_const(&path, DRIVER_MARKER)?;
    let upstream = read_version_const(&path, UPSTREAM_MARKER)?;
    check_same_minor(&driver, &upstream)?;
    Ok(upstream)
}

/// Patchright follows Playwright's major.minor and numbers only its own
/// patch releases, so a differing minor means one constant was bumped and
/// the other forgotten.
fn check_same_minor(driver: &str, upstream: &str) -> Result<()> {
    if minor_of(driver) != minor_of(upstream) {
        bail!(
            "{BUILD_SCRIPT} pins patchright-core {driver} but names Playwright {upstream} as \
             the release it is built on. Set PLAYWRIGHT_UPSTREAM_VERSION to the Playwright \
             release patchright-core {driver} is built on."
        );
    }
    Ok(())
}

/// `1.64` of `1.64.3`.
fn minor_of(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

fn read_version_const(path: &Path, marker: &str) -> Result<String> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let (start, end) =
        string_literal(&content, marker, 0).with_context(|| format!("in {}", path.display()))?;
    Ok(content[start..end].to_string())
}

/// The byte range of the contents of the `index`th string literal after
/// `marker`. The pins are plain literals with no escapes.
fn string_literal(content: &str, marker: &str, index: usize) -> Result<(usize, usize)> {
    let mut at = content
        .find(marker)
        .with_context(|| format!("`{marker}` not found"))?
        + marker.len();
    for nth in 0..=index {
        let start = content[at..]
            .find('"')
            .with_context(|| format!("no string literal {nth} after `{marker}`"))?
            + at
            + 1;
        let end = content[start..]
            .find('"')
            .with_context(|| format!("unterminated string literal after `{marker}`"))?
            + start;
        if nth == index {
            return Ok((start, end));
        }
        at = end + 1;
    }
    unreachable!("the loop returns at `index`")
}

/// `content` with the `index`th string literal after `marker` set to `value`.
fn set_string_literal(content: &str, marker: &str, index: usize, value: &str) -> Result<String> {
    let (start, end) = string_literal(content, marker, index)?;
    Ok(format!("{}{value}{}", &content[..start], &content[end..]))
}

/// `content` with the text between `before` and the next `after` set to
/// `value`, or `None` when either is absent.
fn replace_between(content: &str, before: &str, after: &str, value: &str) -> Option<String> {
    let start = content.find(before)? + before.len();
    let end = content[start..].find(after)? + start;
    Some(format!("{}{value}{}", &content[..start], &content[end..]))
}

/// `MAJOR.MINOR.PATCH` as numbers. A pre-release or anything else is an
/// error: only stable releases are pinned.
fn parse_version(version: &str) -> Result<(u32, u32, u32)> {
    let parts: Vec<u32> = version
        .split('.')
        .map(|part| part.parse::<u32>())
        .collect::<Result<_, _>>()
        .with_context(|| format!("`{version}` is not a stable MAJOR.MINOR.PATCH version"))?;
    match parts[..] {
        [major, minor, patch] => Ok((major, minor, patch)),
        _ => bail!("`{version}` is not a stable MAJOR.MINOR.PATCH version"),
    }
}

/// One file's rewritten contents, held until every rewrite has succeeded.
fn rewrite(
    root: &Path,
    path: &str,
    apply: impl Fn(&str) -> Result<String>,
) -> Result<(PathBuf, String)> {
    let full = root.join(path);
    let content = std::fs::read_to_string(&full).with_context(|| format!("read {path}"))?;
    let updated = apply(&content).with_context(|| format!("update {path}"))?;
    Ok((full, updated))
}

/// Move the pin to a `patchright-core` release. Everything is fetched and
/// every rewrite computed before the first file is written, so a failure
/// leaves the tree as it was.
fn update_driver(version: Option<String>, playwright: Option<String>) -> Result<()> {
    let root = workspace_root();
    let build_script = root.join(BUILD_SCRIPT);
    let current = read_version_const(&build_script, DRIVER_MARKER)?;
    let current_upstream = read_version_const(&build_script, UPSTREAM_MARKER)?;

    let explicit = version.is_some();
    let (new, integrity) = fetch_npm_release(version.as_deref().unwrap_or("latest"))?;
    if !explicit && parse_version(&new)? < parse_version(&current)? {
        bail!(
            "update-driver: npm's latest patchright-core is {new}, older than the pinned \
             {current}. Pass `--version {new}` to downgrade on purpose."
        );
    }
    if new == current && playwright.is_none() {
        println!("update-driver: already on patchright-core {current}, nothing to update.");
        return check_protocol_spec(&root, &root.join(SPEC_DIR), &current_upstream);
    }

    let upstream = match playwright {
        Some(upstream) => upstream,
        None => fetch_upstream_version(&new)?,
    };
    parse_version(&upstream)?;
    check_same_minor(&new, &upstream)?;
    let files = fetch_spec_files(&upstream)?;

    let mut edits = vec![
        rewrite(&root, BUILD_SCRIPT, |content| {
            let content = set_string_literal(content, DRIVER_MARKER, 0, &new)?;
            set_string_literal(&content, UPSTREAM_MARKER, 0, &upstream)
        })?,
        rewrite(&root, DRIVER_URLS, |content| {
            let content = set_string_literal(content, INTEGRITY_MARKER, 0, &new)?;
            set_string_literal(&content, INTEGRITY_MARKER, 1, &integrity)
        })?,
        rewrite(&root, VERSION_TEST, |content| {
            set_string_literal(content, VERSION_TEST_MARKER, 0, &new)
        })?,
    ];
    // The README is prose; a reworded line is not worth failing the bump for.
    let readme = rewrite(&root, "README.md", |content| {
        let content = replace_between(content, "`patchright-core` **", "**", &new)
            .context("no driver version in the Versions table")?;
        replace_between(&content, "** (Playwright ", " ", &upstream)
            .context("no Playwright version in the Versions table")
    });
    let readme_updated = readme.is_ok();
    edits.extend(readme);

    for (path, content) in &edits {
        std::fs::write(path, content).with_context(|| format!("write {}", path.display()))?;
    }
    write_spec(&root, &upstream, &files)?;

    println!(
        "update-driver: patchright-core {current} -> {new}, built on Playwright {upstream} \
         (was {current_upstream}). Nothing is committed. Next:"
    );
    println!("  git diff {SPEC_DIR}");
    println!("      the wire-contract change; a removed or renamed parameter is dropped silently");
    println!("  cargo test -p patchright-rs --lib");
    println!("      what the crate sends, checked against the new spec");
    println!("  cargo run -p patchright-rs --features cli --bin patchright-rs -- install chromium");
    println!("  cargo test -p patchright-rs --test patchright");
    if readme_updated {
        println!("README.md: check the rest of the Versions section (Chromium, client notes).");
    } else {
        println!("README.md was not updated: its Versions table no longer has the expected line.");
    }
    Ok(())
}

/// The version and `dist.integrity` npm records for `patchright-core@tag`,
/// where `tag` is a version or a dist-tag such as `latest`.
fn fetch_npm_release(tag: &str) -> Result<(String, String)> {
    let url = format!("https://registry.npmjs.org/patchright-core/{tag}");
    let release: serde_json::Value =
        serde_json::from_str(&fetch(&url)?).with_context(|| format!("parse {url}"))?;
    let version = release["version"]
        .as_str()
        .with_context(|| format!("no version in {url}"))?;
    parse_version(version)?;
    let integrity = release["dist"]["integrity"]
        .as_str()
        .filter(|integrity| integrity.starts_with("sha512-"))
        .with_context(|| format!("no sha512 dist.integrity in {url}"))?;
    Ok((version.to_string(), integrity.to_string()))
}

/// The Playwright release a `patchright-core` version is built on.
/// Patchright tags its GitHub releases with the Playwright version they
/// follow, so this is the newest such tag with the driver's major.minor.
fn fetch_upstream_version(driver: &str) -> Result<String> {
    let url = "https://api.github.com/repos/Kaliiiiiiiiii-Vinyzu/patchright/releases?per_page=100";
    let releases: serde_json::Value =
        serde_json::from_str(&fetch(url)?).context("parse patchright's release list")?;
    let tags: Vec<&str> = releases
        .as_array()
        .context("patchright's release list is not a JSON array")?
        .iter()
        .filter(|release| release["prerelease"].as_bool() != Some(true))
        .filter_map(|release| release["tag_name"].as_str())
        .collect();
    newest_with_same_minor(driver, &tags).with_context(|| {
        format!(
            "update-driver: patchright has no release tagged for Playwright {}.x; pass \
             `--playwright <version>` to name the release patchright-core {driver} is built on",
            minor_of(driver)
        )
    })
}

/// The highest stable `vMAJOR.MINOR.PATCH` among `tags` that shares
/// `driver`'s major.minor, without the `v`.
fn newest_with_same_minor(driver: &str, tags: &[&str]) -> Option<String> {
    tags.iter()
        .map(|tag| tag.trim_start_matches('v'))
        .filter(|version| minor_of(version) == minor_of(driver))
        .filter_map(|version| parse_version(version).ok().map(|key| (key, version)))
        .max()
        .map(|(_, version)| version.to_string())
}

/// Vendor (or verify) the protocol spec of the pinned driver.
fn sync_protocol_spec(check: bool) -> Result<()> {
    let root = workspace_root();
    let expected = read_spec_version(&root)?;
    let dir = root.join(SPEC_DIR);

    if check {
        return check_protocol_spec(&root, &dir, &expected);
    }

    // Fetch everything before touching the tree: a mid-flight failure then
    // leaves the vendored copy as it was, rather than a truncated spec the
    // stamp still vouches for.
    let files = fetch_spec_files(&expected)?;

    write_spec(&root, &expected, &files)?;

    println!(
        "sync-protocol-spec: vendored {} spec file(s) from Playwright {expected}. \
         Review `git diff {SPEC_DIR}` before committing: a removed or renamed parameter \
         is dropped silently by the driver.",
        files.len()
    );
    Ok(())
}

/// Replace the vendored spec with `files`, fetched from Playwright `version`.
fn write_spec(root: &Path, version: &str, files: &[(String, Vec<u8>)]) -> Result<()> {
    let dir = root.join(SPEC_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    for stale in spec_files(&dir)? {
        std::fs::remove_file(&stale).with_context(|| format!("remove {}", stale.display()))?;
    }
    let mut manifest = String::new();
    for (name, content) in files {
        std::fs::write(dir.join(name), content)
            .with_context(|| format!("write {}", dir.join(name).display()))?;
        manifest.push_str(&format!("{}  {name}\n", digest(content)));
    }
    std::fs::write(root.join(SPEC_MANIFEST), manifest)
        .with_context(|| format!("write {SPEC_MANIFEST}"))?;
    std::fs::write(root.join(SPEC_STAMP), format!("{version}\n"))
        .with_context(|| format!("write {SPEC_STAMP}"))?;
    Ok(())
}

/// Verify the vendored spec offline: the right release, every file the
/// manifest lists, unmodified, and nothing extra.
fn check_protocol_spec(root: &Path, dir: &Path, expected: &str) -> Result<()> {
    let stamp = root.join(SPEC_STAMP);
    let found = std::fs::read_to_string(&stamp)
        .with_context(|| {
            format!(
                "read {}; run `cargo xtask sync-protocol-spec` to vendor the spec",
                stamp.display()
            )
        })?
        .trim()
        .to_string();
    if found != expected {
        bail!(
            "verify-protocol-spec: the vendored protocol spec is from Playwright {found}, but \
             {BUILD_SCRIPT} pins a driver built on {expected}. Run `cargo xtask \
             sync-protocol-spec` and review the diff: a renamed parameter is dropped by the \
             driver, not rejected."
        );
    }

    let manifest = std::fs::read_to_string(root.join(SPEC_MANIFEST))
        .with_context(|| format!("read {SPEC_MANIFEST}"))?;
    let vendored: Vec<(String, Option<Vec<u8>>)> = spec_files(dir)?
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            (name, std::fs::read(&path).ok())
        })
        .collect();
    let (listed, problems) = spec_problems(&manifest, &vendored)?;

    if !problems.is_empty() {
        bail!(
            "verify-protocol-spec: the vendored spec does not match {SPEC_MANIFEST}:\n  {}\n\n\
             It is vendored verbatim from upstream; re-run `cargo xtask sync-protocol-spec`.",
            problems.join("\n  ")
        );
    }

    println!(
        "verify-protocol-spec: {listed} spec file(s) vendored from Playwright {expected}, \
         digests match"
    );
    Ok(())
}

/// Compare a `MANIFEST` against the vendored files, returning how many the
/// manifest lists and every mismatch found. Pure, so the comparison is
/// unit-testable without a checkout to mangle.
fn spec_problems(
    manifest: &str,
    vendored: &[(String, Option<Vec<u8>>)],
) -> Result<(usize, Vec<String>)> {
    let mut listed: Vec<&str> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
        let (want, name) = line
            .split_once("  ")
            .with_context(|| format!("malformed {SPEC_MANIFEST} line: {line}"))?;
        listed.push(name);
        match vendored.iter().find(|(have, _)| have == name) {
            None => problems.push(format!("{name}: missing")),
            Some((_, None)) => problems.push(format!("{name}: unreadable")),
            Some((_, Some(content))) if digest(content) != want => {
                problems.push(format!("{name}: contents differ from the vendored copy"));
            }
            Some(_) => {}
        }
    }
    if listed.is_empty() {
        bail!("verify-protocol-spec: {SPEC_MANIFEST} lists no files");
    }
    for (name, _) in vendored {
        if !listed.contains(&name.as_str()) {
            problems.push(format!("{name}: present but not in {SPEC_MANIFEST}"));
        }
    }
    Ok((listed.len(), problems))
}

/// Fetch every spec file Playwright publishes at this release's tag.
fn fetch_spec_files(version: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let listing = format!(
        "https://api.github.com/repos/microsoft/playwright/contents/packages/protocol/spec?ref=v{version}"
    );
    let entries: serde_json::Value =
        serde_json::from_str(&fetch(&listing)?).context("parse the spec directory listing")?;
    let entries = entries
        .as_array()
        .context("the spec directory listing is not a JSON array")?;

    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for entry in entries {
        let name = entry["name"].as_str().unwrap_or_default().to_string();
        let kind = entry["type"].as_str().unwrap_or_default();
        if kind != "file" || !name.ends_with(".yml") {
            skipped.push(format!("{name} ({kind})"));
            continue;
        }
        let url = entry["download_url"]
            .as_str()
            .with_context(|| format!("no download_url for {name}"))?;
        files.push((name, fetch(url)?.into_bytes()));
    }
    if !skipped.is_empty() {
        // Upstream restructured this directory once already; vendoring part
        // of it quietly is how a wire contract goes half-reviewed.
        bail!(
            "sync-protocol-spec: the spec directory holds entries this command does not \
             vendor, so the layout changed upstream and this command needs updating:\n  {}",
            skipped.join("\n  ")
        );
    }
    if files.is_empty() {
        bail!("sync-protocol-spec: the listing held no .yml files");
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Hex digest of a vendored file. SHA-1 is enough to catch an edited or
/// truncated copy, which is all this guards.
fn digest(content: &[u8]) -> String {
    use sha1::Digest as _;
    sha1::Sha1::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The vendored `.yml` files, sorted, or an empty vector when none exist.
fn spec_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "yml"))
        .collect();
    files.sort();
    Ok(files)
}

fn fetch(url: &str) -> Result<String> {
    ureq::get(url)
        // GitHub rejects requests without one.
        .header("User-Agent", "xtask")
        .call()
        .with_context(|| format!("GET {url}"))?
        .body_mut()
        .read_to_string()
        .with_context(|| format!("read the body of {url}"))
}

/// The workspace root: two levels up from this crate's manifest directory
/// (`crates/xtask`).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("xtask manifest dir has two parents")
        .to_path_buf()
}

#[cfg(test)]
mod spec_version_tests {
    use super::*;

    #[test]
    fn a_patch_level_difference_is_accepted() {
        assert!(check_same_minor("1.64.3", "1.64.1").is_ok());
    }

    #[test]
    fn a_minor_level_difference_is_rejected() {
        assert!(check_same_minor("1.65.0", "1.64.1").is_err());
    }

    #[test]
    fn the_build_script_pins_a_consistent_pair() {
        read_spec_version(&workspace_root()).unwrap();
    }
}

#[cfg(test)]
mod update_driver_tests {
    use super::*;

    #[test]
    fn a_literal_after_a_marker_is_replaced_in_place() {
        let source = "// x\nconst V: &str = \"1.0.0\";\nconst W: &str = \"2.0.0\";\n";
        assert_eq!(
            set_string_literal(source, "const V: &str = ", 0, "1.2.3").unwrap(),
            "// x\nconst V: &str = \"1.2.3\";\nconst W: &str = \"2.0.0\";\n"
        );
    }

    #[test]
    fn the_second_literal_of_a_tuple_is_addressable() {
        let source = "const I: (&str, &str) = (\n    \"1.0.0\",\n    \"sha512-old\",\n);\n";
        assert_eq!(
            set_string_literal(source, "const I: (&str, &str) = (", 1, "sha512-new").unwrap(),
            "const I: (&str, &str) = (\n    \"1.0.0\",\n    \"sha512-new\",\n);\n"
        );
    }

    #[test]
    fn a_missing_marker_is_an_error() {
        assert!(set_string_literal("nothing here", "const V: &str = ", 0, "1").is_err());
    }

    #[test]
    fn every_pin_the_command_rewrites_is_where_it_expects() {
        let root = workspace_root();
        for (path, marker, index) in [
            (BUILD_SCRIPT, DRIVER_MARKER, 0),
            (BUILD_SCRIPT, UPSTREAM_MARKER, 0),
            (DRIVER_URLS, INTEGRITY_MARKER, 0),
            (DRIVER_URLS, INTEGRITY_MARKER, 1),
            (VERSION_TEST, VERSION_TEST_MARKER, 0),
        ] {
            let content = std::fs::read_to_string(root.join(path)).unwrap();
            let (start, end) = string_literal(&content, marker, index)
                .unwrap_or_else(|error| panic!("{path}: {error:#}"));
            let literal = &content[start..end];
            assert!(
                parse_version(literal).is_ok() || literal.starts_with("sha512-"),
                "{path}: `{marker}` literal {index} is `{literal}`"
            );
        }
    }

    #[test]
    fn the_readme_versions_line_is_rewritten() {
        let line = "| Driver | `patchright-core` **1.64.3** (Playwright 1.64.1 + patchright) |";
        let line = replace_between(line, "`patchright-core` **", "**", "1.65.2").unwrap();
        let line = replace_between(&line, "** (Playwright ", " ", "1.65.0").unwrap();
        assert_eq!(
            line,
            "| Driver | `patchright-core` **1.65.2** (Playwright 1.65.0 + patchright) |"
        );
    }

    #[test]
    fn the_real_readme_has_the_versions_line() {
        let readme = std::fs::read_to_string(workspace_root().join("README.md")).unwrap();
        let readme = replace_between(&readme, "`patchright-core` **", "**", "0.0.0").unwrap();
        assert!(replace_between(&readme, "** (Playwright ", " ", "0.0.0").is_some());
    }

    #[test]
    fn only_stable_versions_parse() {
        assert_eq!(parse_version("1.64.3").unwrap(), (1, 64, 3));
        assert!(parse_version("1.65.0-beta.1").is_err());
        assert!(parse_version("1.65").is_err());
        assert!(parse_version("latest").is_err());
    }

    #[test]
    fn the_upstream_release_is_the_newest_tag_of_the_same_minor() {
        let tags = [
            "v1.65.0", "v1.64.1", "v1.64.0", "v1.64.10", "v1.63.0", "nightly",
        ];
        assert_eq!(
            newest_with_same_minor("1.64.3", &tags).as_deref(),
            Some("1.64.10")
        );
        assert_eq!(newest_with_same_minor("1.62.0", &tags), None);
    }
}

#[cfg(test)]
mod spec_manifest_tests {
    use super::*;

    fn vendored(files: &[(&str, &str)]) -> Vec<(String, Option<Vec<u8>>)> {
        files
            .iter()
            .map(|(name, body)| (name.to_string(), Some(body.as_bytes().to_vec())))
            .collect()
    }

    fn manifest_for(files: &[(&str, &str)]) -> String {
        files
            .iter()
            .map(|(name, body)| format!("{}  {name}\n", digest(body.as_bytes())))
            .collect()
    }

    #[test]
    fn a_matching_tree_has_no_problems() {
        let files = [
            ("page.yml", "commands:\n"),
            ("frame.yml", "commands:\n  goto:\n"),
        ];
        let (listed, problems) = spec_problems(&manifest_for(&files), &vendored(&files)).unwrap();

        assert_eq!(listed, 2);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn an_edited_file_is_reported() {
        let files = [("page.yml", "commands:\n")];
        let manifest = manifest_for(&files);
        // What a CRLF checkout does to a byte-exact tree.
        let (_, problems) =
            spec_problems(&manifest, &vendored(&[("page.yml", "commands:\r\n")])).unwrap();

        assert_eq!(
            problems,
            ["page.yml: contents differ from the vendored copy"]
        );
    }

    #[test]
    fn a_missing_file_is_reported() {
        let manifest = manifest_for(&[("page.yml", "a"), ("frame.yml", "b")]);
        let (listed, problems) = spec_problems(&manifest, &vendored(&[("page.yml", "a")])).unwrap();

        assert_eq!(listed, 2);
        assert_eq!(problems, ["frame.yml: missing"]);
    }

    #[test]
    fn a_file_the_manifest_does_not_list_is_reported() {
        let manifest = manifest_for(&[("page.yml", "a")]);
        let (_, problems) = spec_problems(
            &manifest,
            &vendored(&[("page.yml", "a"), ("rogue.yml", "x")]),
        )
        .unwrap();

        assert_eq!(
            problems,
            ["rogue.yml: present but not in protocol-spec/MANIFEST"]
        );
    }

    #[test]
    fn an_empty_manifest_is_an_error() {
        assert!(spec_problems("\n", &vendored(&[("page.yml", "a")])).is_err());
    }

    #[test]
    fn a_malformed_manifest_line_is_an_error() {
        assert!(spec_problems("no-two-space-separator\n", &[]).is_err());
    }
}
