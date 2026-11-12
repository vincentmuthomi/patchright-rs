use patchright_rs::protocol::{BrowserContextOptions, RecordHar, RecordVideo, Viewport};

#[test]
fn record_har_is_applied_by_the_client_not_sent() {
    let options = BrowserContextOptions::builder()
        .record_har(RecordHar::new("/tmp/test.har").mode("minimal"))
        .build();

    let json = serde_json::to_value(options).unwrap();
    assert!(json.get("recordHar").is_none());
}

#[test]
fn test_serialize_record_video() {
    let options = BrowserContextOptions::builder()
        .record_video(RecordVideo::new("/tmp/videos").size(Viewport {
            width: 800,
            height: 600,
        }))
        .build();

    let json = serde_json::to_value(options).unwrap();
    let record_video = json.get("recordVideo").unwrap();

    assert_eq!(record_video["dir"], "/tmp/videos");
    assert_eq!(record_video["size"]["width"], 800);
    assert_eq!(record_video["size"]["height"], 600);
}

#[test]
fn test_serialize_service_workers() {
    let options = BrowserContextOptions::builder()
        .service_workers("block".to_string())
        .build();

    let json = serde_json::to_value(options).unwrap();
    assert_eq!(json["serviceWorkers"], "block");
}

#[test]
fn test_serialize_no_viewport_as_no_default_viewport() {
    let options = BrowserContextOptions::builder().no_viewport(true).build();

    let json = serde_json::to_value(&options).unwrap();

    // Must serialize as "noDefaultViewport" (what the Playwright server expects),
    // not "noViewport" (what camelCase would produce).
    assert_eq!(json["noDefaultViewport"], true);
    assert!(json.get("noViewport").is_none());
}
