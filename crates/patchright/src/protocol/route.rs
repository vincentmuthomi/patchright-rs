// Route protocol object
//
// Represents a route handler for network interception.
// Routes are created when page.route() or context.route() matches a request.
//
// See: https://playwright.dev/docs/api/class-route

use super::route_params::RouteState;
use crate::error::Result;
use crate::protocol::Request;
use crate::protocol::api_request_context::{APIRequestContext, InnerFetchOptions};
use crate::server::channel_owner::{ChannelOwner, ChannelOwnerImpl, ParentOrConnection};
use crate::server::connection::downcast_parent;
use serde_json::{Value, json};
use std::any::Any;
use std::sync::{Arc, Mutex};

/// Route represents a network route handler.
///
/// Routes allow intercepting, aborting, continuing, or fulfilling network requests.
///
/// See: <https://playwright.dev/docs/api/class-route>
#[derive(Clone)]
pub struct Route {
    base: ChannelOwnerImpl,
    /// Whether a handler has taken the route over, whether the driver has
    /// accepted an answer, and what the running handler has done short of
    /// answering.
    state: Arc<Mutex<RouteState>>,
    /// The overrides `fallback()` and `continue_()` calls have recorded,
    /// applied when the request is finally continued. The cell is the
    /// request's own, so `request()` reports them; holding it here keeps
    /// them even when the request cannot be reached.
    fallback_overrides: Arc<Mutex<Option<ContinueOptions>>>,
    /// APIRequestContext for performing fetch operations.
    /// Set by the route event dispatcher (Page or BrowserContext).
    api_request_context: Arc<Mutex<Option<APIRequestContext>>>,
}

impl Route {
    /// Creates a new Route from protocol initialization
    ///
    /// This is called by the object factory when the server sends a `__create__` message
    /// for a Route object.
    pub fn new(
        parent: Arc<dyn ChannelOwner>,
        type_name: String,
        guid: Arc<str>,
        initializer: Value,
    ) -> Result<Self> {
        let fallback_overrides = parent
            .as_any()
            .downcast_ref::<Request>()
            .map(Request::fallback_overrides_cell)
            .unwrap_or_default();
        let base = ChannelOwnerImpl::new(
            ParentOrConnection::Parent(parent),
            type_name,
            guid,
            initializer,
        );

        Ok(Self {
            base,
            state: Arc::default(),
            fallback_overrides,
            api_request_context: Arc::new(Mutex::new(None)),
        })
    }

    /// Returns whether a handler took the route over.
    ///
    /// Returns `false` while every handler so far fell back, explicitly or by
    /// returning without answering or fetching, indicating the next matching
    /// handler should be tried.
    pub(super) fn was_handled(&self) -> bool {
        self.state.lock().unwrap().handled()
    }

    /// Settles the handler that just returned `Ok`: marks the route handled
    /// if the handler kept it, and returns whether it did. `false` means the
    /// next handler is tried, and it starts with what this one did cleared.
    pub(super) fn settle_handler(&self) -> bool {
        self.state.lock().unwrap().settle()
    }

    /// Aborts the request of a handler that returned an error, unless the
    /// driver had already accepted an answer to it.
    ///
    /// A `fetch()` sends no answer, nor does a command the driver's validator
    /// rejects. The abort is sent even if the handler fell back. One the
    /// route itself refuses still marks it handled upstream, so this abort is
    /// refused too and the error says the request is stuck.
    pub(super) async fn abort_unanswered(&self) -> Result<()> {
        if !self.state.lock().unwrap().take_over_unanswered() {
            return Ok(());
        }
        self.send_abort(Some("failed")).await
    }

    /// Sends a command that answers the request, noting the answer once the
    /// driver accepts it.
    async fn answer(&self, method: &str, params: Value) -> Result<()> {
        self.channel()
            .send::<_, serde_json::Value>(method, params)
            .await?;
        self.state.lock().unwrap().answer_accepted();
        Ok(())
    }

    /// Sets the APIRequestContext for this route, enabling `fetch()`.
    ///
    /// Called by the route event dispatcher (Page or BrowserContext) when
    /// dispatching the route to a handler.
    pub(crate) fn set_api_request_context(&self, ctx: APIRequestContext) {
        *self.api_request_context.lock().unwrap() = Some(ctx);
    }

