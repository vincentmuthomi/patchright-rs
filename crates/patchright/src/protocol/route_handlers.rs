//! The route and WebSocket route handlers a page or context registered, and
//! the bookkeeping that registering, unrouting and dispatching do on them.
//! `Page` and `BrowserContext` share these, so a routing fix lands once.

use super::event_registry::Handler;
use super::glob::{GlobMatcher, to_web_socket_base_url};
use super::in_flight::{InFlight, settle_removed};
use super::route::{Route, UnrouteBehavior};
use super::web_socket_route::WebSocketRoute;
use crate::error::Result;
use crate::server::channel::Channel;
use crate::server::connection::{ConnectionExt, ConnectionLike};
use serde_json::{Value, json};
use std::future::Future;
use std::sync::{Arc, Mutex};

/// A route handler a page or context registered.
#[derive(Clone)]
struct RouteHandlerEntry {
    pattern: String,
    /// `pattern` resolved against the context's base URL and compiled once;
    /// `None` for a malformed glob, which matches nothing, as the driver
    /// treats it.
    matcher: Option<GlobMatcher>,
    handler: Handler<Route>,
    /// Invocations of `handler` still running, for `unroute_all`.
    in_flight: Arc<InFlight>,
}

/// The route handlers of one page or context, in registration order.
#[derive(Default)]
pub(crate) struct RouteHandlers {
    /// The context's `base_url`, which every pattern resolves against.
    base_url: Option<String>,
    entries: Mutex<Vec<RouteHandlerEntry>>,
    /// PATCHRIGHT: whether patchright's init-script injection route is
    /// installed on this page or context (`routeInjecting` upstream).
    pub(crate) injecting: std::sync::atomic::AtomicBool,
}

impl RouteHandlers {
    pub(crate) fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url.map(str::to_string),
            entries: Mutex::default(),
            injecting: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Registers `handler` for `pattern`, which is matched as the driver
    /// matches it: resolved against the context's `base_url`.
    pub(crate) fn add<F, Fut>(&self, pattern: &str, handler: F)
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.entries.lock().unwrap().push(RouteHandlerEntry {
            pattern: pattern.to_string(),
            matcher: GlobMatcher::for_url(self.base_url.as_deref(), pattern),
            handler: Arc::new(move |route| Box::pin(handler(route))),
            in_flight: Arc::default(),
        });
    }

    /// Removes every handler registered with exactly `pattern`.
    pub(crate) fn remove(&self, pattern: &str) {
        self.entries
            .lock()
            .unwrap()
            .retain(|entry| entry.pattern != pattern);
    }

    /// Removes every handler, then applies `behavior` to the invocations of
    /// them still running.
    pub(crate) async fn remove_all(&self, behavior: Option<UnrouteBehavior>) {
        let removed = std::mem::take(&mut *self.entries.lock().unwrap());
        settle_removed(removed.iter().map(|entry| &entry.in_flight), behavior).await;
    }

    /// The `setNetworkInterceptionPatterns` parameters for the handlers
    /// registered now.
    fn interception_patterns(&self) -> Value {
        interception_patterns(
            self.entries
                .lock()
                .unwrap()
                .iter()
                .map(|entry| &entry.pattern),
        )
    }

    /// Tells the driver which requests to report: those matching a pattern
    /// registered now. Called after every change to the list.
    pub(crate) async fn sync(&self, channel: &Channel) -> Result<()> {
        channel
            .send_no_result(
                "setNetworkInterceptionPatterns",
                self.interception_patterns(),
            )
            .await
    }

    /// Runs the handlers whose pattern matches the request, last registered
    /// first, until one keeps the route: answers it, or fetches without
    /// falling back. A handler that returns an error has the request aborted
    /// unless it was answered, and no later handler runs. Returns whether a
    /// handler kept the route; `false` means every match fell back, and the
    /// caller passes the request on. `scope` names the page or context in
    /// the log.
    ///
    /// The list is copied first, so a handler may route or unroute without
    /// deadlocking, and one added meanwhile waits for the next request.
    #[must_use]
    pub(crate) async fn run(&self, route: &Route, scope: &str) -> bool {
        let handlers = self.entries.lock().unwrap().clone();
        // Each read sees the overrides a fallback recorded, so a rewritten URL
        // changes which handlers come after it.
        let request = route.request();
        for entry in handlers.iter().rev() {
            if !matches(&entry.matcher, &request.url()) {
                continue;
            }
            let running = entry.in_flight.enter();
            let outcome = (entry.handler)(route.clone()).await;
            drop(running);
            if let Err(e) = outcome {
                let quiet = entry.in_flight.ignores_errors();
                if !quiet {
                    tracing::warn!(scope, "Route handler error: {}", e);
                }
                // A handler that failed before answering leaves the request
                // pending; abort it so the browser sees a failed request
                // instead of waiting out its timeout.
                if let Err(abort_error) = route.abort_unanswered().await
                    && !quiet
                {
                    tracing::warn!(
                        scope,
                        "aborting the unhandled route failed too: {}",
                        abort_error
                    );
                }
                break;
            }
            if route.settle_handler() {
                break;
            }
        }
        route.was_handled()
    }
}

