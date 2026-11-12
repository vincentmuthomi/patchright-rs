use super::{Geolocation, HttpCredentials, StorageState};
use crate::api::launch_options::IgnoreDefaultArgs;
use crate::protocol::ProxySettings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Viewport dimensions for browser context.
///
/// See: <https://playwright.dev/docs/api/class-browser#browser-new-context>
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Viewport {
    /// Page width in pixels
    pub width: u32,
    /// Page height in pixels
    pub height: u32,
}

impl From<crate::protocol::DeviceViewport> for Viewport {
    fn from(size: crate::protocol::DeviceViewport) -> Self {
        Self {
            width: size.width,
            height: size.height,
        }
    }
}

/// Options for recording HAR.
///
/// See: <https://playwright.dev/docs/api/class-browser#browser-new-context-option-record-har>
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RecordHar {
    /// Path on the filesystem to write the HAR file to.
    pub path: String,
    /// Optional setting to control whether to omit request content from the HAR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omit_content: Option<bool>,
    /// Optional setting to control resource content management.
    /// "omit" | "embed" | "attach"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// "full" | "minimal"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// A URL glob; only requests whose URL matches are stored in the HAR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_filter: Option<String>,
}

impl RecordHar {
    /// Record a HAR to the given path.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            omit_content: None,
            content: None,
            mode: None,
            url_filter: None,
        }
    }
    /// Omit response bodies from the HAR.
    pub fn omit_content(mut self, omit_content: bool) -> Self {
        self.omit_content = Some(omit_content);
        self
    }
    /// Content mode ("embed", "attach", or "omit").
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }
    /// Recording mode ("full" or "minimal").
    pub fn mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = Some(mode.into());
        self
    }
    /// Only record requests matching this URL glob.
    pub fn url_filter(mut self, url_filter: impl Into<String>) -> Self {
        self.url_filter = Some(url_filter.into());
        self
    }
}

/// Options for recording video.
///
/// See: <https://playwright.dev/docs/api/class-browser#browser-new-context-option-record-video>
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RecordVideo {
    /// Path to the directory to put videos into.
    pub dir: String,
    /// Optional dimensions of the recorded videos.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Viewport>,
    /// Frame rate of the recorded videos. Must be positive. Firefox and
    /// WebKit capture at most 25 frames per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    /// Decorate each action in the recorded videos, as
    /// [`Screencast::show_actions`](crate::protocol::Screencast::show_actions)
    /// does for a screencast.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_actions: Option<crate::protocol::ShowActionsOptions>,
}

impl RecordVideo {
    /// Record videos into the given directory.
    pub fn new(dir: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            ..Self::default()
        }
    }
    /// Recorded video size.
    pub fn size(mut self, size: Viewport) -> Self {
        self.size = Some(size);
        self
    }
    /// Recorded video frame rate.
    pub fn fps(mut self, fps: u32) -> Self {
        self.fps = Some(fps);
        self
    }
    /// Decorate each action in the recorded videos.
    pub fn show_actions(mut self, show_actions: crate::protocol::ShowActionsOptions) -> Self {
        self.show_actions = Some(show_actions);
        self
    }
}

/// Controls how downloads are handled in a [`BrowserContext`](super::BrowserContext).
///
/// See the `accept_downloads` field of [`BrowserContextOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum AcceptDownloads {
    /// Allow and capture downloads via the `download` event.
    #[serde(rename = "accept")]
    Accept,
    /// Block downloads.
    #[serde(rename = "deny")]
    Deny,
    /// Let the browser handle downloads natively without routing through Playwright.
    #[serde(rename = "internal")]
    Internal,
}

impl From<bool> for AcceptDownloads {
    fn from(value: bool) -> Self {
        if value { Self::Accept } else { Self::Deny }
    }
}

/// `extraHTTPHeaders` goes to the driver as its `NameValue[]` list, not as
/// the map the builder takes. The pairs are streamed from the map rather
/// than copied into an intermediate value.
fn serialize_header_pairs<S: serde::Serializer>(
    headers: &Option<HashMap<String, String>>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct NameValue<'a> {
        name: &'a str,
        value: &'a str,
    }
    match headers {
        Some(headers) => serializer.collect_seq(
            headers
                .iter()
                .map(|(name, value)| NameValue { name, value }),
        ),
        None => serializer.serialize_none(),
    }
}