    /// Returns the request that is being routed.
    ///
    /// See: <https://playwright.dev/docs/api/class-route#route-request>
    pub fn request(&self) -> Request {
        // The Route's parent is the Request object
        if let Some(request) = downcast_parent::<Request>(self) {
            return request;
        }

        // Fallback: Create a stub Request from initializer data
        // This should rarely happen in practice
        let request_data = self
            .initializer()
            .get("request")
            .cloned()
            .unwrap_or_else(|| {
                serde_json::json!({
                    "url": "",
                    "method": "GET"
                })
            });

        let parent = self
            .parent()
            .unwrap_or_else(|| Arc::new(self.clone()) as Arc<dyn ChannelOwner>);

        let request_guid = request_data
            .get("guid")
            .and_then(|v| v.as_str())
            .unwrap_or("request-stub");

        Request::new(
            parent,
            "Request".to_string(),
            Arc::from(request_guid),
            request_data,
        )
        .expect("stub Request construction cannot fail")
        .with_fallback_overrides(self.fallback_overrides.clone())
    }

    /// Aborts the route's request.
    ///
    /// # Arguments
    ///
    /// * `error_code` - Optional error code (default: "failed")
    ///
    /// Available error codes:
    /// - "aborted" - User-initiated cancellation
    /// - "accessdenied" - Permission denied
    /// - "addressunreachable" - Host unreachable
    /// - "blockedbyclient" - Client blocked request
    /// - "connectionaborted", "connectionclosed", "connectionfailed", "connectionrefused", "connectionreset"
    /// - "internetdisconnected"
    /// - "namenotresolved"
    /// - "timedout"
    /// - "failed" - Generic error (default)
    ///
    /// # Errors
    ///
    /// Returns an error if this handler already called `fallback()`, if the
    /// driver already accepted an answer, or if the driver rejects the
    /// command.
    ///
    /// See: <https://playwright.dev/docs/api/class-route#route-abort>
    pub async fn abort(&self, error_code: Option<&str>) -> Result<()> {
        self.state.lock().unwrap().begin_answer()?;
        self.send_abort(error_code).await
    }

    /// Sends the driver's `abort` command.
    async fn send_abort(&self, error_code: Option<&str>) -> Result<()> {
        let params = json!({
            "errorCode": error_code.unwrap_or("failed")
        });

        self.answer("abort", params).await
    }

    /// Continues the route's request with optional modifications.
    ///
    /// This is a final action — no other route handlers will run for this request.
    /// Use `fallback()` instead if you want subsequent handlers to have a chance.
    ///
    /// # Arguments
    ///
    /// * `overrides` - Optional modifications to apply to the request
    ///
    /// Overrides recorded by earlier handlers' `fallback()` calls apply too,
    /// with these taking precedence field by field.
    /// [`request()`](Self::request) reports the result.
    ///
    /// # Errors
    ///
    /// Returns an error if this handler already called `fallback()`, if the
    /// driver already accepted an answer, or if the driver rejects the
    /// command.
    ///
    /// See: <https://playwright.dev/docs/api/class-route#route-continue>
    pub async fn continue_(&self, overrides: Option<ContinueOptions>) -> Result<()> {
        self.state.lock().unwrap().begin_answer()?;
        self.record_overrides(overrides);
        let overrides = self.fallback_overrides.lock().unwrap().clone();
        self.continue_internal(overrides, false).await
    }

    /// Passes the request to the next matching handler, recording `overrides`
    /// for it.
    ///
    /// Unlike `continue_()`, `fallback()` sends nothing to the browser. The
    /// next matching handler sees the request (a page's handlers come before
    /// its context's), and when none is left the request is continued once,
    /// with every fallback's overrides applied (a later one wins field by
    /// field). This enables middleware-like handler composition where several
    /// handlers inspect and modify one request.
    ///
    /// The overrides land on [`request()`](Self::request) at once, so a later
    /// handler reads the URL, method, headers and post data as overridden, and
    /// is chosen by matching its pattern against the overridden URL.
    ///
    /// # Arguments
    ///
    /// * `overrides` - Optional modifications to apply to the request
    ///
    /// # Errors
    ///
    /// Returns an error if the route has already been handled, or if this
    /// handler already called `fallback()`.
    ///
    /// See: <https://playwright.dev/docs/api/class-route#route-fallback>
    pub async fn fallback(&self, overrides: Option<ContinueOptions>) -> Result<()> {
        self.state.lock().unwrap().fall_back()?;
        self.record_overrides(overrides);
        Ok(())
    }

    /// Lays `overrides` over those recorded so far.
    fn record_overrides(&self, overrides: Option<ContinueOptions>) {
        let mut recorded = self.fallback_overrides.lock().unwrap();
        *recorded = super::route_params::merge_overrides(recorded.take(), overrides);
    }