/// A WebSocket route handler a page or context registered.
#[derive(Clone)]
struct WebSocketRouteHandlerEntry {
    pattern: String,
    /// As in [`RouteHandlerEntry`], with an `http(s)` base read as `ws(s)`.
    matcher: Option<GlobMatcher>,
    handler: Handler<WebSocketRoute>,
}

/// The WebSocket route handlers of one page or context, in registration
/// order.
#[derive(Default)]
pub(crate) struct WebSocketRouteHandlers {
    /// The context's `base_url` read as `ws(s)`, which every pattern
    /// resolves against.
    base_url: Option<String>,
    entries: Mutex<Vec<WebSocketRouteHandlerEntry>>,
}

impl WebSocketRouteHandlers {
    pub(crate) fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url.map(to_web_socket_base_url),
            entries: Mutex::default(),
        }
    }

    /// Registers `handler` for `pattern`, which is matched as the driver
    /// matches it: resolved against the context's `base_url`.
    pub(crate) fn add<F, Fut>(&self, pattern: &str, handler: F)
    where
        F: Fn(WebSocketRoute) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.entries
            .lock()
            .unwrap()
            .push(WebSocketRouteHandlerEntry {
                pattern: pattern.to_string(),
                matcher: GlobMatcher::for_url(self.base_url.as_deref(), pattern),
                handler: Arc::new(move |route| Box::pin(handler(route))),
            });
    }

    /// The `setWebSocketInterceptionPatterns` parameters for the handlers
    /// registered now.
    fn interception_patterns(&self) -> Value {
        interception_patterns(
            self.entries
                .lock()
                .unwrap()
                .iter()
                .map(|entry| &entry.pattern),
        )
    }

    /// Tells the driver which sockets to report: those matching a pattern
    /// registered now. Called after every change to the list.
    pub(crate) async fn sync(&self, channel: &Channel) -> Result<()> {
        channel
            .send_no_result(
                "setWebSocketInterceptionPatterns",
                self.interception_patterns(),
            )
            .await
    }

    /// The handler for a socket to `url`: the last registered whose pattern
    /// matches it.
    fn find(&self, url: &str) -> Option<WebSocketRouteHandlerEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|entry| matches(&entry.matcher, url))
            .cloned()
    }

    /// Fetches the `WebSocketRoute` a `webSocketRoute` event announced and
    /// runs it through these handlers, on a task of its own. `scope` names
    /// the page or context in the log.
    pub(crate) fn dispatch(
        self: &Arc<Self>,
        connection: Arc<dyn ConnectionLike>,
        guid: &str,
        scope: &'static str,
    ) {
        let handlers = Arc::clone(self);
        let guid = guid.to_string();
        crate::task::spawn(async move {
            match connection.get_typed::<WebSocketRoute>(&guid).await {
                Ok(route) => handlers.run(route, scope).await,
                Err(e) => tracing::warn!("Failed to get WebSocketRoute object: {}", e),
            }
        });
    }

    /// Runs the matching handler on `route`, then has the page's socket
    /// report itself open if the handler neither connected nor closed it. A
    /// socket no handler matches is left alone.
    async fn run(&self, route: WebSocketRoute, scope: &str) {
        let Some(entry) = self.find(route.url()) else {
            return;
        };
        if let Err(e) = (entry.handler)(route.clone()).await {
            tracing::error!(scope, "webSocketRoute handler error: {}", e);
        }
        route.after_handle().await;
    }
}

