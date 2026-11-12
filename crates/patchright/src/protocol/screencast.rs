//! Live screencast frame streaming, optional disk recording, and
//! action / chapter / HTML overlays.
//!
//! Available on every [`Page`] via
//! [`screencast()`](crate::protocol::Page::screencast). Once started,
//! the Playwright server streams JPEG frames as they're rendered,
//! delivered to handlers registered with [`Screencast::on_frame`].
//! Optionally records to disk via the [`Artifact`](crate::protocol::artifact::Artifact)
//! save-on-stop pathway, and can overlay action labels, chapter cards,
//! or arbitrary HTML on the streamed frames.
//!
//! The action / chapter / HTML overlay surfaces are useful for "agent
//! receipts" — an LLM-driven flow can produce annotated video logs of
//! what it did alongside the action log.
//!
//! # Disk recording vs the Video class
//!
//! [`Video`](crate::protocol::Video) and [`Screencast`] cover
//! complementary lifecycles, both backed by the same underlying
//! `Artifact` save mechanism:
//!
//! - **`Video`** — automatic, captures the entire page session from
//!   open to close. Enabled with `BrowserContextOptions::record_video`.
//!   Use when you want a continuous recording over the whole session.
//! - **`Screencast::start({ path })`** — user-initiated, captures only
//!   during the start/stop window, saves to `path` on stop. Use when
//!   you want a recording that brackets a specific phase.
//!
//! # Example
//!
//! ```no_run
//! use patchright_rs::{Playwright, ScreencastStartOptions};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let pw = Playwright::launch().await?;
//!     let browser = pw.chromium().launch().await?;
//!     let page = browser.new_page().await?;
//!     let screencast = page.screencast();
//!
//!     // Stream frames live
//!     screencast.on_frame(|frame| async move {
//!         println!("got {} byte frame", frame.data.len());
//!         Ok(())
//!     });
//!
//!     screencast.start(ScreencastStartOptions::default()
//!         .path(std::path::PathBuf::from("/tmp/run.webm"))).await?;
//!
//!     page.goto("https://example.com", None).await?;
//!     screencast.show_chapter(
//!         "Logged in",
//!         Default::default(),
//!     ).await?;
//!
//!     screencast.stop().await?; // saves /tmp/run.webm
//!     browser.close().await?;
//!     Ok(())
//! }
//! ```
//!
//! See: <https://playwright.dev/docs/api/class-page#page-screencast>

use crate::error::Result;
use crate::protocol::page::Page;
use crate::server::channel_owner::ChannelOwner;
use std::path::PathBuf;

/// A single frame emitted while a screencast is active. Wire format is
/// JPEG; `data` holds the raw bytes ready to write to disk or pass to
/// an image decoder.
///
/// `data` is a [`bytes::Bytes`] handle so the decoded JPEG is allocated
/// exactly once per frame and cloning into each registered handler is
/// a refcount bump rather than a memcpy. `Bytes` implements
/// `Deref<Target = [u8]>`, so existing reads (`frame.data.len()`,
/// `&frame.data[..]`, `tokio::fs::write(path, &frame.data)`) compile
/// unchanged from the previous `Vec<u8>` shape.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ScreencastFrame {
    /// JPEG-encoded frame bytes.
    pub data: bytes::Bytes,
    /// Frame presentation timestamp in seconds.
    /// `None` if the driver did not supply one.
    pub timestamp: Option<f64>,
}

/// Options for [`Screencast::start`].
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct ScreencastStartOptions {
    /// Output frame size. When `None`, Playwright uses the page's
    /// current viewport size.
    pub size: Option<ScreencastSize>,
    /// JPEG quality, `0..=100`. Server default is implementation-defined.
    pub quality: Option<u8>,
    /// When set, the screencast is also recorded to a file at this
    /// path. The file is written on [`Screencast::stop`]. The recording
    /// covers only the active start/stop window — for a continuous
    /// "always-on" recording over the whole page session, use
    /// `BrowserContextOptions::record_video` instead (the `Video`
    /// class).
    pub path: Option<PathBuf>,
    /// Frame rate of the file recorded to [`path`](Self::path); it has no
    /// effect without one. Must be positive. Firefox and WebKit capture at
    /// most 25 frames per second.
    pub fps: Option<u32>,
}