/// Options for creating a new browser context.
///
/// Allows customizing viewport, user agent, locale, timezone, geolocation,
/// permissions, and other browser context settings.
///
/// A proxy and a saved session, built inline; `storage_state_path` loads the
/// same state from a file written by `BrowserContext::storage_state`:
///
/// ```rust
/// use patchright_rs::protocol::{
///     BrowserContextOptions, Cookie, LocalStorageItem, Origin, ProxySettings, StorageState,
/// };
///
/// let session = StorageState::default()
///     .cookies(vec![
///         Cookie::new("session_id", "abc123")
///             .domain(".example.com")
///             .http_only(true)
///             .secure(true)
///             .same_site("Lax"),
///     ])
///     .origins(vec![Origin::new(
///         "https://example.com",
///         vec![LocalStorageItem::new("user_prefs", "{\"theme\":\"dark\"}")],
///     )]);
///
/// let options = BrowserContextOptions::builder()
///     .proxy(
///         ProxySettings::new("http://proxy.example.com:8080")
///             .bypass(".example.com")
///             .username("user")
///             .password("pass"),
///     )
///     .storage_state(session)
///     .build();
/// ```
///
/// See: <https://playwright.dev/docs/api/class-browser#browser-new-context>
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct BrowserContextOptions {
    /// Sets consistent viewport for all pages in the context.
    /// Set to null via `no_viewport(true)` to disable viewport emulation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,

    /// Disables viewport emulation when set to true.
    /// Note: Playwright's public API calls this `noViewport`, but the protocol
    /// expects `noDefaultViewport`. playwright-python applies this transformation
    /// in `_prepare_browser_context_params`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "noDefaultViewport")]
    pub no_viewport: Option<bool>,

    /// The screen size `window.screen` and media queries report, separate
    /// from the viewport. Defaults to the viewport's size, and is used only
    /// when a viewport is set, so it has no effect with `no_viewport`. A
    /// device descriptor's [`screen`](crate::protocol::DeviceDescriptor::screen)
    /// belongs here when emulating that device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<Viewport>,

    /// Custom user agent string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,

    /// Locale for the context (e.g., "en-GB", "de-DE", "fr-FR")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,

    /// Timezone identifier (e.g., "America/New_York", "Europe/Berlin")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone_id: Option<String>,

    /// Geolocation coordinates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geolocation: Option<Geolocation>,

    /// Credentials for HTTP authentication, matched per request origin.
    #[serde(rename = "httpCredentials", skip_serializing_if = "Option::is_none")]
    pub http_credentials: Option<Vec<HttpCredentials>>,

    /// List of permissions to grant (e.g., "geolocation", "notifications")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,

    /// Network proxy settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,

    /// Emulates 'prefers-colors-scheme' media feature ("light", "dark", "no-preference")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,

    /// Whether the viewport supports touch events
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_touch: Option<bool>,

    /// Whether the meta viewport tag is respected
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_mobile: Option<bool>,

    /// Whether JavaScript is enabled in the context
    #[serde(rename = "javaScriptEnabled", skip_serializing_if = "Option::is_none")]
    pub javascript_enabled: Option<bool>,

    /// Emulates network being offline
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,

    /// How to handle downloads. See [`AcceptDownloads`] for options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept_downloads: Option<AcceptDownloads>,

    /// Whether to bypass Content-Security-Policy
    #[serde(rename = "bypassCSP", skip_serializing_if = "Option::is_none")]
    pub bypass_csp: Option<bool>,

    /// Whether to ignore HTTPS errors
    #[serde(rename = "ignoreHTTPSErrors", skip_serializing_if = "Option::is_none")]
    pub ignore_https_errors: Option<bool>,

    /// Device scale factor (default: 1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_scale_factor: Option<f64>,

    /// Extra HTTP headers to send with every request
    #[serde(
        rename = "extraHTTPHeaders",
        serialize_with = "serialize_header_pairs",
        skip_serializing_if = "Option::is_none"
    )]
    pub extra_http_headers: Option<HashMap<String, String>>,

    /// Base URL for relative navigation
    #[serde(rename = "baseURL", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Storage state to populate the context (cookies, localStorage, sessionStorage).
    /// Can be an inline StorageState object or a file path string.
    /// Use builder methods `storage_state()` for inline or `storage_state_path()` for file path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_state: Option<StorageState>,

    /// Storage state file path (alternative to inline storage_state).
    /// This is handled by the builder and converted to storage_state during serialization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_state_path: Option<String>,

    // Launch options (for launch_persistent_context)
    /// Additional arguments to pass to browser instance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    /// Browser distribution channel (e.g., "chrome", "msedge")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,

    /// Enable Chromium sandboxing (default: false on Linux)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromium_sandbox: Option<bool>,

    /// Auto-open DevTools.
    ///
    /// Playwright 1.58 removed this option. The driver drops undeclared
    /// parameters without erroring, so setting it has no effect and no longer
    /// reaches the wire. Nor does it imply a headed browser any more. For
    /// Chromium, pass `--auto-open-devtools-for-tabs` in `args` and set
    /// `headless(false)`. Both apply only to
    /// `BrowserType::launch_persistent_context`; for a context from
    /// `Browser::new_context`, set them on the [`LaunchOptions`] the browser
    /// was launched with.
    ///
    /// [`LaunchOptions`]: crate::api::LaunchOptions
    #[deprecated(
        since = "0.20.0",
        note = "removed from the protocol in Playwright 1.58; setting it has no effect. For Chromium, pass `--auto-open-devtools-for-tabs` in `args` (persistent contexts) or `LaunchOptions::args` instead"
    )]
    #[serde(skip_serializing)]
    pub devtools: Option<bool>,

    /// Directory to save downloads
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads_path: Option<String>,

    /// Path to custom browser executable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,

    /// Firefox user preferences (Firefox only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firefox_user_prefs: Option<HashMap<String, serde_json::Value>>,

    /// Run in headless mode (default: true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headless: Option<bool>,

    /// Filter or disable default browser arguments.
    /// When `true`, Playwright does not pass its own default args.
    /// When an array, filters out the given default arguments.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsertype#browser-type-launch-persistent-context>
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_default_args: Option<IgnoreDefaultArgs>,

    /// Slow down operations by N milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slow_mo: Option<f64>,

    /// Timeout for browser launch in milliseconds, for
    /// `launch_persistent_context` (default:
    /// [`DEFAULT_LAUNCH_TIMEOUT_MS`](crate::DEFAULT_LAUNCH_TIMEOUT_MS))
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,

    /// Directory to save traces
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traces_dir: Option<String>,

    /// Check if strict selectors mode is enabled
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_selectors: Option<bool>,

    /// Emulates 'prefers-reduced-motion' media feature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduced_motion: Option<String>,

    /// Emulates 'forced-colors' media feature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forced_colors: Option<String>,

    /// Whether to allow sites to register Service workers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_workers: Option<String>,

    /// Records the context's network traffic as a HAR file. Not a driver
    /// parameter: the recording starts once the context exists and is written
    /// when it closes (see [`BrowserContextOptionsBuilder::record_har`]).
    #[serde(skip)]
    pub record_har: Option<RecordHar>,

    /// Options for recording video
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_video: Option<RecordVideo>,
}

