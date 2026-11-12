use patchright_rs::protocol::{
    ActionCursor, ChapterOptions, ScreencastStartOptions, ShowActionsOptions, ShowOverlayOptions,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn test_screencast_streams_frames_to_handler() {
    let (_pw, browser, page) = crate::common::setup().await;

    page.set_content(
        "<body style='background:#06f;color:#fff;font-size:48px'>frame</body>",
        None,
    )
    .await
    .expect("Failed to set content");

    let frames = Arc::new(AtomicUsize::new(0));
    let bytes_seen = Arc::new(parking_lot::Mutex::new(0usize));
    let timestamp_seen = Arc::new(parking_lot::Mutex::new(None::<f64>));
    let counter = frames.clone();
    let bytes = bytes_seen.clone();
    let timestamp = timestamp_seen.clone();
    page.screencast().on_frame(move |frame| {
        let counter = counter.clone();
        let bytes = bytes.clone();
        let timestamp = timestamp.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            *bytes.lock() = frame.data.len();
            *timestamp.lock() = frame.timestamp;
            Ok(())
        }
    });

    page.screencast()
        .start(ScreencastStartOptions::default())
        .await
        .expect("screencast start failed");

    let _ = page
        .evaluate_value("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")
        .await;

    let mut waited = std::time::Duration::ZERO;
    while frames.load(Ordering::SeqCst) == 0 && waited < std::time::Duration::from_secs(10) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += std::time::Duration::from_millis(100);
    }

    page.screencast()
        .stop()
        .await
        .expect("screencast stop failed");

    assert!(
        frames.load(Ordering::SeqCst) > 0,
        "expected at least one screencast frame within 10s"
    );
    assert!(
        *bytes_seen.lock() > 0,
        "frame data should be a non-empty JPEG byte buffer"
    );
    assert!(
        timestamp_seen.lock().is_some(),
        "screencast frame should carry a presentation timestamp (Playwright 1.61)"
    );

    browser.close().await.expect("Failed to close browser");
}

#[tokio::test]
async fn test_screencast_keeps_streaming_past_the_unacked_burst() {
    let (_pw, browser, page) = crate::common::setup().await;

    // Keep repainting so the driver always has a new frame to send.
    page.set_content(
        "<body style='font-size:64px'>0</body>\
         <script>\
           let n = 0;\
           function tick() {\
             n++;\
             document.body.textContent = n;\
             document.body.style.background = 'hsl(' + (n % 360) + ',90%,50%)';\
             requestAnimationFrame(tick);\
           }\
           requestAnimationFrame(tick);\
         </script>",
        None,
    )
    .await
    .expect("Failed to set content");

    let frames = Arc::new(AtomicUsize::new(0));
    let counter = frames.clone();
    page.screencast().on_frame(move |_frame| {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    });

    page.screencast()
        .start(ScreencastStartOptions::default())
        .await
        .expect("screencast start failed");

    // Playwright 1.62 made delivery flow-controlled: the driver sends a small
    // burst and then waits for `screencastFrameAck` before sending more.
    // Without the ack this stalls at a handful of frames however long you
    // wait, so a threshold comfortably above that burst is what distinguishes
    // "streaming" from "stalled" -- `> 0` cannot.
    const WELL_PAST_THE_BURST: usize = 12;
    let streamed = crate::common::poll_until(std::time::Duration::from_secs(20), || {
        frames.load(Ordering::SeqCst) >= WELL_PAST_THE_BURST
    })
    .await;

    page.screencast()
        .stop()
        .await
        .expect("screencast stop failed");

    assert!(
        streamed,
        "screencast stalled after {} frames; expected at least {} \
         (is `screencastFrameAck` still being sent?)",
        frames.load(Ordering::SeqCst),
        WELL_PAST_THE_BURST
    );

    browser.close().await.expect("Failed to close browser");
}