impl ScreencastStartOptions {
    /// Output video size.
    pub fn size(mut self, size: ScreencastSize) -> Self {
        self.size = Some(size);
        self
    }
    /// JPEG quality, `0..=100`.
    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = Some(quality);
        self
    }
    /// Output file path.
    pub fn path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }
    /// Frame rate of the recorded file.
    pub fn fps(mut self, fps: u32) -> Self {
        self.fps = Some(fps);
        self
    }

    /// The `screencastStart` parameters: frames are sent only when a frame
    /// handler is registered, and a file is recorded only when a path is set.
    pub(crate) fn params(&self, send_frames: bool) -> serde_json::Value {
        let mut params = serde_json::json!({});
        if let Some(size) = self.size {
            params["size"] = serde_json::json!({
                "width": size.width,
                "height": size.height,
            });
        }
        if let Some(quality) = self.quality {
            params["quality"] = serde_json::json!(quality);
        }
        if let Some(fps) = self.fps {
            params["fps"] = serde_json::json!(fps);
        }
        params["sendFrames"] = serde_json::json!(send_frames);
        params["record"] = serde_json::json!(self.path.is_some());
        params
    }
}

/// Pixel dimensions for a screencast frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreencastSize {
    pub width: u32,
    pub height: u32,
}

/// Position for the action-label overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionPosition {
    TopLeft,
    Top,
    TopRight,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl serde::Serialize for ActionPosition {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl ActionPosition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ActionPosition::TopLeft => "top-left",
            ActionPosition::Top => "top",
            ActionPosition::TopRight => "top-right",
            ActionPosition::BottomLeft => "bottom-left",
            ActionPosition::Bottom => "bottom",
            ActionPosition::BottomRight => "bottom-right",
        }
    }
}

/// Pointer-cursor decoration for action overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionCursor {
    /// No cursor decoration.
    None,
    /// Draw a pointer cursor at each action point.
    Pointer,
}

impl serde::Serialize for ActionCursor {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl ActionCursor {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ActionCursor::None => "none",
            ActionCursor::Pointer => "pointer",
        }
    }
}

/// CSS for the decorations [`ShowActionsOptions`] draws, each a list of
/// declarations such as `"width: 20px; height: 20px; background: red"`.
///
/// See: <https://playwright.dev/docs/api/class-screencast#screencast-show-actions-option-style>
#[derive(Debug, Default, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct ShowActionsStyle {
    /// The marker drawn at each action point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<String>,
    /// The outline drawn around the action's target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
    /// The action's title label; replaces the deprecated `font_size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl ShowActionsStyle {
    /// CSS for the action-point marker.
    pub fn point(mut self, css: impl Into<String>) -> Self {
        self.point = Some(css.into());
        self
    }
    /// CSS for the target highlight.
    pub fn highlight(mut self, css: impl Into<String>) -> Self {
        self.highlight = Some(css.into());
        self
    }
    /// CSS for the action title.
    pub fn title(mut self, css: impl Into<String>) -> Self {
        self.title = Some(css.into());
        self
    }
}

/// Options for [`Screencast::show_actions`], and for
/// [`RecordVideo::show_actions`](crate::protocol::RecordVideo::show_actions)
/// on a context's recorded video.
#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ShowActionsOptions {
    /// How long each action label stays on screen (milliseconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Where the label appears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<ActionPosition>,
    /// Label font size, pixels.
    #[deprecated(
        since = "0.20.0",
        note = "use `style` with a `title` such as `font-size: 16px`"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<i32>,
    /// Pointer-cursor decoration at action points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<ActionCursor>,
    /// CSS for the marker, the target highlight, and the title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<ShowActionsStyle>,
}

impl ShowActionsOptions {
    /// How long to show each action, in milliseconds.
    pub fn duration(mut self, duration: f64) -> Self {
        self.duration = Some(duration);
        self
    }
    /// Where to render the action labels.
    pub fn position(mut self, position: ActionPosition) -> Self {
        self.position = Some(position);
        self
    }
    /// Label font size in pixels.
    #[deprecated(
        since = "0.20.0",
        note = "use `style` with a `title` such as `font-size: 16px`"
    )]
    pub fn font_size(mut self, font_size: i32) -> Self {
        #[allow(deprecated)]
        {
            self.font_size = Some(font_size);
        }
        self
    }
    /// CSS for the action decorations.
    pub fn style(mut self, style: ShowActionsStyle) -> Self {
        self.style = Some(style);
        self
    }
    /// Pointer-cursor decoration at action points.
    pub fn cursor(mut self, cursor: ActionCursor) -> Self {
        self.cursor = Some(cursor);
        self
    }
}

