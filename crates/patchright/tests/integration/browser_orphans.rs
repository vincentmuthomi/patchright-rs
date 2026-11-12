//! The driver owns the browser processes. If we kill the driver without
//! letting it tear them down, headed Chrome survives and is reparented to
//! init — a leak the client can never clean up afterwards.
//!
//! Headless Chromium exits on its own when its parent dies, so only a headed
//! launch exercises this. That means the test needs a display: it skips on a
//! Linux box without one rather than reporting a false pass.

#![cfg(unix)]

use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "PW_ORPHAN_CHILD";
const MARKER_ENV: &str = "PW_ORPHAN_MARKER";
const CHILD_TEST: &str = "browser_orphans::child_leaves_a_headed_browser_running";

fn has_display() -> bool {
    !cfg!(target_os = "linux")
        || std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// PIDs of browser processes whose command line carries `marker`.
///
/// Other integration tests launch browsers concurrently, so counting every
/// Playwright-managed chromium would see their processes come and go. The
/// marker is an inert extra switch only our child's browser was launched
/// with; chromium keeps unknown switches on its command line, where ps can
/// see them.
fn marked_browser_pids(marker: &str) -> HashSet<u32> {
    let out = match Command::new("ps").args(["-eo", "pid=,command="]).output() {
        Ok(out) => out,
        Err(_) => return HashSet::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains("ms-playwright") && line.contains(marker))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

fn kill(pids: &HashSet<u32>) {
    for pid in pids {
        // SAFETY: kill(2) with a PID we just read from ps; a stale PID only
        // yields ESRCH, which we ignore.
        unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
    }
}

#[test]
fn dropping_playwright_does_not_orphan_a_headed_browser() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    if !has_display() {
        eprintln!("skipping: headed Chromium needs a display");
        return;
    }

    let marker = format!("--pw-rs-orphan-test={}", std::process::id());

    let status = Command::new(std::env::current_exe().expect("current_exe"))
        .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(MARKER_ENV, &marker)
        .status()
        .expect("spawn child test binary");
    assert!(status.success(), "child failed to launch a browser");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut leaked = HashSet::new();
    while Instant::now() < deadline {
        leaked = marked_browser_pids(&marker);
        if leaked.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    kill(&leaked);
    panic!(
        "{} browser process(es) outlived the client that launched them: {leaked:?}",
        leaked.len()
    );
}

/// Child half. Launches a headed browser and returns without closing it,
/// standing in for a test that panics or times out before its cleanup runs.
///
/// Only meaningful when spawned by the parent, which passes the marker. CI
/// also runs the whole ignored set directly (`--run-ignored ignored-only`);
/// with no marker to tag the browser with, there is nothing to assert, so
/// this returns rather than launching an unattributable browser.
#[tokio::test]
#[ignore = "spawned by dropping_playwright_does_not_orphan_a_headed_browser"]
async fn child_leaves_a_headed_browser_running() {
    use patchright_rs::api::LaunchOptions;
    use patchright_rs::protocol::Playwright;

    let Ok(marker) = std::env::var(MARKER_ENV) else {
        return;
    };

    let playwright = Playwright::launch().await.expect("launch Playwright");
    let browser = playwright
        .chromium()
        .launch_with_options(LaunchOptions::new().headless(false).args(vec![marker]))
        .await
        .expect("launch headed Chromium");
    browser.new_page().await.expect("new page");
}