/// The `{"patterns": [{"glob": ...}]}` parameters both interception commands
/// take.
fn interception_patterns<'a>(patterns: impl Iterator<Item = &'a String>) -> Value {
    let globs: Vec<Value> = patterns.map(|glob| json!({ "glob": glob })).collect();
    json!({ "patterns": globs })
}

/// Whether a handler's compiled pattern matches `url`.
fn matches(matcher: &Option<GlobMatcher>, url: &str) -> bool {
    matcher.as_ref().is_some_and(|m| m.matches(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_patterns(patterns: &[&str]) -> RouteHandlers {
        let handlers = RouteHandlers::default();
        for pattern in patterns {
            handlers.add(pattern, |_route| async { Ok(()) });
        }
        handlers
    }

    fn in_flight(handlers: &RouteHandlers) -> Vec<Arc<InFlight>> {
        handlers
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|entry| entry.in_flight.clone())
            .collect()
    }

    #[test]
    fn interception_patterns_lists_each_glob_in_registration_order() {
        let handlers = with_patterns(&["**/a", "**/b", "**/a"]);
        assert_eq!(
            handlers.interception_patterns(),
            json!({ "patterns": [{ "glob": "**/a" }, { "glob": "**/b" }, { "glob": "**/a" }] })
        );
    }

    #[test]
    fn remove_drops_every_handler_with_that_pattern_and_keeps_the_rest() {
        let handlers = with_patterns(&["**/a", "**/b", "**/a"]);
        handlers.remove("**/a");
        assert_eq!(
            handlers.interception_patterns(),
            json!({ "patterns": [{ "glob": "**/b" }] })
        );
    }

    #[tokio::test]
    async fn remove_all_empties_the_list_and_settles_the_removed_handlers() {
        let handlers = with_patterns(&["**/a", "**/b"]);
        let removed = in_flight(&handlers);
        handlers
            .remove_all(Some(UnrouteBehavior::IgnoreErrors))
            .await;
        assert_eq!(handlers.interception_patterns(), json!({ "patterns": [] }));
        assert!(
            removed.iter().all(|handler| handler.ignores_errors()),
            "each removed handler is told to keep its errors quiet"
        );
    }

    #[tokio::test]
    async fn remove_all_by_default_leaves_the_removed_handlers_reporting_errors() {
        let handlers = with_patterns(&["**/a"]);
        let removed = in_flight(&handlers);
        handlers.remove_all(None).await;
        assert_eq!(handlers.interception_patterns(), json!({ "patterns": [] }));
        assert!(!removed[0].ignores_errors());
    }

    fn with_ws_patterns(patterns: &[&str]) -> WebSocketRouteHandlers {
        let handlers = WebSocketRouteHandlers::default();
        for pattern in patterns {
            handlers.add(pattern, |_route| async { Ok(()) });
        }
        handlers
    }

    fn found(handlers: &WebSocketRouteHandlers, url: &str) -> Option<String> {
        handlers.find(url).map(|entry| entry.pattern)
    }

    #[test]
    fn a_socket_goes_to_the_last_registered_handler_that_matches_it() {
        let handlers = with_ws_patterns(&["**", "**/chat", "**/feed"]);
        assert_eq!(
            found(&handlers, "ws://example.com/chat").as_deref(),
            Some("**/chat")
        );
        assert_eq!(
            found(&handlers, "ws://example.com/other").as_deref(),
            Some("**")
        );
    }

    #[test]
    fn a_later_catch_all_shadows_an_earlier_specific_websocket_handler() {
        let handlers = with_ws_patterns(&["**/chat", "**"]);
        assert_eq!(
            found(&handlers, "ws://example.com/chat").as_deref(),
            Some("**")
        );
    }

    #[test]
    fn a_socket_no_pattern_matches_finds_no_handler() {
        let handlers = with_ws_patterns(&["wss://**/*"]);
        assert_eq!(found(&handlers, "ws://example.com/chat"), None);
    }

    #[test]
    fn a_malformed_pattern_matches_no_socket() {
        let handlers = with_ws_patterns(&["ws://**/{a,{b}}"]);
        assert_eq!(found(&handlers, "ws://example.com/a"), None);
    }

    #[test]
    fn websocket_interception_patterns_list_each_glob_in_registration_order() {
        let handlers = with_ws_patterns(&["**/chat", "wss://example.com/feed"]);
        assert_eq!(
            handlers.interception_patterns(),
            json!({ "patterns": [{ "glob": "**/chat" }, { "glob": "wss://example.com/feed" }] })
        );
    }
}