#[tokio::test]
async fn test_screencast_records_to_path() {
    let (_pw, browser, page) = crate::common::setup().await;

    page.set_content("<body>recorded</body>", None)
        .await
        .expect("Failed to set content");

    let path = std::env::temp_dir().join(format!(
        "pw-rust-screencast-{}.webm",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&path);

    page.screencast()
        .start(ScreencastStartOptions::default().path(path.clone()))
        .await
        .expect("screencast start with path failed");

    let _ = page
        .evaluate_value("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")
        .await;

    page.screencast()
        .stop()
        .await
        .expect("screencast stop failed");

    let metadata =
        std::fs::metadata(&path).expect("recorded screencast file should exist after stop");
    assert!(
        metadata.len() > 0,
        "recorded screencast should be non-empty"
    );

    let _ = std::fs::remove_file(&path);
    browser.close().await.expect("Failed to close browser");
}

#[tokio::test]
async fn test_screencast_overlay_and_chapter_calls_succeed() {
    let (_pw, browser, page) = crate::common::setup().await;

    page.set_content("<body>overlays</body>", None)
        .await
        .expect("Failed to set content");

    let screencast = page.screencast();
    screencast
        .start(ScreencastStartOptions::default())
        .await
        .expect("start failed");

    screencast
        .show_actions(
            ShowActionsOptions::default()
                .duration(500.0)
                .cursor(ActionCursor::Pointer),
        )
        .await
        .expect("show_actions failed");
    screencast
        .hide_actions()
        .await
        .expect("hide_actions failed");

    screencast
        .show_chapter(
            "Phase 1",
            ChapterOptions::default()
                .description("setting up")
                .duration(500.0),
        )
        .await
        .expect("show_chapter failed");

    let id = screencast
        .show_overlay(
            "<div style='color:white;background:black;padding:8px'>hi</div>",
            ShowOverlayOptions::default().duration(500.0),
        )
        .await
        .expect("show_overlay failed");
    assert!(!id.0.is_empty(), "overlay id should be non-empty");

    screencast
        .set_overlay_visible(false)
        .await
        .expect("set_overlay_visible(false) failed");
    screencast
        .set_overlay_visible(true)
        .await
        .expect("set_overlay_visible(true) failed");

    screencast
        .remove_overlay(id)
        .await
        .expect("remove_overlay failed");

    screencast.stop().await.expect("stop failed");
    browser.close().await.expect("Failed to close browser");
}

#[tokio::test]
async fn a_screencast_records_at_a_frame_rate_and_refuses_a_zero_one() {
    let (_pw, browser, page) = crate::common::setup().await;
    page.set_content("<body>recorded</body>", None)
        .await
        .expect("set_content");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fps.webm");

    let err = page
        .screencast()
        .start(ScreencastStartOptions::default().path(path.clone()).fps(0))
        .await
        .expect_err("a zero frame rate is refused");
    assert!(
        err.to_string()
            .contains(r#""fps" must be a positive number, got 0"#),
        "{err}"
    );

    page.screencast()
        .start(ScreencastStartOptions::default().path(path.clone()).fps(60))
        .await
        .expect("start at 60 fps");
    page.evaluate_value("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")
        .await
        .expect("wait for two frames");
    page.screencast().stop().await.expect("stop");
    assert!(std::fs::metadata(&path).expect("recorded file").len() > 0);

    browser.close().await.expect("close");
}

#[tokio::test]
#[ignore = "patchright: evaluates in an isolated world by default; the test reads page globals (passes under .main_world())"]
async fn show_actions_with_a_style_still_lets_actions_run() {
    let (_pw, browser, page) = crate::common::setup().await;
    page.set_content(
        "<button onclick=\"window.result = 'Clicked'\">Click</button>",
        None,
    )
    .await
    .expect("set_content");
    let screencast = page.screencast();
    screencast
        .start(ScreencastStartOptions::default())
        .await
        .expect("start");
    screencast
        .show_actions(
            ShowActionsOptions::default().style(
                patchright_rs::ShowActionsStyle::default()
                    .point("width: 20px; height: 20px; background: red")
                    .highlight("outline: 2px solid #333")
                    .title("font-size: 16px"),
            ),
        )
        .await
        .expect("show_actions with a style");

    page.locator("button").click(None).await.expect("click");
    let result: String = page
        .evaluate("() => window.result", None::<&()>)
        .await
        .expect("result");
    assert_eq!(result, "Clicked");

    screencast.stop().await.expect("stop");
    browser.close().await.expect("close");
}