    /// Continues a request no handler handled, with every fallback's
    /// overrides applied. Called by the dispatcher once the page's and the
    /// context's handlers have all fallen back; a no-op if one handled it.
    /// No caller can act on a failure, so it is logged here.
    pub(crate) async fn continue_unhandled(&self) {
        if !self.state.lock().unwrap().take_over_unhandled() {
            return;
        }
        let overrides = self.fallback_overrides.lock().unwrap().clone();
        if let Err(e) = self.continue_internal(overrides, true).await {
            tracing::debug!("continuing a route no handler handled failed: {}", e);
        }
    }

    /// Sends the driver's `continue` command.
    async fn continue_internal(
        &self,
        overrides: Option<ContinueOptions>,
        is_fallback: bool,
    ) -> Result<()> {
        let params = super::route_params::continue_params(overrides, is_fallback);

        self.answer("continue", params).await
    }

    /// Fulfills the route's request with a custom response.
    ///
    /// # Arguments
    ///
    /// * `options` - Response configuration (status, headers, body, etc.)
    ///
    /// # Errors
    ///
    /// Returns an error if this handler already called `fallback()`, if the
    /// driver already accepted an answer, or if the driver rejects the
    /// command, for example when the page has closed.
    ///
    /// See: <https://playwright.dev/docs/api/class-route#route-fulfill>
    pub async fn fulfill(&self, options: impl Into<Option<FulfillOptions>>) -> Result<()> {
        self.state.lock().unwrap().begin_answer()?;
        let params = super::route_params::fulfill_params(options.into().unwrap_or_default());

        self.answer("fulfill", params).await
    }

    /// Performs the request and fetches result without fulfilling it, so that the
    /// response can be modified and then fulfilled.
    ///
    /// Delegates to `APIRequestContext.inner_fetch()`. The URL, method, headers
    /// and post data each default to the request's, as earlier handlers'
    /// `fallback()` calls left it.
    ///
    /// The handler then answers the request with `fulfill()`, `continue_()`,
    /// or `abort()`, or passes it on with `fallback()`. One that returns
    /// without doing either keeps the route, so no later handler runs and the
    /// request waits; one that returns an error before answering has its
    /// request aborted.
    ///
    /// # Arguments
    ///
    /// * `options` - Optional overrides for the fetch request
    ///
    /// # Errors
    ///
    /// Returns an error if the route was not dispatched through a context
    /// with a request context, or if the fetch itself fails.
    ///
    /// See: <https://playwright.dev/docs/api/class-route#route-fetch>
    pub async fn fetch(&self, options: impl Into<Option<FetchOptions>>) -> Result<FetchResponse> {
        let options = options.into();

        let api_ctx = self
            .api_request_context
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| {
                crate::error::Error::ProtocolError(
                    "No APIRequestContext available for route.fetch(). \
                     This can happen if the route was not dispatched through \
                     a BrowserContext with an associated request context."
                        .to_string(),
                )
            })?;

        let request = self.request();
        let opts = options.unwrap_or_default();
        let url = opts.url.unwrap_or_else(|| request.url());
        let post_data = super::route_params::post_data_bytes(
            opts.post_data.as_deref(),
            opts.post_data_bytes.as_deref(),
        )
        .map(<[u8]>::to_vec)
        .or_else(|| request.post_data_buffer());

        let inner_opts = InnerFetchOptions {
            method: opts.method.or_else(|| Some(request.method())),
            headers: opts.headers.or_else(|| Some(request.headers())),
            post_data: None,
            post_data_bytes: post_data,
            max_redirects: opts.max_redirects,
            max_retries: opts.max_retries,
            timeout: opts.timeout,
        };

        // From here the request may reach the server, even if the fetch then
        // fails, so the handler keeps the route unless it falls back.
        self.state.lock().unwrap().fetch();
        api_ctx.inner_fetch(&url, Some(inner_opts)).await
    }
}

/// Behavior when removing route handlers via `unroute_all()`.
///
/// See: <https://playwright.dev/docs/api/class-page#page-unroute-all>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnrouteBehavior {
    /// Wait for in-flight handlers to complete before removing
    Wait,
    /// Stop handlers and ignore any errors they throw
    IgnoreErrors,
    /// Default behavior (does not wait, does not ignore errors)
    Default,
}

/// Response from `route.fetch()`, allowing inspection and modification before fulfillment.
///
/// See: <https://playwright.dev/docs/api/class-apiresponse>
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FetchResponse {
    /// HTTP status code
    pub status: u16,
    /// HTTP status text
    pub status_text: String,
    /// Response headers as name-value pairs
    pub headers: Vec<(String, String)>,
    /// Response body as bytes
    pub body: Vec<u8>,
}