impl BrowserContextOptions {
    /// Creates a new builder for BrowserContextOptions
    pub fn builder() -> BrowserContextOptionsBuilder {
        BrowserContextOptionsBuilder::default()
    }
}

/// Builder for BrowserContextOptions
#[derive(Debug, Clone, Default)]
pub struct BrowserContextOptionsBuilder {
    viewport: Option<Viewport>,
    no_viewport: Option<bool>,
    screen: Option<Viewport>,
    user_agent: Option<String>,
    locale: Option<String>,
    timezone_id: Option<String>,
    geolocation: Option<Geolocation>,
    http_credentials: Option<Vec<HttpCredentials>>,
    permissions: Option<Vec<String>>,
    proxy: Option<ProxySettings>,
    color_scheme: Option<String>,
    has_touch: Option<bool>,
    is_mobile: Option<bool>,
    javascript_enabled: Option<bool>,
    offline: Option<bool>,
    accept_downloads: Option<AcceptDownloads>,
    bypass_csp: Option<bool>,
    ignore_https_errors: Option<bool>,
    device_scale_factor: Option<f64>,
    extra_http_headers: Option<HashMap<String, String>>,
    base_url: Option<String>,
    storage_state: Option<StorageState>,
    storage_state_path: Option<String>,
    // Launch options
    args: Option<Vec<String>>,
    channel: Option<String>,
    chromium_sandbox: Option<bool>,
    devtools: Option<bool>,
    downloads_path: Option<String>,
    executable_path: Option<String>,
    firefox_user_prefs: Option<HashMap<String, serde_json::Value>>,
    headless: Option<bool>,
    ignore_default_args: Option<IgnoreDefaultArgs>,
    slow_mo: Option<f64>,
    timeout: Option<f64>,
    traces_dir: Option<String>,
    strict_selectors: Option<bool>,
    reduced_motion: Option<String>,
    forced_colors: Option<String>,
    service_workers: Option<String>,
    record_har: Option<RecordHar>,
    record_video: Option<RecordVideo>,
}

