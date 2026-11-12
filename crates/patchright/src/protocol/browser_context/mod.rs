// BrowserContext protocol object
//
// Represents an isolated browser context (session) within a browser instance.
// Multiple contexts can exist in a single browser, each with its own cookies,
// cache, and local storage.

use crate::error::Result;
use crate::protocol::{Browser, Page, Request, ResponseObject};
use crate::server::channel_owner::{ChannelOwner, ChannelOwnerImpl};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::protocol::event_registry::EventRegistry;
use tokio::sync::oneshot;

/// Type alias for boxed binding callback future
type BindingCallbackFuture = Pin<Box<dyn Future<Output = serde_json::Value> + Send>>;

/// Type alias for boxed service worker handler future
type ServiceWorkerHandlerFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// Context-level service worker event handler
type ServiceWorkerHandler =
    Arc<dyn Fn(crate::protocol::Worker) -> ServiceWorkerHandlerFuture + Send + Sync>;

/// Type alias for boxed context-level handler future
type CtxHandlerFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// Binding callback: receives deserialized JS args, returns a JSON value
type BindingCallback = Arc<dyn Fn(Vec<serde_json::Value>) -> BindingCallbackFuture + Send + Sync>;

/// BrowserContext represents an isolated browser session.
///
/// Contexts are isolated environments within a browser instance. Each context
/// has its own cookies, cache, and local storage, enabling independent sessions
/// without interference.
///
/// # Example
///
/// ```no_run
/// use patchright_rs::protocol::Playwright;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let playwright = Playwright::launch().await?;
///     let browser = playwright.chromium().launch().await?;
///
///     // Create isolated contexts
///     let context1 = browser.new_context().await?;
///     let context2 = browser.new_context().await?;
///
///     // Create pages in each context
///     let page1 = context1.new_page().await?;
///     let page2 = context2.new_page().await?;
///
///     // Access all pages in a context
///     let pages = context1.pages();
///     assert_eq!(pages.len(), 1);
///
///     // Access the browser from a context
///     let ctx_browser = context1.browser().unwrap();
///     assert_eq!(ctx_browser.name(), browser.name());
///
///     // App mode: access initial page created automatically
///     let chromium = playwright.chromium();
///     let app_context = chromium
///         .launch_persistent_context_with_options(
///             "/tmp/app-data",
///             patchright_rs::protocol::BrowserContextOptions::builder()
///                 .args(vec!["--app=https://example.com".to_string()])
///                 .headless(true)
///                 .build()
///         )
///         .await?;
///
///     // Get the initial page (don't create a new one!)
///     let app_pages = app_context.pages();
///     if !app_pages.is_empty() {
///         let initial_page = &app_pages[0];
///         // Use the initial page...
///     }
///
///     // Cleanup
///     context1.close().await?;
///     context2.close().await?;
///     app_context.close().await?;
///     browser.close().await?;
///     Ok(())
/// }
/// ```
///
/// See: <https://playwright.dev/docs/api/class-browsercontext>
#[derive(Clone)]
pub struct BrowserContext {
    base: ChannelOwnerImpl,
    /// Browser instance that owns this context (None for persistent contexts)
    browser: Option<Browser>,
    /// All open pages in this context
    pages: Arc<Mutex<Vec<Page>>>,
    /// The `base_url` the context was created with, which route patterns,
    /// `wait_for_url` and `to_have_url` resolve against.
    base_url: Option<String>,
    /// Route handlers for context-level network interception
    route_handlers: Arc<crate::protocol::route_handlers::RouteHandlers>,
    /// APIRequestContext GUID from initializer (resolved lazily)
    request_context_guid: Option<String>,
    /// Tracing GUID from initializer (resolved lazily)
    tracing_guid: Option<String>,
    /// Debugger GUID from initializer (resolved lazily)
    debugger_guid: Option<String>,
    /// Default action timeout for all pages in this context (milliseconds), stored as f64 bits.
    default_timeout_ms: Arc<std::sync::atomic::AtomicU64>,
    /// Default navigation timeout for all pages in this context (milliseconds), stored as f64 bits.
    default_navigation_timeout_ms: Arc<std::sync::atomic::AtomicU64>,
    /// `page` event: handlers and one-shot `expect_page` waiters.
    page_events: Arc<EventRegistry<Page>>,
    /// `close` event: one-time transition; `dispatch_all` wakes every waiter.
    close_events: Arc<EventRegistry<()>>,
    /// `request` event: handlers and one-shot waiters.
    request: Arc<EventRegistry<Request>>,
    /// `requestFinished` event handlers (no `expect_*`; waiter queue stays empty).
    request_finished: Arc<EventRegistry<Request>>,
    /// `requestFailed` event handlers (no `expect_*`; waiter queue stays empty).
    request_failed: Arc<EventRegistry<Request>>,
    /// `response` event: handlers and one-shot waiters.
    response: Arc<EventRegistry<ResponseObject>>,
    /// `dialog` event handlers (no `expect_*`; waiter queue stays empty).
    dialog: Arc<EventRegistry<crate::protocol::Dialog>>,
    /// `dialogClosed` handlers, subscribed on the first one rather than
    /// eagerly like `dialog`: nothing accumulates these passively.
    dialog_closed: Arc<EventRegistry<crate::protocol::Dialog>>,
    /// Registered binding callbacks keyed by name (for expose_function / expose_binding)
    binding_callbacks: Arc<Mutex<HashMap<String, BindingCallback>>>,
    /// `console` event: handlers and one-shot `expect_console_message` waiters.
    console: Arc<EventRegistry<crate::protocol::ConsoleMessage>>,
    /// `pageError`-derived weberror event: handlers and one-shot waiters.
    weberror: Arc<EventRegistry<crate::protocol::WebError>>,
    /// Context-level service worker event handlers (fired when a service worker is registered)
    serviceworker_handlers: Arc<Mutex<Vec<ServiceWorkerHandler>>>,
    /// Handlers for the events forwarded from each page.
    forwarders: events::Forwarders,
    /// One-shot senders waiting for the next "serviceworker" event (expect_event("serviceworker"))
    serviceworker_waiters: Arc<Mutex<Vec<oneshot::Sender<crate::protocol::Worker>>>>,
    /// Active service workers tracked via "serviceWorker" events
    service_workers_list: Arc<Mutex<Vec<crate::protocol::Worker>>>,
    /// WebSocketRoute handlers for route_web_socket()
    ws_route_handlers: Arc<crate::protocol::route_handlers::WebSocketRouteHandlers>,
    /// Whether this context has been closed.
    /// Set to true when close() is called or a "close" event is received.
    is_closed: Arc<AtomicBool>,
    /// The HAR the `record_har` option started, until `close()` writes it.
    /// An async lock, held across the export, so a concurrent `close()`
    /// waits for the file instead of closing the context under it.
    option_har: Arc<tokio::sync::Mutex<Option<crate::protocol::tracing::HarRecording>>>,
}

// Each concern is its own `impl BrowserContext` block in a child module.
// Rustdoc lists inherent impls in declaration order, so these follow the
// order a reader meets the API. One comment per line on purpose: rustfmt
// sorts adjacent `mod` lines alphabetically.
// Construction, pages and companion objects, timeouts, close.
mod lifecycle;
// Cookies, storage state, headers, permissions, geolocation, credentials.
mod state;
// Routes and network interception.
mod network;
// Event subscriptions, page-event forwarders and waiters.
mod events;
// Init scripts, exposed functions and bindings.
mod bindings;
// `BrowserContextOptions` and the types it carries.
mod options;
// Server-to-registry event dispatch, last because it is internal.
mod dispatch;

pub use options::{
    AcceptDownloads, BrowserContextCloseOptions, BrowserContextOptions,
    BrowserContextOptionsBuilder, RecordHar, RecordVideo, Viewport,
};
pub use state::{
    ClearCookiesOptions, Cookie, Geolocation, GrantPermissionsOptions, HttpCredentials,
    HttpCredentialsSend, LocalStorageItem, Origin, StorageState, StorageStateOptions,
};

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("guid", &self.guid())
            .finish()
    }
}