impl FetchResponse {
    /// Returns the HTTP status code
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Returns the status text
    pub fn status_text(&self) -> &str {
        &self.status_text
    }

    /// Returns response headers
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Returns the response body as bytes
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Returns the response body as text
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).map_err(|e| {
            crate::error::Error::ProtocolError(format!("Response body is not valid UTF-8: {}", e))
        })
    }

    /// Returns the response body parsed as JSON
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| {
            crate::error::Error::ProtocolError(format!("Failed to parse response JSON: {}", e))
        })
    }

    /// Returns true if status is in 200-299 range
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Options for continuing a request with modifications.
///
/// Allows modifying headers, method, post data, and URL when continuing a route.
/// Used by both `continue_()` and `fallback()`.
///
/// See: <https://playwright.dev/docs/api/class-route#route-continue>
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ContinueOptions {
    /// Modified request headers
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// Modified request method (GET, POST, etc.)
    pub method: Option<String>,
    /// Modified POST data as string
    pub post_data: Option<String>,
    /// Modified POST data as bytes
    pub post_data_bytes: Option<Vec<u8>>,
    /// Modified request URL (must have same protocol)
    pub url: Option<String>,
    /// PATCHRIGHT: asks the patched driver to inject the page's init scripts
    /// (and exposed bindings) into this document response. Set by the
    /// injection route patchright installs; see `crate::patchright`.
    pub(crate) patchright_init_script: Option<bool>,
}

impl ContinueOptions {
    /// Creates a new builder for ContinueOptions
    pub fn builder() -> ContinueOptionsBuilder {
        ContinueOptionsBuilder::default()
    }
}

/// Builder for ContinueOptions
#[derive(Debug, Clone, Default)]
pub struct ContinueOptionsBuilder {
    headers: Option<std::collections::HashMap<String, String>>,
    method: Option<String>,
    post_data: Option<String>,
    post_data_bytes: Option<Vec<u8>>,
    url: Option<String>,
}

impl ContinueOptionsBuilder {
    /// Sets the request headers
    pub fn headers(mut self, headers: std::collections::HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Sets the request method
    pub fn method(mut self, method: String) -> Self {
        self.method = Some(method);
        self
    }

    /// Sets the POST data as a string
    pub fn post_data(mut self, post_data: String) -> Self {
        self.post_data = Some(post_data);
        self.post_data_bytes = None; // Clear bytes if setting string
        self
    }

    /// Sets the POST data as bytes
    pub fn post_data_bytes(mut self, post_data_bytes: Vec<u8>) -> Self {
        self.post_data_bytes = Some(post_data_bytes);
        self.post_data = None; // Clear string if setting bytes
        self
    }

    /// Sets the request URL (must have same protocol as original)
    pub fn url(mut self, url: String) -> Self {
        self.url = Some(url);
        self
    }

    /// Builds the ContinueOptions
    pub fn build(self) -> ContinueOptions {
        ContinueOptions {
            headers: self.headers,
            method: self.method,
            post_data: self.post_data,
            post_data_bytes: self.post_data_bytes,
            url: self.url,
            patchright_init_script: None,
        }
    }
}

/// Options for fulfilling a route with a custom response.
///
/// See: <https://playwright.dev/docs/api/class-route#route-fulfill>
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FulfillOptions {
    /// HTTP status code (default: 200)
    pub status: Option<u16>,
    /// Response headers
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// Response body as bytes
    pub body: Option<Vec<u8>>,
    /// Content-Type header value
    pub content_type: Option<String>,
}

impl FulfillOptions {
    /// Creates a new FulfillOptions builder
    pub fn builder() -> FulfillOptionsBuilder {
        FulfillOptionsBuilder::default()
    }
}

/// Builder for FulfillOptions
#[derive(Debug, Clone, Default)]
pub struct FulfillOptionsBuilder {
    status: Option<u16>,
    headers: Option<std::collections::HashMap<String, String>>,
    body: Option<Vec<u8>>,
    content_type: Option<String>,
}

impl FulfillOptionsBuilder {
    /// Sets the HTTP status code
    pub fn status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Sets the response headers
    pub fn headers(mut self, headers: std::collections::HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Sets the response body from bytes
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// Sets the response body from a string
    pub fn body_string(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into().into_bytes());
        self
    }

    /// Sets the response body from JSON (automatically sets content-type to application/json)
    pub fn json(mut self, value: &impl serde::Serialize) -> Result<Self> {
        let json_str = serde_json::to_string(value).map_err(|e| {
            crate::error::Error::ProtocolError(format!("JSON serialization failed: {}", e))
        })?;
        self.body = Some(json_str.into_bytes());
        self.content_type = Some("application/json".to_string());
        Ok(self)
    }