/// Options for [`Screencast::show_chapter`].
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct ChapterOptions {
    /// Optional second line under the chapter title.
    pub description: Option<String>,
    /// How long the chapter card stays on screen (milliseconds).
    pub duration: Option<f64>,
}

impl ChapterOptions {
    /// Chapter description text.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    /// Chapter duration, in milliseconds.
    pub fn duration(mut self, duration: f64) -> Self {
        self.duration = Some(duration);
        self
    }
}

/// Options for [`Screencast::show_overlay`].
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct ShowOverlayOptions {
    /// How long the overlay stays on screen (milliseconds).
    pub duration: Option<f64>,
}

impl ShowOverlayOptions {
    /// How long to show the overlay, in milliseconds.
    pub fn duration(mut self, duration: f64) -> Self {
        self.duration = Some(duration);
        self
    }
}

/// Identifier for an active HTML overlay; pass to
/// [`Screencast::remove_overlay`] to dismiss the overlay before its
/// duration expires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayId(pub String);

/// Live frame-streaming entry point. Obtained from
/// [`Page::screencast`](crate::protocol::Page::screencast).
#[derive(Clone)]
pub struct Screencast {
    page: Page,
}

impl Screencast {
    pub(crate) fn new(page: Page) -> Self {
        Self { page }
    }

    /// Begin streaming. Frames arrive on handlers registered via
    /// [`on_frame`](Self::on_frame); register them before calling
    /// `start` so no frames are missed.
    ///
    /// If `options.path` is set, the screencast is also recorded to
    /// disk; the file is written when [`stop`](Self::stop) is called.
    #[tracing::instrument(level = "info", skip_all, fields(page_guid = %self.page.guid()))]
    pub async fn start(&self, options: ScreencastStartOptions) -> Result<()> {
        self.page.screencast_start(options).await
    }

    /// Stop the screencast. If `start` was called with a `path`, the
    /// recorded file is written to that path before this call returns.
    #[tracing::instrument(level = "info", skip_all, fields(page_guid = %self.page.guid()))]
    pub async fn stop(&self) -> Result<()> {
        self.page.screencast_stop().await
    }

    /// Register a handler for incoming frames. Multiple handlers may be
    /// registered; they fire in order for each frame.
    pub fn on_frame<F, Fut>(&self, handler: F)
    where
        F: Fn(ScreencastFrame) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.page.screencast_on_frame(handler);
    }

    /// Overlay action labels on the streamed frames as actions occur.
    /// Pair with [`hide_actions`](Self::hide_actions) to stop.
    #[tracing::instrument(level = "debug", skip_all, fields(page_guid = %self.page.guid()))]
    pub async fn show_actions(&self, options: ShowActionsOptions) -> Result<()> {
        self.page.screencast_show_actions(options).await
    }

    /// Stop overlaying action labels. No-op if not currently shown.
    #[tracing::instrument(level = "debug", skip_all, fields(page_guid = %self.page.guid()))]
    pub async fn hide_actions(&self) -> Result<()> {
        self.page.screencast_hide_actions().await
    }

    /// Show a chapter card with the given title (and optional
    /// description). Useful for splitting a session into named phases
    /// for an agent's video log.
    #[tracing::instrument(level = "debug", skip_all, fields(page_guid = %self.page.guid(), title = %title))]
    pub async fn show_chapter(&self, title: &str, options: ChapterOptions) -> Result<()> {
        self.page.screencast_chapter(title, options).await
    }

    /// Render arbitrary HTML as an overlay. Returns an [`OverlayId`]
    /// you can pass to [`remove_overlay`](Self::remove_overlay) to
    /// dismiss it early; otherwise it dismisses itself after
    /// `options.duration` (if set) or stays until removed.
    #[tracing::instrument(level = "debug", skip_all, fields(page_guid = %self.page.guid()))]
    pub async fn show_overlay(&self, html: &str, options: ShowOverlayOptions) -> Result<OverlayId> {
        self.page.screencast_show_overlay(html, options).await
    }

    /// Remove an overlay previously created via
    /// [`show_overlay`](Self::show_overlay). Idempotent.
    #[tracing::instrument(level = "debug", skip_all, fields(page_guid = %self.page.guid()))]
    pub async fn remove_overlay(&self, id: OverlayId) -> Result<()> {
        self.page.screencast_remove_overlay(id).await
    }

    /// Toggle visibility of all currently-shown overlays without
    /// removing them. Useful for hiding overlays during a section the
    /// agent considers "noise" and re-showing them later.
    #[tracing::instrument(level = "debug", skip_all, fields(page_guid = %self.page.guid(), visible))]
    pub async fn set_overlay_visible(&self, visible: bool) -> Result<()> {
        self.page.screencast_set_overlay_visible(visible).await
    }
}

