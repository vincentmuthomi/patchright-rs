use super::BrowserContext;
use crate::protocol::api_request_context::APIRequestContext;
use crate::protocol::{Page, Request, ResponseObject, Route};
use crate::server::channel::Channel;
use crate::server::channel_owner::ChannelOwner;
use crate::server::connection::ConnectionExt;
use serde_json::Value;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Protocol event dispatch from the server into the context's registries.
impl BrowserContext {
    fn dispatch_request_event(&self, method: &str, params: Value) {
        if let Some(request_guid) = params
            .get("request")
            .and_then(|v| v.get("guid"))
            .and_then(|v| v.as_str())
        {
            let connection = self.connection();
            let request_guid_owned = request_guid.to_owned();
            let page_guid_owned = params
                .get("page")
                .and_then(|v| v.get("guid"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_owned());
            // Extract failureText for requestFailed events
            let failure_text = params
                .get("failureText")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            // Extract response GUID for requestFinished events (to read timing)
            let response_guid_owned = params
                .get("response")
                .and_then(|v| v.get("guid"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            // Extract responseEndTiming from requestFinished event params
            let response_end_timing = params.get("responseEndTiming").and_then(|v| v.as_f64());
            let method = method.to_owned();
            // Clone context-level handler vecs for use in spawn
            let ctx_request = self.request.clone();
            let ctx_request_finished = self.request_finished.clone();
            let ctx_request_failed = self.request_failed.clone();
            crate::task::spawn(async move {
                let request: Request =
                    match connection.get_typed::<Request>(&request_guid_owned).await {
                        Ok(r) => r,
                        Err(_) => return,
                    };

                // Set failure text on the request before dispatching to handlers
                if let Some(text) = failure_text {
                    request.set_failure_text(text);
                }

                // For requestFinished, extract timing from the Response object's initializer
                if method == "requestFinished"
                    && let Some(timing) =
                        extract_timing(&connection, response_guid_owned, response_end_timing).await
                {
                    request.set_timing(timing);
                }

                // Dispatch to context-level handlers first (matching playwright-python behavior)
                match method.as_str() {
                    "request" => _ = ctx_request.dispatch(request.clone()).await,
                    "requestFinished" => _ = ctx_request_finished.dispatch(request.clone()).await,
                    "requestFailed" => _ = ctx_request_failed.dispatch(request.clone()).await,
                    _ => {}
                }

                // Then dispatch to page-level handlers
                if let Some(page_guid) = page_guid_owned {
                    let page: Page = match connection.get_typed::<Page>(&page_guid).await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    match method.as_str() {
                        "request" => page.trigger_request_event(request).await,
                        "requestFailed" => page.trigger_request_failed_event(request).await,
                        "requestFinished" => page.trigger_request_finished_event(request).await,
                        _ => unreachable!("Unreachable method {}", method),
                    }
                }
            });
        }
    }

    fn dispatch_response_event(&self, _method: &str, params: Value) {
        if let Some(response_guid) = params
            .get("response")
            .and_then(|v| v.get("guid"))
            .and_then(|v| v.as_str())
        {
            let connection = self.connection();
            let response_guid_owned = response_guid.to_owned();
            let page_guid_owned = params
                .get("page")
                .and_then(|v| v.get("guid"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_owned());
            let ctx_response = self.response.clone();
            crate::task::spawn(async move {
                let response: ResponseObject = match connection
                    .get_typed::<ResponseObject>(&response_guid_owned)
                    .await
                {
                    Ok(r) => r,
                    Err(_) => return,
                };

                // Dispatch to context-level handlers first (matching playwright-python behavior)
                ctx_response.dispatch(response.clone()).await;

                // Then dispatch to page-level handlers
                if let Some(page_guid) = page_guid_owned {
                    let page: Page = match connection.get_typed::<Page>(&page_guid).await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    page.trigger_response_event(response).await;
                }
            });
        }
    }
}

impl ChannelOwner for BrowserContext {
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

    fn channel(&self) -> &Channel {
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

    fn on_event(&self, method: &str, params: Value) {
        match method {
            "request" | "requestFailed" | "requestFinished" => {
                self.dispatch_request_event(method, params)
            }
            "response" => self.dispatch_response_event(method, params),
            "close" => {
                // BrowserContext close event — mark as closed and fire registered close handlers
                self.is_closed.store(true, Ordering::Relaxed);
                let close_events = self.close_events.clone();
                crate::task::spawn(async move {
                    close_events.dispatch_all(()).await;
                });
            }
            "page" => {
                // Page events are triggered when pages are created, including:
                // - Initial page in persistent context with --app mode
                // - Popup pages opened through user interactions
                // Event format: {page: {guid: "..."}}
                if let Some(page_guid) = params
                    .get("page")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                {
                    let connection = self.connection();
                    let page_guid_owned = page_guid.to_string();
                    let pages = self.pages.clone();
                    let page_events = self.page_events.clone();
                    let forwarders = self.forwarders.clone();

                    crate::task::spawn(async move {
                        // Get and downcast the Page object
                        let page: Page = match connection.get_typed::<Page>(&page_guid_owned).await
                        {
                            Ok(p) => p,
                            Err(_) => return,
                        };

                        // Track the page
                        pages.lock().unwrap().push(page.clone());

                        // Forward this page's lifecycle events to the context's handlers.
                        forwarders.wire(&page).await;

                        // If this page has an opener, dispatch popup event to opener's handlers.
                        // The opener guid is in the page's initializer: {"opener": {"guid": "..."}}
                        if let Some(opener_guid) = page
                            .initializer()
                            .get("opener")
                            .and_then(|v| v.get("guid"))
                            .and_then(|v| v.as_str())
                            && let Ok(opener) = connection.get_typed::<Page>(opener_guid).await
                        {
                            opener.trigger_popup_event(page.clone()).await;
                        }

                        // Dispatch to context-level page handlers, then the
                        // longest-waiting expect_page() caller.
                        page_events.dispatch(page).await;
                    });
                }
            }
            "pageError" => {
                // pageError event: fired when an uncaught JS exception occurs on a page.
                // Event format:
                //   { "error": { "error": { "message": "...", "name": "...", "stack": "..." } },
                //     "page": { "guid": "page@..." } }
                //
                // Dispatch path:
                //  1. Construct WebError and fire context-level on_weberror handlers.
                //  2. Forward the raw message to the page's on_pageerror handlers.
                let message = params
                    .get("error")
                    .and_then(|e| e.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();

                let page_guid_owned = params
                    .get("page")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let location =
                    params
                        .get("location")
                        .map(|loc| crate::protocol::WebErrorLocation {
                            url: loc
                                .get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            line: loc.get("line").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                            column: loc.get("column").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        });

                let connection = self.connection();
                let weberror = self.weberror.clone();

                crate::task::spawn(async move {
                    // Resolve page (optional — may be None if page already closed)
                    let page = if let Some(ref guid) = page_guid_owned {
                        connection.get_typed::<Page>(guid).await.ok()
                    } else {
                        None
                    };

                    // 1. Dispatch to context-level weberror handlers
                    let web_error = crate::protocol::WebError::new(
                        message.clone(),
                        page.clone(),
                        location.clone(),
                    );
                    weberror.dispatch(web_error).await;

                    // 2. Forward to page-level pageerror handlers
                    if let Some(p) = page {
                        p.trigger_pageerror_event(message).await;
                    }
                });
            }
            "dialog" => {
                // Dialog events come to BrowserContext.
                // Dispatch to context-level handlers first, then forward to the Page.
                // Event format: {dialog: {guid: "..."}}
                // The Dialog protocol object has the Page as its parent
                if let Some(dialog_guid) = params
                    .get("dialog")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                {
                    let connection = self.connection();
                    let dialog_guid_owned = dialog_guid.to_string();
                    let ctx_dialog = self.dialog.clone();

                    crate::task::spawn(async move {
                        // We subscribed, so the driver is waiting on us to answer;
                        // a dialog we cannot resolve stays open.
                        let dialog = match connection
                            .get_typed::<crate::protocol::Dialog>(&dialog_guid_owned)
                            .await
                        {
                            Ok(d) => d,
                            Err(e) => {
                                tracing::warn!(
                                    "dialog {dialog_guid_owned} could not be resolved and stays unanswered: {e}"
                                );
                                return;
                            }
                        };

                        // Context-level handlers first, then the page's.
                        let heard_by_context = ctx_dialog.dispatch(dialog.clone()).await;
                        let heard_by_page = match dialog.page() {
                            Some(page) => page.on_dialog_event(dialog.clone()).await,
                            None => false,
                        };

                        // A dialog can land after `on_dialog` subscribes but before
                        // its handler is added, or belong to a page we cannot
                        // resolve; answer it as the driver would have.
                        let answered = match super::events::unheard_dialog_answer(
                            heard_by_context,
                            heard_by_page,
                            dialog.type_(),
                        ) {
                            Some(super::events::UnheardDialogAnswer::Accept) => {
                                dialog.accept(None).await
                            }
                            Some(super::events::UnheardDialogAnswer::Dismiss) => {
                                dialog.dismiss().await
                            }
                            None => Ok(()),
                        };
                        if let Err(e) = answered {
                            tracing::debug!("answering an unheard dialog failed: {e}");
                        }
                    });
                }
            }
            "dialogClosed" => {
                // Same delivery as `dialog`: the context sees it, then the
                // page whose dialog it was.
                if let Some(dialog_guid) = params
                    .get("dialog")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                {
                    let connection = self.connection();
                    let dialog_guid_owned = dialog_guid.to_string();
                    let ctx_dialog_closed = self.dialog_closed.clone();

                    crate::task::spawn(async move {
                        let Ok(dialog) = connection
                            .get_typed::<crate::protocol::Dialog>(&dialog_guid_owned)
                            .await
                        else {
                            return;
                        };

                        ctx_dialog_closed.dispatch(dialog.clone()).await;

                        if let Some(page) =
                            crate::server::connection::downcast_parent::<Page>(&dialog)
                        {
                            page.trigger_dialog_closed_event(dialog).await;
                        }
                    });
                }
            }
            "bindingCall" => {
                // A JS caller invoked an exposed function. Dispatch to the registered
                // callback and send the result back via BindingCall::fulfill.
                // Event format: {binding: {guid: "..."}}
                if let Some(binding_guid) = params
                    .get("binding")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                {
                    let connection = self.connection();
                    let binding_guid_owned = binding_guid.to_string();
                    let binding_callbacks = self.binding_callbacks.clone();

                    crate::task::spawn(async move {
                        let binding_call: crate::protocol::BindingCall = match connection
                            .get_typed::<crate::protocol::BindingCall>(&binding_guid_owned)
                            .await
                        {
                            Ok(bc) => bc,
                            Err(e) => {
                                tracing::warn!("Failed to get BindingCall object: {}", e);
                                return;
                            }
                        };

                        let name = binding_call.name().to_string();

                        // Look up the registered callback
                        let callback = {
                            let callbacks = binding_callbacks.lock().unwrap();
                            callbacks.get(&name).cloned()
                        };

                        let Some(callback) = callback else {
                            tracing::warn!("No callback registered for binding '{}'", name);
                            let _ = binding_call
                                .reject(&format!("No Rust handler for binding '{name}'"))
                                .await;
                            return;
                        };

                        // Deserialize the args from Playwright protocol format
                        let raw_args = binding_call.args();
                        let args = Self::deserialize_binding_args(raw_args);

                        // Call the callback and serialize the result
                        let result_value = callback(args).await;
                        let serialized =
                            crate::protocol::evaluate_conversion::serialize_argument(&result_value);

                        if let Err(e) = binding_call.resolve(serialized).await {
                            tracing::warn!("Failed to resolve BindingCall '{}': {}", name, e);
                        }
                    });
                }
            }
            "route" => {
                // Handle context-level network routing event
                if let Some(route_guid) = params
                    .get("route")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                {
                    let connection = self.connection();
                    let route_guid_owned = route_guid.to_string();
                    let context = self.clone();
                    let request_context_guid = self.request_context_guid.clone();

                    crate::task::spawn(async move {
                        let route: Route =
                            match connection.get_typed::<Route>(&route_guid_owned).await {
                                Ok(r) => r,
                                Err(e) => {
                                    tracing::warn!("Failed to get route object: {}", e);
                                    return;
                                }
                            };

                        // Set APIRequestContext on the route for fetch() support
                        if let Some(ref guid) = request_context_guid
                            && let Ok(api_ctx) =
                                connection.get_typed::<APIRequestContext>(guid).await
                        {
                            route.set_api_request_context(api_ctx);
                        }

                        context.on_route_event(route).await;
                    });
                }
            }
            "console" => {
                // Console events are sent to BrowserContext.
                // Construct ConsoleMessage from params, dispatch to context-level handlers,
                // then forward to the Page's on_console handlers.
                //
                // Event params format:
                // {
                //   type: "log"|"error"|"warning"|...,
                //   text: "rendered text",
                //   location: { url: "...", lineNumber: N, columnNumber: N },
                //   page: { guid: "page@..." },
                //   args: [ { guid: "JSHandle@..." }, ... ]  -- resolved to Arc<JSHandle>
                //   timestamp: <f64 milliseconds since Unix epoch>
                // }
                let type_ = params
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("log")
                    .to_string();
                let text = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let loc_url = params
                    .get("location")
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // 1.60 emits `line`/`column`; older drivers used
                // `lineNumber`/`columnNumber` (deprecated, may be removed). Prefer
                // the new keys, fall back to the legacy ones.
                let loc_line = params
                    .get("location")
                    .and_then(|v| v.get("line").or_else(|| v.get("lineNumber")))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                let loc_col = params
                    .get("location")
                    .and_then(|v| v.get("column").or_else(|| v.get("columnNumber")))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                let page_guid_owned = params
                    .get("page")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                // Collect arg GUIDs before spawning.
                let arg_guids: Vec<String> = params
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| {
                                v.get("guid")
                                    .and_then(|g| g.as_str())
                                    .map(|s| s.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let timestamp = params
                    .get("timestamp")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);

                let connection = self.connection();
                let ctx_console = self.console.clone();

                crate::task::spawn(async move {
                    use crate::protocol::JSHandle;
                    use crate::protocol::console_message::{
                        ConsoleMessage, ConsoleMessageLocation,
                    };

                    // Optionally resolve the page back-reference
                    let page = if let Some(ref guid) = page_guid_owned {
                        connection.get_typed::<Page>(guid).await.ok()
                    } else {
                        None
                    };

                    // Resolve JSHandle args from the connection registry.
                    let args: Vec<std::sync::Arc<JSHandle>> = {
                        let mut resolved = Vec::with_capacity(arg_guids.len());
                        for guid in &arg_guids {
                            if let Ok(handle) = connection.get_typed::<JSHandle>(guid).await {
                                resolved.push(std::sync::Arc::new(handle));
                            }
                        }
                        resolved
                    };

                    let location = ConsoleMessageLocation {
                        url: loc_url,
                        line_number: loc_line,
                        column_number: loc_col,
                    };

                    let msg =
                        ConsoleMessage::new(type_, text, location, page.clone(), args, timestamp);

                    // Handlers first, then the longest-waiting
                    // expect_console_message() caller — the same order as
                    // every other event since the registry migration.
                    ctx_console.dispatch(msg.clone()).await;

                    // Forward to page-level handlers
                    if let Some(p) = page {
                        p.trigger_console_event(msg).await;
                    }
                });
            }
            "serviceWorker" => {
                // A new service worker was registered in this context.
                // Event format: {worker: {guid: "Worker@..."}}
                if let Some(worker_guid) = params
                    .get("worker")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                {
                    let connection = self.connection();
                    let worker_guid_owned = worker_guid.to_string();
                    let serviceworker_handlers = self.serviceworker_handlers.clone();
                    let serviceworker_waiters = self.serviceworker_waiters.clone();
                    let service_workers_list = self.service_workers_list.clone();

                    crate::task::spawn(async move {
                        let worker: crate::protocol::Worker = match connection
                            .get_typed::<crate::protocol::Worker>(&worker_guid_owned)
                            .await
                        {
                            Ok(w) => w,
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to get Worker object for serviceWorker event: {}",
                                    e
                                );
                                return;
                            }
                        };

                        // Track for service_workers() accessor
                        service_workers_list.lock().unwrap().push(worker.clone());

                        let handlers = serviceworker_handlers.lock().unwrap().clone();
                        for handler in handlers {
                            let worker_clone = worker.clone();
                            crate::task::spawn(async move {
                                if let Err(e) = handler(worker_clone).await {
                                    tracing::error!("Error in serviceworker handler: {}", e);
                                }
                            });
                        }
                        // Notify the oldest expect_event("serviceworker") waiter still listening.
                        crate::protocol::event_registry::deliver_to_oldest_live(
                            &mut serviceworker_waiters.lock().unwrap(),
                            worker,
                        );
                    });
                }
            }
            "webSocketRoute" => {
                // A WebSocket matched a route_web_socket pattern on the context.
                // Event format: {webSocketRoute: {guid: "WebSocketRoute@..."}}
                if let Some(wsr_guid) = params
                    .get("webSocketRoute")
                    .and_then(|v| v.get("guid"))
                    .and_then(|v| v.as_str())
                {
                    self.ws_route_handlers
                        .dispatch(self.connection(), wsr_guid, "Context");
                }
            }
            _ => {
                // Other events will be handled in future phases
            }
        }
    }

    fn was_collected(&self) -> bool {
        self.base.was_collected()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Extracts timing data from a Response object's initializer, patching in
/// `responseEnd` from the event's `responseEndTiming` if available.
async fn extract_timing(
    connection: &std::sync::Arc<dyn crate::server::connection::ConnectionLike>,
    response_guid: Option<String>,
    response_end_timing: Option<f64>,
) -> Option<serde_json::Value> {
    let resp_guid = response_guid?;
    let resp_obj: crate::protocol::ResponseObject = connection
        .get_typed::<crate::protocol::ResponseObject>(&resp_guid)
        .await
        .ok()?;
    let mut timing = resp_obj.initializer().get("timing")?.clone();
    crate::protocol::ResourceTiming::merge_response_end(&mut timing, response_end_timing);
    Some(timing)
}