    /// Sets the Content-Type header
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Builds the FulfillOptions
    pub fn build(self) -> FulfillOptions {
        FulfillOptions {
            status: self.status,
            headers: self.headers,
            body: self.body,
            content_type: self.content_type,
        }
    }
}

/// Options for fetching a route's request.
///
/// See: <https://playwright.dev/docs/api/class-route#route-fetch>
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FetchOptions {
    /// Modified request headers
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// Modified request method (GET, POST, etc.)
    pub method: Option<String>,
    /// Modified POST data as string
    pub post_data: Option<String>,
    /// Modified POST data as bytes
    pub post_data_bytes: Option<Vec<u8>>,
    /// Modified request URL
    pub url: Option<String>,
    /// Maximum number of redirects to follow (default: 20)
    pub max_redirects: Option<u32>,
    /// Maximum number of retries (default: 0)
    pub max_retries: Option<u32>,
    /// Request timeout in milliseconds
    pub timeout: Option<f64>,
}

impl FetchOptions {
    /// Creates a new FetchOptions builder
    pub fn builder() -> FetchOptionsBuilder {
        FetchOptionsBuilder::default()
    }
}

/// Builder for FetchOptions
#[derive(Debug, Clone, Default)]
pub struct FetchOptionsBuilder {
    headers: Option<std::collections::HashMap<String, String>>,
    method: Option<String>,
    post_data: Option<String>,
    post_data_bytes: Option<Vec<u8>>,
    url: Option<String>,
    max_redirects: Option<u32>,
    max_retries: Option<u32>,
    timeout: Option<f64>,
}

impl FetchOptionsBuilder {
    /// Sets the request headers
    pub fn headers(mut self, headers: std::collections::HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Sets the request method
    pub fn method(mut self, method: String) -> Self {
        self.method = Some(method);
        self
    }

    /// Sets the POST data as a string
    pub fn post_data(mut self, post_data: String) -> Self {
        self.post_data = Some(post_data);
        self.post_data_bytes = None;
        self
    }

    /// Sets the POST data as bytes
    pub fn post_data_bytes(mut self, post_data_bytes: Vec<u8>) -> Self {
        self.post_data_bytes = Some(post_data_bytes);
        self.post_data = None;
        self
    }

    /// Sets the request URL
    pub fn url(mut self, url: String) -> Self {
        self.url = Some(url);
        self
    }

    /// Sets the maximum number of redirects to follow
    pub fn max_redirects(mut self, n: u32) -> Self {
        self.max_redirects = Some(n);
        self
    }

    /// Sets the maximum number of retries
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = Some(n);
        self
    }

    /// Sets the request timeout in milliseconds
    pub fn timeout(mut self, ms: f64) -> Self {
        self.timeout = Some(ms);
        self
    }

    /// Builds the FetchOptions
    pub fn build(self) -> FetchOptions {
        FetchOptions {
            headers: self.headers,
            method: self.method,
            post_data: self.post_data,
            post_data_bytes: self.post_data_bytes,
            url: self.url,
            max_redirects: self.max_redirects,
            max_retries: self.max_retries,
            timeout: self.timeout,
        }
    }
}

impl ChannelOwner for Route {
    fn guid(&self) -> &str {
        self.base.guid()
    }

    fn type_name(&self) -> &str {
        self.base.type_name()
    }

    fn parent(&self) -> Option<Arc<dyn ChannelOwner>> {
        self.base.parent()
    }

    fn connection(&self) -> Arc<dyn crate::server::connection::ConnectionLike> {
        self.base.connection()
    }

    fn initializer(&self) -> &Value {
        self.base.initializer()
    }

    fn channel(&self) -> &crate::server::channel::Channel {
        self.base.channel()
    }

    fn dispose(&self, reason: crate::server::channel_owner::DisposeReason) {
        self.base.dispose(reason)
    }

    fn adopt(&self, child: Arc<dyn ChannelOwner>) {
        self.base.adopt(child)
    }

    fn add_child(&self, guid: Arc<str>, child: Arc<dyn ChannelOwner>) {
        self.base.add_child(guid, child)
    }

    fn remove_child(&self, guid: &str) {
        self.base.remove_child(guid)
    }

    fn on_event(&self, _method: &str, _params: Value) {
        // Route events will be handled in future phases
    }

    fn was_collected(&self) -> bool {
        self.base.was_collected()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("guid", &self.guid())
            .field("request", &self.request().guid())
            .finish()
    }
}