impl BrowserContextOptionsBuilder {
    /// Sets the viewport dimensions
    pub fn viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = Some(viewport);
        self.no_viewport = None; // Clear no_viewport if setting viewport
        self
    }

    /// Disables viewport emulation
    pub fn no_viewport(mut self, no_viewport: bool) -> Self {
        self.no_viewport = Some(no_viewport);
        if no_viewport {
            self.viewport = None; // Clear viewport if setting no_viewport
        }
        self
    }

    /// Sets the screen size `window.screen` and media queries report,
    /// separate from the viewport. Used only when a viewport is set. Takes a
    /// device descriptor's `screen` directly.
    ///
    /// See: <https://playwright.dev/docs/api/class-browser#browser-new-context-option-screen>
    pub fn screen(mut self, screen: impl Into<Viewport>) -> Self {
        self.screen = Some(screen.into());
        self
    }

    /// Sets the user agent string
    pub fn user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = Some(user_agent);
        self
    }

    /// Sets the locale
    pub fn locale(mut self, locale: String) -> Self {
        self.locale = Some(locale);
        self
    }

    /// Sets the timezone identifier
    pub fn timezone_id(mut self, timezone_id: String) -> Self {
        self.timezone_id = Some(timezone_id);
        self
    }

    /// Sets the geolocation
    pub fn geolocation(mut self, geolocation: Geolocation) -> Self {
        self.geolocation = Some(geolocation);
        self
    }

    /// Sets credentials for HTTP authentication.
    ///
    /// Each request uses the first entry whose `origin` matches it; an entry
    /// without an origin matches anything.
    pub fn http_credentials(mut self, credentials: Vec<HttpCredentials>) -> Self {
        self.http_credentials = Some(credentials);
        self
    }

    /// Sets the permissions to grant
    pub fn permissions(mut self, permissions: Vec<String>) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// Sets the network proxy settings for this context.
    ///
    /// This allows routing all network traffic through a proxy server,
    /// useful for rotating proxies without creating new browsers.
    ///
    /// See: <https://playwright.dev/docs/api/class-browser#browser-new-context>
    pub fn proxy(mut self, proxy: ProxySettings) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Sets the color scheme preference
    pub fn color_scheme(mut self, color_scheme: String) -> Self {
        self.color_scheme = Some(color_scheme);
        self
    }

    /// Sets whether the viewport supports touch events
    pub fn has_touch(mut self, has_touch: bool) -> Self {
        self.has_touch = Some(has_touch);
        self
    }

    /// Sets whether this is a mobile viewport
    pub fn is_mobile(mut self, is_mobile: bool) -> Self {
        self.is_mobile = Some(is_mobile);
        self
    }

    /// Sets whether JavaScript is enabled
    pub fn javascript_enabled(mut self, javascript_enabled: bool) -> Self {
        self.javascript_enabled = Some(javascript_enabled);
        self
    }

    /// Sets whether to emulate offline network
    pub fn offline(mut self, offline: bool) -> Self {
        self.offline = Some(offline);
        self
    }

    /// Sets how to handle downloads. Accepts `AcceptDownloads` or `bool`
    /// (`true` → `Accept`, `false` → `Deny`).
    pub fn accept_downloads(mut self, accept_downloads: impl Into<AcceptDownloads>) -> Self {
        self.accept_downloads = Some(accept_downloads.into());
        self
    }

    /// Sets whether to bypass Content-Security-Policy
    pub fn bypass_csp(mut self, bypass_csp: bool) -> Self {
        self.bypass_csp = Some(bypass_csp);
        self
    }

    /// Sets whether to ignore HTTPS errors
    pub fn ignore_https_errors(mut self, ignore_https_errors: bool) -> Self {
        self.ignore_https_errors = Some(ignore_https_errors);
        self
    }

    /// Sets the device scale factor
    pub fn device_scale_factor(mut self, device_scale_factor: f64) -> Self {
        self.device_scale_factor = Some(device_scale_factor);
        self
    }

    /// Sets extra HTTP headers
    pub fn extra_http_headers(mut self, extra_http_headers: HashMap<String, String>) -> Self {
        self.extra_http_headers = Some(extra_http_headers);
        self
    }

    /// Sets the base URL for relative navigation
    pub fn base_url(mut self, base_url: String) -> Self {
        self.base_url = Some(base_url);
        self
    }

    /// Sets the storage state inline (cookies, localStorage).
    ///
    /// Populates the browser context with the provided storage state, including
    /// cookies and local storage. This is useful for initializing a context with
    /// a saved authentication state.
    ///
    /// Mutually exclusive with `storage_state_path()`.
    ///
    /// See: <https://playwright.dev/docs/api/class-browser#browser-new-context-option-storage-state>
    pub fn storage_state(mut self, storage_state: StorageState) -> Self {
        self.storage_state = Some(storage_state);
        self.storage_state_path = None; // Clear path if setting inline
        self
    }

    /// Sets the storage state from a file path.
    ///
    /// The file should contain a JSON representation of StorageState with cookies
    /// and origins. This is useful for loading authentication state saved from a
    /// previous session.
    ///
    /// Mutually exclusive with `storage_state()`.
    ///
    /// The file should have this format:
    /// ```json
    /// {
    ///   "cookies": [{
    ///     "name": "session_id",
    ///     "value": "abc123",
    ///     "domain": ".example.com",
    ///     "path": "/",
    ///     "expires": -1,
    ///     "httpOnly": true,
    ///     "secure": true,
    ///     "sameSite": "Lax"
    ///   }],
    ///   "origins": [{
    ///     "origin": "https://example.com",
    ///     "localStorage": [{
    ///       "name": "user_prefs",
    ///       "value": "{\"theme\":\"dark\"}"
    ///     }]
    ///   }]
    /// }
    /// ```
    ///
    /// See: <https://playwright.dev/docs/api/class-browser#browser-new-context-option-storage-state>
    pub fn storage_state_path(mut self, path: String) -> Self {
        self.storage_state_path = Some(path);
        self.storage_state = None; // Clear inline if setting path
        self
    }

    /// Sets additional arguments to pass to browser instance (for launch_persistent_context)
    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = Some(args);
        self
    }

    /// Sets browser distribution channel (for launch_persistent_context)
    pub fn channel(mut self, channel: String) -> Self {
        self.channel = Some(channel);
        self
    }

    /// Enables or disables Chromium sandboxing (for launch_persistent_context)
    pub fn chromium_sandbox(mut self, enabled: bool) -> Self {
        self.chromium_sandbox = Some(enabled);
        self
    }

    /// Auto-open DevTools. Playwright 1.58 removed the option, so this has no effect.
    #[deprecated(
        since = "0.20.0",
        note = "removed from the protocol in Playwright 1.58; setting it has no effect. For Chromium, pass `--auto-open-devtools-for-tabs` in `args` (persistent contexts) or `LaunchOptions::args` instead"
    )]
    pub fn devtools(mut self, enabled: bool) -> Self {
        self.devtools = Some(enabled);
        self
    }

    /// Sets directory to save downloads (for launch_persistent_context)
    pub fn downloads_path(mut self, path: String) -> Self {
        self.downloads_path = Some(path);
        self
    }

    /// Sets path to custom browser executable (for launch_persistent_context)
    pub fn executable_path(mut self, path: String) -> Self {
        self.executable_path = Some(path);
        self
    }

    /// Sets Firefox user preferences (for launch_persistent_context, Firefox only)
    pub fn firefox_user_prefs(mut self, prefs: HashMap<String, serde_json::Value>) -> Self {
        self.firefox_user_prefs = Some(prefs);
        self
    }

    /// Run in headless mode (for launch_persistent_context)
    pub fn headless(mut self, enabled: bool) -> Self {
        self.headless = Some(enabled);
        self
    }

    /// Filter or disable default browser arguments (for launch_persistent_context).
    ///
    /// When `IgnoreDefaultArgs::Bool(true)`, Playwright does not pass its own
    /// default arguments and only uses the ones from `args`.
    /// When `IgnoreDefaultArgs::Array(vec)`, filters out the given default arguments.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsertype#browser-type-launch-persistent-context>
    pub fn ignore_default_args(mut self, args: IgnoreDefaultArgs) -> Self {
        self.ignore_default_args = Some(args);
        self
    }

    /// Slow down operations by N milliseconds (for launch_persistent_context)
    pub fn slow_mo(mut self, ms: f64) -> Self {
        self.slow_mo = Some(ms);
        self
    }

    /// Set timeout for browser launch in milliseconds (for launch_persistent_context)
    pub fn timeout(mut self, ms: f64) -> Self {
        self.timeout = Some(ms);
        self
    }

    /// Set directory to save traces (for launch_persistent_context)
    pub fn traces_dir(mut self, path: String) -> Self {
        self.traces_dir = Some(path);
        self
    }

    /// Check if strict selectors mode is enabled
    pub fn strict_selectors(mut self, enabled: bool) -> Self {
        self.strict_selectors = Some(enabled);
        self
    }

    /// Emulates 'prefers-reduced-motion' media feature
    pub fn reduced_motion(mut self, value: String) -> Self {
        self.reduced_motion = Some(value);
        self
    }

    /// Emulates 'forced-colors' media feature
    pub fn forced_colors(mut self, value: String) -> Self {
        self.forced_colors = Some(value);
        self
    }

    /// Whether to allow sites to register Service workers ("allow" | "block")
    pub fn service_workers(mut self, value: String) -> Self {
        self.service_workers = Some(value);
        self
    }

    /// Records the context's network traffic as a HAR file at the path in
    /// `record_har`.
    ///
    /// The file is written when the context closes, by
    /// [`BrowserContext::close`](crate::protocol::BrowserContext::close), so
    /// close the context to get it: a browser closed without closing the
    /// context first writes nothing. A `.zip` path stores resource bodies as
    /// separate entries; any other path embeds them in the `.har`.
    ///
    /// # Errors
    ///
    /// Creating the context fails with
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument), before any
    /// context exists, if the `content` or `mode` string is not one
    /// Playwright accepts.
    ///
    /// If the HAR cannot be written, `close` returns the error and leaves the
    /// context open; a second `close` closes it.
    ///
    /// See: <https://playwright.dev/docs/api/class-browser#browser-new-context-option-record-har>
    pub fn record_har(mut self, record_har: RecordHar) -> Self {
        self.record_har = Some(record_har);
        self
    }

    /// Sets options for recording video
    pub fn record_video(mut self, record_video: RecordVideo) -> Self {
        self.record_video = Some(record_video);
        self
    }

    /// Builds the BrowserContextOptions
    pub fn build(self) -> BrowserContextOptions {
        BrowserContextOptions {
            viewport: self.viewport,
            no_viewport: self.no_viewport,
            screen: self.screen,
            user_agent: self.user_agent,
            locale: self.locale,
            timezone_id: self.timezone_id,
            geolocation: self.geolocation,
            http_credentials: self.http_credentials,
            permissions: self.permissions,
            proxy: self.proxy,
            color_scheme: self.color_scheme,
            has_touch: self.has_touch,
            is_mobile: self.is_mobile,
            javascript_enabled: self.javascript_enabled,
            offline: self.offline,
            accept_downloads: self.accept_downloads,
            bypass_csp: self.bypass_csp,
            ignore_https_errors: self.ignore_https_errors,
            device_scale_factor: self.device_scale_factor,
            extra_http_headers: self.extra_http_headers,
            base_url: self.base_url,
            storage_state: self.storage_state,
            storage_state_path: self.storage_state_path,
            // Launch options
            args: self.args,
            channel: self.channel,
            chromium_sandbox: self.chromium_sandbox,
            #[allow(deprecated)]
            devtools: self.devtools,
            downloads_path: self.downloads_path,
            executable_path: self.executable_path,
            firefox_user_prefs: self.firefox_user_prefs,
            headless: self.headless,
            ignore_default_args: self.ignore_default_args,
            slow_mo: self.slow_mo,
            timeout: self.timeout,
            traces_dir: self.traces_dir,
            strict_selectors: self.strict_selectors,
            reduced_motion: self.reduced_motion,
            forced_colors: self.forced_colors,
            service_workers: self.service_workers,
            record_har: self.record_har,
            record_video: self.record_video,
        }
    }
}

