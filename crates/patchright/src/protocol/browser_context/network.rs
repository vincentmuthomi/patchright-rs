use super::BrowserContext;
use crate::error::{Error, Result};
use crate::protocol::Route;
use crate::protocol::route::UnrouteBehavior;
use crate::server::channel_owner::ChannelOwner;
use std::future::Future;

impl BrowserContext {
    /// PATCHRIGHT: whether the init-script injection route is installed.
    pub(crate) fn inject_route_installed(&self) -> bool {
        self.route_handlers
            .injecting
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// PATCHRIGHT: installs the init-script injection route on this context
    /// once (`installInjectRoute` upstream). See [`crate::patchright`].
    pub(crate) async fn install_inject_route(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        if self.route_handlers.injecting.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let installed = self
            .route("**/*", crate::patchright::inject_route_handler)
            .await;
        if installed.is_err() {
            self.route_handlers.injecting.store(false, Ordering::SeqCst);
        }
        installed
    }
}

/// Routes, network and WebSocket interception, HAR replay.
impl BrowserContext {
    /// Registers a route handler for context-level network interception.
    ///
    /// Routes registered on a context apply to all pages within the context.
    /// Page-level routes take precedence over context-level routes.
    ///
    /// # Arguments
    ///
    /// * `pattern` - URL pattern to match (supports glob patterns like "**/*.png").
    ///   A relative pattern such as `"/api/*"` resolves against the context's `base_url`.
    /// * `handler` - Async closure that handles the route
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-route>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid(), url = %pattern))]
    pub async fn route<F, Fut>(&self, pattern: &str, handler: F) -> Result<()>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.route_handlers.add(pattern, handler);
        self.route_handlers.sync(self.channel()).await
    }

    /// Fulfills matching requests from every page in this context using an
    /// in-process tower `Service`, such as an axum `Router` or a tower-http
    /// `ServeDir`, with no socket.
    ///
    /// Each request whose URL matches `pattern` is rebuilt as an
    /// `http::Request`, handed to a clone of `service`, and fulfilled with the
    /// response. The [`route_service`](crate::protocol::route_service) module
    /// documents what the service sees, the limits of route interception
    /// compared with a real listener, and how to wait on a wasm frontend.
    ///
    /// # Arguments
    ///
    /// * `pattern` - URL pattern to match (supports glob patterns like `"https://app.example/**"`)
    /// * `service` - Any [`RouteService`](crate::protocol::route_service::RouteService):
    ///   an axum `Router`, a tower-http `ServeDir`, a `tower::service_fn`; cloned per request
    ///
    /// # Errors
    ///
    /// Returns an error if network interception cannot be enabled. A service
    /// that fails at request time aborts that request and logs the error; it
    /// does not surface here.
    ///
    /// See: <https://playwright.dev/docs/mock>
    #[cfg(feature = "route-service")]
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid(), url = %pattern))]
    pub async fn route_service<S: crate::protocol::route_service::RouteService>(
        &self,
        pattern: &str,
        service: S,
    ) -> Result<()> {
        // Captured here, a hop away, so the service's view of the browser
        // (its engine, its cookie jar) does not depend on walking each
        // request's object chain.
        let context = Some(self.clone());
        self.route(pattern, move |route| {
            crate::protocol::route_service::fulfill_from_service(
                route,
                service.clone(),
                context.clone(),
            )
        })
        .await
    }

    /// Removes route handler(s) matching the given URL pattern.
    ///
    /// # Arguments
    ///
    /// * `pattern` - URL pattern to remove handlers for
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-unroute>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid(), url = %pattern))]
    pub async fn unroute(&self, pattern: &str) -> Result<()> {
        self.route_handlers.remove(pattern);
        self.route_handlers.sync(self.channel()).await
    }

    /// Removes all registered route handlers.
    ///
    /// # Arguments
    ///
    /// * `behavior` - What to do about handler invocations still running:
    ///   [`UnrouteBehavior::Wait`] returns only after they finish,
    ///   [`UnrouteBehavior::IgnoreErrors`] lets them fail without a log line,
    ///   and the default (`None`) neither waits nor silences them.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-unroute-all>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn unroute_all(&self, behavior: Option<UnrouteBehavior>) -> Result<()> {
        self.route_handlers.remove_all(behavior).await;
        self.route_handlers.sync(self.channel()).await
    }

    /// Replays network requests from a HAR file recorded previously.
    ///
    /// Requests matching `options.url` (or all requests if omitted) will be
    /// served from the archive for every page in this context.  Unmatched
    /// requests are either aborted or passed through depending on
    /// `options.not_found` (`"abort"` is the default).
    ///
    /// # Arguments
    ///
    /// * `har_path` - Path to the `.har` file on disk
    /// * `options` - Optional settings (url filter, not_found policy, update mode)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - `har_path` does not exist or cannot be read by the Playwright server
    /// - The Playwright server fails to open the archive
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-route-from-har>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn route_from_har(
        &self,
        har_path: &str,
        options: impl Into<Option<crate::protocol::RouteFromHarOptions>>,
    ) -> Result<()> {
        let options = options.into();
        let opts = options.unwrap_or_default();
        let not_found = opts.not_found.unwrap_or_else(|| "abort".to_string());
        let url_filter = opts.url.clone();

        let abs_path = std::path::Path::new(har_path).canonicalize().map_err(|e| {
            Error::InvalidPath(format!(
                "route_from_har: cannot resolve '{}': {}",
                har_path, e
            ))
        })?;
        let abs_str = abs_path.to_string_lossy().into_owned();

        let connection = self.connection();
        let local_utils = {
            let all = connection.all_objects_sync();
            all.into_iter()
                .find(|o| o.type_name() == "LocalUtils")
                .and_then(|o| {
                    o.as_any()
                        .downcast_ref::<crate::protocol::LocalUtils>()
                        .cloned()
                })
                .ok_or_else(|| {
                    Error::ProtocolError(
                        "route_from_har: LocalUtils not found in connection registry".to_string(),
                    )
                })?
        };

        let har_id = local_utils.har_open(&abs_str).await?;

        let pattern = url_filter.unwrap_or_else(|| "**/*".to_string());

        let har_id_clone = har_id.clone();
        let local_utils_clone = local_utils.clone();
        let not_found_clone = not_found.clone();

        self.route(&pattern, move |route| {
            let har_id = har_id_clone.clone();
            let local_utils = local_utils_clone.clone();
            let not_found = not_found_clone.clone();
            async move {
                let request = route.request();
                let req_url = request.url();
                let req_method = request.method();

                let headers = crate::protocol::route_params::header_array(request.header_pairs());

                let lookup = local_utils
                    .har_lookup(
                        &har_id,
                        &req_url,
                        &req_method,
                        headers,
                        None,
                        request.is_navigation_request(),
                    )
                    .await;

                match lookup {
                    Err(e) => {
                        tracing::warn!("har_lookup error for {}: {}", req_url, e);
                        route.continue_(None).await
                    }
                    Ok(result) => match result.action.as_str() {
                        "redirect" => {
                            let redirect_url = result.redirect_url.unwrap_or_default();
                            let opts = crate::protocol::ContinueOptions::builder()
                                .url(redirect_url)
                                .build();
                            route.continue_(Some(opts)).await
                        }
                        "fulfill" => {
                            route
                                .fulfill(Some(crate::protocol::route_params::har_fulfill_options(
                                    result.status,
                                    result.body.as_deref(),
                                    result.headers.as_deref(),
                                )))
                                .await
                        }
                        _ => {
                            if not_found == "fallback" {
                                route.fallback(None).await
                            } else {
                                route.abort(None).await
                            }
                        }
                    },
                }
            }
        })
        .await
    }

    /// Intercepts WebSocket connections matching the given URL pattern for all pages in this context.
    ///
    /// When a WebSocket connection from any page in this context matches `url`,
    /// the `handler` is called with a [`WebSocketRoute`](crate::protocol::WebSocketRoute) object.
    /// The handler must call [`connect_to_server`](crate::protocol::WebSocketRoute::connect_to_server)
    /// to forward the connection to the real server, or
    /// [`close`](crate::protocol::WebSocketRoute::close) to terminate it.
    ///
    /// # Arguments
    ///
    /// * `url` — URL glob pattern (e.g. `"**"` for every socket, or `"wss://example.com/ws"`).
    ///   A relative pattern such as `"/ws"` resolves against the context's `base_url`,
    ///   read as `ws://` or `wss://`.
    /// * `handler` — Async closure receiving a `WebSocketRoute`.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call to enable interception fails.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-route-web-socket>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid(), url = %url))]
    pub async fn route_web_socket<F, Fut>(&self, url: &str, handler: F) -> Result<()>
    where
        F: Fn(crate::protocol::WebSocketRoute) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.ws_route_handlers.add(url, handler);
        self.ws_route_handlers.sync(self.channel()).await
    }

    /// The `base_url` this context was created with, which route patterns
    /// and the URLs given to `wait_for_url` and `to_have_url` resolve
    /// against.
    pub(crate) fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// Runs this context's route handlers on a request: one the driver
    /// reported to the context, or one its page's handlers all fell back on,
    /// as upstream's page hands it to its context. Continues the request if
    /// these fall back too.
    pub(crate) async fn on_route_event(&self, route: Route) {
        if self.route_handlers.run(&route, "Context").await {
            return;
        }
        // Every handler fell back, or none matched: send the request on with
        // the overrides its fallbacks recorded.
        route.continue_unhandled().await;
    }
}