#[cfg(test)]
mod options_tests {
    use super::{
        ActionCursor, ActionPosition, ScreencastSize, ScreencastStartOptions, ShowActionsOptions,
        ShowActionsStyle,
    };
    use crate::protocol_spec::{
        PAGE, command_parameters, keys, mixin_properties, nested_properties,
    };
    use serde_json::json;

    #[test]
    fn show_actions_options_serialize_to_the_driver_spelling() {
        let options = ShowActionsOptions::default()
            .duration(500.0)
            .position(ActionPosition::TopLeft)
            .cursor(ActionCursor::Pointer)
            .style(
                ShowActionsStyle::default()
                    .point("width: 20px")
                    .highlight("outline: 2px solid #333")
                    .title("font-size: 16px"),
            );
        let sent = serde_json::to_value(&options).unwrap();
        assert_eq!(
            sent,
            json!({
                "duration": 500.0,
                "position": "top-left",
                "cursor": "pointer",
                "style": {
                    "point": "width: 20px",
                    "highlight": "outline: 2px solid #333",
                    "title": "font-size: 16px"
                }
            })
        );
        assert!(keys(&sent).is_subset(&mixin_properties("ShowActionsOptions")));
        assert!(keys(&sent["style"]).is_subset(&nested_properties("ShowActionsOptions", "style")));
        assert_eq!(
            serde_json::to_value(ShowActionsOptions::default()).unwrap(),
            json!({})
        );
    }

    #[test]
    #[allow(deprecated)]
    fn the_deprecated_font_size_is_still_sent() {
        let sent = serde_json::to_value(ShowActionsOptions::default().font_size(14)).unwrap();
        assert_eq!(sent, json!({"fontSize": 14}));
    }

    #[test]
    fn start_params_carry_fps_and_what_the_page_needs() {
        let options = ScreencastStartOptions::default()
            .size(ScreencastSize {
                width: 640,
                height: 480,
            })
            .quality(80)
            .fps(60)
            .path("out.webm".into());
        let sent = options.params(true);
        assert_eq!(
            sent,
            json!({
                "size": {"width": 640, "height": 480},
                "quality": 80,
                "fps": 60,
                "sendFrames": true,
                "record": true
            })
        );
        assert!(keys(&sent).is_subset(&command_parameters(PAGE, "screencastStart")));
        assert_eq!(
            ScreencastStartOptions::default().params(false),
            json!({"sendFrames": false, "record": false})
        );
    }

    #[test]
    fn start_builder_sets_quality_and_size() {
        let o = ScreencastStartOptions::default()
            .quality(80)
            .size(ScreencastSize {
                width: 1280,
                height: 720,
            });
        assert_eq!(o.quality, Some(80));
        assert_eq!(o.size.map(|s| (s.width, s.height)), Some((1280, 720)));
    }

    #[test]
    fn action_cursor_as_str_maps_each_variant() {
        assert_eq!(ActionCursor::None.as_str(), "none");
        assert_eq!(ActionCursor::Pointer.as_str(), "pointer");
    }

    #[test]
    fn action_position_as_str_maps_each_variant() {
        assert_eq!(ActionPosition::TopLeft.as_str(), "top-left");
        assert_eq!(ActionPosition::Top.as_str(), "top");
        assert_eq!(ActionPosition::TopRight.as_str(), "top-right");
        assert_eq!(ActionPosition::BottomLeft.as_str(), "bottom-left");
        assert_eq!(ActionPosition::Bottom.as_str(), "bottom");
        assert_eq!(ActionPosition::BottomRight.as_str(), "bottom-right");
    }

    #[test]
    fn show_actions_builder_sets_cursor() {
        let o = ShowActionsOptions::default().cursor(ActionCursor::Pointer);
        assert_eq!(o.cursor, Some(ActionCursor::Pointer));
    }
}