/// Options for [`BrowserContext::close_with_options`](super::BrowserContext::close_with_options).
///
/// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-close>
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct BrowserContextCloseOptions {
    /// The reason reported by operations the close interrupts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl BrowserContextCloseOptions {
    /// The reason interrupted operations report.
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_options_send_only_a_set_reason_under_the_driver_spelling() {
        use crate::protocol_spec::{BROWSER_CONTEXT, command_parameters, keys};
        assert_eq!(
            serde_json::to_value(BrowserContextCloseOptions::default()).unwrap(),
            serde_json::json!({})
        );
        let sent =
            serde_json::to_value(BrowserContextCloseOptions::default().reason("done")).unwrap();
        assert_eq!(sent, serde_json::json!({"reason": "done"}));
        assert!(keys(&sent).is_subset(&command_parameters(BROWSER_CONTEXT, "close")));
    }

    #[test]
    fn test_browser_context_options_ignore_default_args_bool_serialization() {
        let options = BrowserContextOptions::builder()
            .ignore_default_args(IgnoreDefaultArgs::Bool(true))
            .build();

        let value = serde_json::to_value(&options).unwrap();
        assert_eq!(value["ignoreDefaultArgs"], serde_json::json!(true));
    }

    #[test]
    fn test_browser_context_options_ignore_default_args_array_serialization() {
        let options = BrowserContextOptions::builder()
            .ignore_default_args(IgnoreDefaultArgs::Array(vec!["--foo".to_string()]))
            .build();

        let value = serde_json::to_value(&options).unwrap();
        assert_eq!(value["ignoreDefaultArgs"], serde_json::json!(["--foo"]));
    }

    #[test]
    fn test_browser_context_options_ignore_default_args_absent() {
        let options = BrowserContextOptions::builder().build();

        let value = serde_json::to_value(&options).unwrap();
        assert!(value.get("ignoreDefaultArgs").is_none());
    }

    #[test]
    fn every_key_sent_is_a_parameter_the_driver_declares() {
        use crate::protocol_spec::{
            BROWSER, BROWSER_TYPE, command_parameters, keys, mixin_properties, nested_properties,
        };

        // A literal with no `..Default::default()`, so a new field fails to
        // compile here until it is added and checked.
        let options = BrowserContextOptions {
            viewport: Some(Viewport {
                width: 1,
                height: 1,
            }),
            no_viewport: Some(false),
            screen: Some(Viewport {
                width: 1,
                height: 1,
            }),
            user_agent: Some(String::new()),
            locale: Some(String::new()),
            timezone_id: Some(String::new()),
            geolocation: Some(crate::protocol::Geolocation {
                latitude: 0.0,
                longitude: 0.0,
                accuracy: None,
            }),
            http_credentials: Some(vec![crate::protocol::HttpCredentials::new("u", "p")]),
            permissions: Some(vec![]),
            proxy: Some(crate::protocol::ProxySettings::new("http://proxy")),
            color_scheme: Some(String::new()),
            has_touch: Some(false),
            is_mobile: Some(false),
            javascript_enabled: Some(false),
            offline: Some(false),
            accept_downloads: Some(AcceptDownloads::Accept),
            bypass_csp: Some(false),
            ignore_https_errors: Some(false),
            device_scale_factor: Some(1.0),
            extra_http_headers: Some(HashMap::new()),
            base_url: Some(String::new()),
            storage_state: Some(crate::protocol::StorageState::default()),
            storage_state_path: Some(String::new()),
            args: Some(vec![]),
            channel: Some(String::new()),
            chromium_sandbox: Some(false),
            #[allow(deprecated)]
            devtools: Some(false),
            downloads_path: Some(String::new()),
            executable_path: Some(String::new()),
            firefox_user_prefs: Some(HashMap::new()),
            headless: Some(true),
            ignore_default_args: Some(IgnoreDefaultArgs::Array(vec![])),
            slow_mo: Some(0.0),
            timeout: Some(0.0),
            traces_dir: Some(String::new()),
            strict_selectors: Some(false),
            reduced_motion: Some(String::new()),
            forced_colors: Some(String::new()),
            service_workers: Some(String::new()),
            record_har: Some(RecordHar::new("h.har")),
            record_video: Some(
                RecordVideo::new("videos")
                    .size(Viewport {
                        width: 1,
                        height: 1,
                    })
                    .fps(30)
                    .show_actions(
                        crate::protocol::ShowActionsOptions::default()
                            .duration(1.0)
                            .style(crate::protocol::ShowActionsStyle::default().title("x")),
                    ),
            ),
        };

        let value = serde_json::to_value(&options).unwrap();
        let video = &value["recordVideo"];
        assert!(keys(video).is_subset(&nested_properties("ContextOptions", "recordVideo")));
        assert!(keys(&video["showActions"]).is_subset(&mixin_properties("ShowActionsOptions")));
        let sent = keys(&value);
        let mut declared = command_parameters(BROWSER, "newContext");
        declared.extend(command_parameters(BROWSER_TYPE, "launchPersistentContext"));
        let undeclared: Vec<&str> = sent
            .iter()
            .filter(|key| !declared.contains(*key))
            .map(String::as_str)
            .collect();
        // Two of these never reach the driver as parameters: storageStatePath
        // is read and sent inline as storageState, and timeout moves into the
        // message metadata, where the server takes a call's deadline from.
        assert_eq!(undeclared, ["storageStatePath", "timeout"]);
    }

    #[test]
    fn acronym_options_use_the_driver_spelling() {
        let options = BrowserContextOptions::builder()
            .javascript_enabled(false)
            .bypass_csp(true)
            .ignore_https_errors(true)
            .base_url("http://localhost".to_string())
            .build();

        let value = serde_json::to_value(&options).unwrap();
        assert_eq!(value["javaScriptEnabled"], serde_json::json!(false));
        assert_eq!(value["bypassCSP"], serde_json::json!(true));
        assert_eq!(value["ignoreHTTPSErrors"], serde_json::json!(true));
        assert_eq!(value["baseURL"], serde_json::json!("http://localhost"));
        for wrong in [
            "javascriptEnabled",
            "bypassCsp",
            "ignoreHttpsErrors",
            "baseUrl",
        ] {
            assert!(value.get(wrong).is_none(), "sent the unknown key {wrong}");
        }
    }

    #[test]
    fn extra_http_headers_serialize_as_name_value_pairs() {
        let headers = HashMap::from([
            ("X-One".to_string(), "1".to_string()),
            ("X-Two".to_string(), "2".to_string()),
        ]);
        let options = BrowserContextOptions::builder()
            .extra_http_headers(headers)
            .build();

        let value = serde_json::to_value(&options).unwrap();
        assert!(value.get("extraHttpHeaders").is_none());
        let mut pairs = value["extraHTTPHeaders"]
            .as_array()
            .expect("extraHTTPHeaders is a list")
            .clone();
        pairs.sort_by_key(|pair| pair["name"].as_str().unwrap_or_default().to_string());
        assert_eq!(
            pairs,
            vec![
                serde_json::json!({"name": "X-One", "value": "1"}),
                serde_json::json!({"name": "X-Two", "value": "2"}),
            ]
        );
    }

    #[test]
    fn test_accept_downloads_serializes_as_protocol_string() {
        for (variant, expected) in [
            (AcceptDownloads::Accept, "accept"),
            (AcceptDownloads::Deny, "deny"),
            (AcceptDownloads::Internal, "internal"),
        ] {
            let options = BrowserContextOptions::builder()
                .accept_downloads(variant)
                .build();
            let value = serde_json::to_value(&options).unwrap();
            assert_eq!(value["acceptDownloads"], serde_json::json!(expected));
        }
    }

    #[test]
    fn test_accept_downloads_bool_compatibility() {
        let opts = BrowserContextOptions::builder()
            .accept_downloads(true)
            .build();
        assert_eq!(opts.accept_downloads, Some(AcceptDownloads::Accept));

        let opts = BrowserContextOptions::builder()
            .accept_downloads(false)
            .build();
        assert_eq!(opts.accept_downloads, Some(AcceptDownloads::Deny));
    }
}
