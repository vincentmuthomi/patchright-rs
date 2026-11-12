use super::BrowserContext;
use crate::error::Result;
use crate::protocol::api_request_context::APIRequestContext;
use crate::protocol::cdp_session::CDPSession;
use crate::protocol::event_registry::EventRegistry;
use crate::protocol::route_handlers::{RouteHandlers, WebSocketRouteHandlers};
use crate::protocol::tracing::Tracing;
use crate::protocol::{Browser, Page};
use crate::server::channel::Channel;
use crate::server::channel_owner::{ChannelOwner, ChannelOwnerImpl, ParentOrConnection};
use crate::server::connection::ConnectionExt;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Lifecycle: construction, pages and companion objects, timeouts, close.
impl BrowserContext {
    /// Creates a new BrowserContext from protocol initialization
    ///
    /// This is called by the object factory when the server sends a `__create__` message
    /// for a BrowserContext object.
    ///
    /// # Arguments
    ///
    /// * `parent` - The parent Browser object
    /// * `type_name` - The protocol type name ("BrowserContext")
    /// * `guid` - The unique identifier for this context
    /// * `initializer` - The initialization data from the server
    ///
    /// # Errors
    ///
    /// Returns error if initializer is malformed
    pub fn new(
        parent: Arc<dyn ChannelOwner>,
        type_name: String,
        guid: Arc<str>,
        initializer: Value,
    ) -> Result<Self> {
        // Route patterns on the context and its pages resolve against this.
        let base_url = initializer["options"]["baseURL"]
            .as_str()
            .map(str::to_string);

        // Extract APIRequestContext GUID from initializer before moving it
        let request_context_guid = initializer
            .get("requestContext")
            .and_then(|v| v.get("guid"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract Tracing GUID from initializer before moving it
        let tracing_guid = initializer
            .get("tracing")
            .and_then(|v| v.get("guid"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract Debugger GUID from initializer before moving it
        let debugger_guid = initializer
            .get("debugger")
            .and_then(|v| v.get("guid"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let base = ChannelOwnerImpl::new(
            ParentOrConnection::Parent(parent.clone()),
            type_name,
            guid,
            initializer,
        );

        // Store browser reference if parent is a Browser
        // Returns None only for special contexts (Android, Electron) where parent is not a Browser
        // For both regular contexts and persistent contexts, parent is a Browser instance
        let browser = parent.as_any().downcast_ref::<Browser>().cloned();

        let context = Self {
            base,
            browser,
            pages: Arc::new(Mutex::new(Vec::new())),
            route_handlers: Arc::new(RouteHandlers::new(base_url.as_deref())),
            request_context_guid,
            tracing_guid,
            debugger_guid,
            default_timeout_ms: Arc::new(std::sync::atomic::AtomicU64::new(
                crate::DEFAULT_TIMEOUT_MS.to_bits(),
            )),
            default_navigation_timeout_ms: Arc::new(std::sync::atomic::AtomicU64::new(
                crate::DEFAULT_TIMEOUT_MS.to_bits(),
            )),
            page_events: EventRegistry::new("page"),
            close_events: EventRegistry::new("close"),
            request: EventRegistry::new("request"),
            request_finished: EventRegistry::new("requestFinished"),
            request_failed: EventRegistry::new("requestFailed"),
            response: EventRegistry::new("response"),
            dialog: EventRegistry::new("dialog"),
            dialog_closed: EventRegistry::new("dialogClosed"),
            binding_callbacks: Arc::new(Mutex::new(HashMap::new())),
            console: EventRegistry::new("console"),
            weberror: EventRegistry::new("weberror"),
            serviceworker_handlers: Arc::new(Mutex::new(Vec::new())),
            forwarders: super::events::Forwarders::default(),
            serviceworker_waiters: Arc::new(Mutex::new(Vec::new())),
            service_workers_list: Arc::new(Mutex::new(Vec::new())),
            ws_route_handlers: Arc::new(WebSocketRouteHandlers::new(base_url.as_deref())),
            base_url,
            is_closed: Arc::new(AtomicBool::new(false)),
            option_har: Arc::new(tokio::sync::Mutex::new(None)),
        };

        // Console events must be subscribed to receive them without a registered handler,
        // enabling the console_messages() and page_errors() passive accumulators on Page.
        // Dialogs are not: the driver answers a dialog nobody subscribed to, and an
        // eager subscription would leave every unhandled dialog to the client.
        let channel = context.channel().clone();
        crate::task::spawn(async move {
            _ = channel.update_subscription("console", true).await;
        });

        // Note: Selectors registration is done by the caller (e.g. Browser::new_context())
        // after this object is returned, so that add_context() can be awaited properly.

        Ok(context)
    }

    /// Returns the channel for sending protocol messages
    ///
    /// Used internally for sending RPC calls to the context.
    pub(super) fn channel(&self) -> &Channel {
        self.base.channel()
    }

    /// Creates a new page in this browser context.
    ///
    /// Pages are isolated tabs/windows within a context. Each page starts
    /// at "about:blank" and can be navigated independently.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Context has been closed
    /// - Communication with browser process fails
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-new-page>
    #[tracing::instrument(level = "info", skip_all, fields(guid = %self.guid()))]
    pub async fn new_page(&self) -> Result<Page> {
        // Response contains the GUID of the created Page
        #[derive(Deserialize)]
        struct NewPageResponse {
            page: GuidRef,
        }

        #[derive(Deserialize)]
        struct GuidRef {
            #[serde(deserialize_with = "crate::server::connection::deserialize_arc_str")]
            guid: Arc<str>,
        }

        // Send newPage RPC to server
        let response: NewPageResponse = self
            .channel()
            .send("newPage", serde_json::json!({}))
            .await?;

        // Retrieve and downcast the Page object from the connection registry
        let page: Page = self
            .connection()
            .get_typed::<Page>(&response.page.guid)
            .await?;

        // Note: Don't track the page here - it will be tracked via the "page" event
        // that Playwright server sends automatically when a page is created.
        // Tracking it here would create duplicates.

        // Propagate context-level timeout defaults to the new page
        let ctx_timeout = self.default_timeout_ms();
        let ctx_nav_timeout = self.default_navigation_timeout_ms();
        if ctx_timeout.to_bits() != crate::DEFAULT_TIMEOUT_MS.to_bits() {
            page.set_default_timeout(ctx_timeout).await;
        }
        if ctx_nav_timeout.to_bits() != crate::DEFAULT_TIMEOUT_MS.to_bits() {
            page.set_default_navigation_timeout(ctx_nav_timeout).await;
        }

        Ok(page)
    }

    /// Returns all open pages in the context.
    ///
    /// This method provides a snapshot of all currently active pages that belong
    /// to this browser context instance. Pages created via `new_page()` and popup
    /// pages opened through user interactions are included.
    ///
    /// In persistent contexts launched with `--app=url`, this will include the
    /// initial page created automatically by Playwright.
    ///
    /// # Errors
    ///
    /// This method does not return errors. It provides a snapshot of pages at
    /// the time of invocation.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-pages>
    pub fn pages(&self) -> Vec<Page> {
        self.pages.lock().unwrap().clone()
    }

    /// Returns all active service workers registered in this browser context.
    ///
    /// Service workers are accumulated as they are registered (`serviceWorker` event).
    /// Each call returns a snapshot of the current list.
    ///
    /// Note: Testing service workers typically requires HTTPS. In plain HTTP or
    /// `about:blank` contexts this list is empty.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-service-workers>
    pub fn service_workers(&self) -> Vec<crate::protocol::Worker> {
        self.service_workers_list.lock().unwrap().clone()
    }

    /// Returns the browser instance that owns this context.
    ///
    /// Returns `None` only for contexts created outside of normal browser
    /// (e.g., Android or Electron contexts). For both regular contexts and
    /// persistent contexts, this returns the owning Browser instance.
    ///
    /// # Errors
    ///
    /// This method does not return errors.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-browser>
    pub fn browser(&self) -> Option<Browser> {
        self.browser.clone()
    }

    /// Returns the APIRequestContext associated with this context.
    ///
    /// The APIRequestContext is created automatically by the server for each
    /// BrowserContext. It enables performing HTTP requests and is used internally
    /// by `Route::fetch()`.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-request>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn request(&self) -> Result<APIRequestContext> {
        let guid = self.request_context_guid.as_ref().ok_or_else(|| {
            crate::error::Error::ProtocolError(
                "No APIRequestContext available for this context".to_string(),
            )
        })?;

        self.connection().get_typed::<APIRequestContext>(guid).await
    }

    /// Creates a new Chrome DevTools Protocol session for the given page.
    ///
    /// CDPSession provides low-level access to the Chrome DevTools Protocol.
    /// This method is only available in Chromium-based browsers.
    ///
    /// # Arguments
    ///
    /// * `page` - The page to create a CDP session for
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The browser is not Chromium-based
    /// - Context has been closed
    /// - Communication with browser process fails
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-new-cdp-session>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid(), page_guid = %page.guid()))]
    pub async fn new_cdp_session(&self, page: &Page) -> Result<CDPSession> {
        #[derive(serde::Deserialize)]
        struct NewCDPSessionResponse {
            session: GuidRef,
        }

        #[derive(serde::Deserialize)]
        struct GuidRef {
            #[serde(deserialize_with = "crate::server::connection::deserialize_arc_str")]
            guid: Arc<str>,
        }

        let response: NewCDPSessionResponse = self
            .channel()
            .send(
                "newCDPSession",
                serde_json::json!({ "page": { "guid": page.guid() } }),
            )
            .await?;

        self.connection()
            .get_typed::<CDPSession>(&response.session.guid)
            .await
    }

    /// Returns the Tracing object for this browser context.
    ///
    /// The Tracing object is created automatically by the Playwright server for each
    /// BrowserContext. Use it to start and stop trace recording.
    ///
    /// # Errors
    ///
    /// Returns error if no Tracing object is available for this context (rare,
    /// should not happen in normal usage).
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-tracing>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn tracing(&self) -> Result<Tracing> {
        let guid = self.tracing_guid.as_ref().ok_or_else(|| {
            crate::error::Error::ProtocolError(
                "No Tracing object available for this context".to_string(),
            )
        })?;

        self.connection().get_typed::<Tracing>(guid).await
    }

    /// Returns the [`Debugger`](crate::protocol::Debugger) for this context.
    ///
    /// The Debugger surfaces programmatic control of Playwright Inspector's
    /// "PAUSED" overlay — `request_pause`, `resume`, `next`, `run_to`, and a
    /// `pausedStateChanged` event. Used by IDE integrations and
    /// inspector-style tools.
    ///
    /// See: <https://playwright.dev/docs/api/class-debugger>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn debugger(&self) -> Result<crate::protocol::Debugger> {
        let guid = self.debugger_guid.as_ref().ok_or_else(|| {
            crate::error::Error::ProtocolError(
                "No Debugger object available for this context".to_string(),
            )
        })?;
        self.connection()
            .get_typed::<crate::protocol::Debugger>(guid)
            .await
    }

    /// Returns the Clock object for this browser context.
    ///
    /// The Clock object enables fake timer control — install fake timers,
    /// fast-forward time, pause/resume, and set fixed or system time.
    ///
    /// `page.clock()` delegates to this method via the page's parent context.
    ///
    /// See: <https://playwright.dev/docs/api/class-clock>
    pub fn clock(&self) -> crate::protocol::clock::Clock {
        crate::protocol::clock::Clock::for_context(self)
    }

    /// Manage the context's virtual WebAuthn authenticator:
    /// install it, then register / list / delete passkeys for
    /// `navigator.credentials` testing.
    ///
    /// See: <https://playwright.dev/docs/api/class-credentials>
    pub fn credentials(&self) -> crate::protocol::Credentials {
        crate::protocol::Credentials::new(self.channel().clone())
    }

    /// Starts the HAR recording a `record_har` option asks for, once the
    /// context exists. [`close`](Self::close) writes it.
    pub(crate) async fn start_option_har(
        &self,
        har: crate::protocol::har_options::OptionHar,
    ) -> Result<()> {
        let recording = self
            .tracing()
            .await?
            .begin_har(har.path, har.options)
            .await?;
        *self.option_har.lock().await = Some(recording);
        Ok(())
    }

    /// Closes the browser context and all its pages.
    ///
    /// This is a graceful operation that sends a close command to the context
    /// and waits for it to shut down properly.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Context has already been closed
    /// - Communication with browser process fails
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-close>
    #[tracing::instrument(level = "info", skip_all, fields(guid = %self.guid()))]
    pub async fn close(&self) -> Result<()> {
        self.close_with_options(super::BrowserContextCloseOptions::default())
            .await
    }

    /// Closes the browser context and all its pages, recording why.
    ///
    /// Behaves as [`close`](Self::close), and the
    /// [`reason`](super::BrowserContextCloseOptions::reason) is reported by any
    /// operation the close interrupts.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Context has already been closed
    /// - Communication with browser process fails
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-close>
    #[tracing::instrument(level = "info", skip_all, fields(guid = %self.guid()))]
    pub async fn close_with_options(
        &self,
        options: super::BrowserContextCloseOptions,
    ) -> Result<()> {
        // Write the HAR the `record_har` option started, while the context
        // can still export it. Any failure returns before the context
        // closes, as in the other language bindings. A failed export keeps
        // the recording, so a second `close()` tries it again; once exported,
        // the driver has let the recording go, so a failed write is reported
        // once and a second `close()` closes the context.
        let mut option_har = self.option_har.lock().await;
        if let Some(recording) = option_har.as_ref() {
            let tracing = self.tracing().await?;
            let artifact = tracing.har_export(recording).await?;
            let written = match &artifact {
                Some(guid) => tracing.write_har(guid, recording).await,
                None => Ok(()),
            };
            *option_har = None;
            written?;
        }
        drop(option_har);

        // Unregister from Selectors coordinator so closed channels are not sent future messages.
        let selectors = self.connection().selectors();
        selectors.remove_context(self.channel());

        let result = self.channel().send_no_result("close", &options).await;
        // Mark as closed regardless of error (best-effort)
        self.is_closed.store(true, Ordering::Relaxed);
        result
    }

    /// Sets the default timeout for all operations in this browser context.
    ///
    /// This applies to all pages already open in this context as well as pages
    /// created subsequently. Pass `0` to disable timeouts.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Timeout in milliseconds
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-set-default-timeout>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn set_default_timeout(&self, timeout: f64) {
        self.default_timeout_ms
            .store(timeout.to_bits(), std::sync::atomic::Ordering::Relaxed);
        let pages: Vec<Page> = self.pages.lock().unwrap().clone();
        for page in pages {
            page.set_default_timeout(timeout).await;
        }
    }

    /// Sets the default timeout for navigation operations in this browser context.
    ///
    /// This applies to all pages already open in this context as well as pages
    /// created subsequently. Pass `0` to disable timeouts.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Timeout in milliseconds
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-set-default-navigation-timeout>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn set_default_navigation_timeout(&self, timeout: f64) {
        self.default_navigation_timeout_ms
            .store(timeout.to_bits(), std::sync::atomic::Ordering::Relaxed);
        let pages: Vec<Page> = self.pages.lock().unwrap().clone();
        for page in pages {
            page.set_default_navigation_timeout(timeout).await;
        }
    }

    /// Returns the context's current default action timeout in milliseconds.
    fn default_timeout_ms(&self) -> f64 {
        f64::from_bits(
            self.default_timeout_ms
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Returns the context's current default navigation timeout in milliseconds.
    fn default_navigation_timeout_ms(&self) -> f64 {
        f64::from_bits(
            self.default_navigation_timeout_ms
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Pauses the browser context.
    ///
    /// This pauses the execution of all pages in the context.
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn pause(&self) -> Result<()> {
        self.channel()
            .send_no_result("pause", serde_json::Value::Null)
            .await
    }

    /// Returns whether this browser context has been closed.
    ///
    /// Returns `true` after [`close()`](Self::close) has been called on this context, or after the
    /// context receives a close event from the server (e.g. when the browser is closed).
    ///
    /// Note: this reflects eventual state. If the context was closed by a server-initiated
    /// event, `is_closed()` becomes `true` only after the "close" event has been received
    /// and processed.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-is-closed>
    pub fn is_closed(&self) -> bool {
        self.is_closed.load(Ordering::Relaxed)
    }
}
