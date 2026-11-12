use super::{BrowserContext, CtxHandlerFuture, ServiceWorkerHandlerFuture};
use crate::error::Result;
use crate::protocol::EventValue;
use crate::protocol::event_registry::{EventRegistry, Handler, bridge};
use crate::protocol::event_waiter::EventWaiter;
use crate::protocol::{Download, Frame, Page, Request, ResponseObject};
use crate::server::channel_owner::ChannelOwner;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// Event subscriptions (`on_*`), page-event forwarders, and one-shot waiters (`expect_*`).
///
/// A waiter is created before the action that fires its event, then awaited
/// after it:
///
/// ```no_run
/// # use patchright_rs::Playwright;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let pw = Playwright::launch().await?;
/// # let browser = pw.chromium().launch().await?;
/// # let context = browser.new_context().await?;
/// let opened = context.expect_page(None).await?;
/// let _page = context.new_page().await?;
/// let new_page = opened.wait().await?;
///
/// let closed = context.expect_close(None).await?;
/// context.close().await?;
/// closed.wait().await?;
/// # Ok(())
/// # }
/// ```
impl BrowserContext {
    /// Adds a listener for the `page` event.
    ///
    /// The handler is called whenever a new page is created in this context,
    /// including popup pages opened through user interactions.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async function that receives the new `Page`
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-page>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_page<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Page) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<Page> = Arc::new(move |page| Box::pin(handler(page)));
        self.page_events.add_handler(handler);
        Ok(())
    }

    /// Subscribe to `reg`'s event if nothing is listening yet.
    ///
    /// Same contract as `Page::subscribe_if_idle`: the server only pushes an
    /// event once asked, so the first handler or `expect_*` turns it on, and
    /// the name comes from the registry rather than being restated per site.
    async fn subscribe_if_idle<T>(&self, reg: &EventRegistry<T>) {
        if reg.is_idle() {
            _ = self.channel().update_subscription(reg.name(), true).await;
        }
    }

    /// Adds a listener for the `download` event: fired when any page in the
    /// context starts a download. Forwarded from each page's own `download`
    /// event.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-download>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_download<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Download) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        add_forwarded(&self.forwarders.download, handler);
        Ok(())
    }

    /// Adds a listener for the `frameAttached` event: fired when a frame is
    /// attached in any page of the context. Forwarded from each page.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-frame-attached>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_frame_attached<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Frame) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        add_forwarded(&self.forwarders.frame_attached, handler);
        Ok(())
    }

    /// Adds a listener for the `frameDetached` event: fired when a frame is
    /// detached in any page of the context. Forwarded from each page.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-frame-detached>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_frame_detached<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Frame) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        add_forwarded(&self.forwarders.frame_detached, handler);
        Ok(())
    }

    /// Adds a listener for the `frameNavigated` event: fired when a frame
    /// navigates in any page of the context. Forwarded from each page.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-frame-navigated>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_frame_navigated<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Frame) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        add_forwarded(&self.forwarders.frame_navigated, handler);
        Ok(())
    }

    /// Adds a listener for the `pageLoad` event: fired when any page in the
    /// context fires its `load` event. The handler receives that `Page`.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-page-load>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_page_load<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Page) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        add_forwarded(&self.forwarders.page_load, handler);
        Ok(())
    }

    /// Adds a listener for the `pageClose` event: fired when any page in the
    /// context closes. The handler receives that `Page`.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-page-close>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_page_close<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Page) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        add_forwarded(&self.forwarders.page_close, handler);
        Ok(())
    }

    /// Adds a listener for the `close` event.
    ///
    /// The handler is called when the browser context is closed.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async function called with no arguments when the context closes
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-close>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_close<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<()> = Arc::new(move |()| Box::pin(handler()));
        self.close_events.add_handler(handler);
        Ok(())
    }

    /// Adds a listener for the `request` event.
    ///
    /// The handler fires whenever a request is issued from any page in the context.
    /// This is equivalent to subscribing to `on_request` on each individual page,
    /// but covers all current and future pages of the context.
    ///
    /// Context-level handlers fire before page-level handlers.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async function that receives the `Request`
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-request>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_request<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<Request> = Arc::new(move |request| Box::pin(handler(request)));
        self.subscribe_if_idle(&self.request).await;
        self.request.add_handler(handler);
        Ok(())
    }

    /// Adds a listener for the `requestFinished` event.
    ///
    /// The handler fires after the request has been successfully received by the server
    /// and a response has been fully downloaded for any page in the context.
    ///
    /// Context-level handlers fire before page-level handlers.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async function that receives the completed `Request`
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-request-finished>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_request_finished<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<Request> = Arc::new(move |request| Box::pin(handler(request)));
        self.subscribe_if_idle(&self.request_finished).await;
        self.request_finished.add_handler(handler);
        Ok(())
    }

    /// Adds a listener for the `requestFailed` event.
    ///
    /// The handler fires when a request from any page in the context fails,
    /// for example due to a network error or if the server returned an error response.
    ///
    /// Context-level handlers fire before page-level handlers.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async function that receives the failed `Request`
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-request-failed>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_request_failed<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<Request> = Arc::new(move |request| Box::pin(handler(request)));
        self.subscribe_if_idle(&self.request_failed).await;
        self.request_failed.add_handler(handler);
        Ok(())
    }

    /// Adds a listener for the `response` event.
    ///
    /// The handler fires whenever a response is received from any page in the context.
    ///
    /// Context-level handlers fire before page-level handlers.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async function that receives the `ResponseObject`
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-response>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_response<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(ResponseObject) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<ResponseObject> =
            Arc::new(move |response| Box::pin(handler(response)));
        self.subscribe_if_idle(&self.response).await;
        self.response.add_handler(handler);
        Ok(())
    }

    /// Adds a listener for the `dialog` event on this browser context.
    ///
    /// The handler fires whenever a JavaScript dialog (alert, confirm, prompt,
    /// or beforeunload) is triggered from **any** page in the context. Context-level
    /// handlers fire before page-level handlers.
    ///
    /// With no `dialog` handler on the context or the page, a dialog is
    /// dismissed automatically (a `beforeunload` dialog is accepted). Once a
    /// handler is registered, the dialog waits until a handler accepts or
    /// dismisses it, so a handler that does neither leaves the page blocked.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async function that receives the [`Dialog`](crate::protocol::Dialog) and calls
    ///   `dialog.accept()` or `dialog.dismiss()`.
    ///
    /// # Errors
    ///
    /// Returns error if communication with the browser process fails.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-dialog>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_dialog<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(crate::protocol::Dialog) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<crate::protocol::Dialog> =
            Arc::new(move |dialog| Box::pin(handler(dialog)));
        self.subscribe_if_idle(&self.dialog).await;
        self.dialog.add_handler(handler);
        Ok(())
    }

    /// Adds a listener for the `dialogclosed` event, which fires once a
    /// dialog has been accepted, dismissed, or closed by the user, on any
    /// page in the context.
    ///
    /// Context-level handlers fire before page-level ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the handler cannot be registered.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-dialog-closed>
    pub async fn on_dialog_closed<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(crate::protocol::Dialog) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<crate::protocol::Dialog> =
            Arc::new(move |dialog| Box::pin(handler(dialog)));
        self.subscribe_if_idle(&self.dialog_closed).await;
        self.dialog_closed.add_handler(handler);
        Ok(())
    }

    /// Subscribe to `dialogClosed` if nothing has yet, so a page-level
    /// handler receives the event even when the context has none of its own.
    pub(crate) async fn ensure_dialog_closed_subscription(&self) {
        self.subscribe_if_idle(&self.dialog_closed).await;
    }

    /// Registers a context-level console event handler.
    ///
    /// The handler fires for any console message emitted by any page in this context.
    /// Context-level handlers fire before page-level handlers.
    ///
    /// The server only sends console events after the first handler is registered
    /// (subscription is managed automatically per context channel).
    ///
    /// # Arguments
    ///
    /// * `handler` - Async closure that receives the [`ConsoleMessage`](crate::protocol::ConsoleMessage)
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-console>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_console<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(crate::protocol::ConsoleMessage) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<crate::protocol::ConsoleMessage> =
            Arc::new(move |msg| Box::pin(handler(msg)));

        self.subscribe_if_idle(&self.console).await;
        self.console.add_handler(handler);

        Ok(())
    }

    /// Registers a context-level handler for uncaught JavaScript exceptions.
    ///
    /// The handler fires whenever a page in this context throws an unhandled
    /// JavaScript error (i.e. an exception that propagates to `window.onerror`
    /// or an unhandled promise rejection). The [`WebError`](crate::protocol::WebError)
    /// passed to the handler contains the error message and an optional back-reference
    /// to the originating page.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async closure that receives a [`WebError`](crate::protocol::WebError).
    ///
    /// # Errors
    ///
    /// Returns error if communication with the browser process fails.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-web-error>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_weberror<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(crate::protocol::WebError) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler: Handler<crate::protocol::WebError> =
            Arc::new(move |web_error| Box::pin(handler(web_error)));
        self.weberror.add_handler(handler);
        Ok(())
    }

    /// Registers a handler for the `serviceWorker` event.
    ///
    /// The handler is called when a new service worker is registered in the browser context.
    ///
    /// Note: Service worker testing typically requires HTTPS and a registered service worker.
    ///
    /// # Arguments
    ///
    /// * `handler` - Async closure called with the new [`Worker`](crate::protocol::Worker) object
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-service-worker>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn on_serviceworker<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(crate::protocol::Worker) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler = Arc::new(
            move |worker: crate::protocol::Worker| -> ServiceWorkerHandlerFuture {
                Box::pin(handler(worker))
            },
        );
        self.serviceworker_handlers.lock().unwrap().push(handler);
        Ok(())
    }

    /// Waits for a new page to be created in this browser context.
    ///
    /// Creates a one-shot waiter that resolves when the next `page` event fires.
    /// The waiter **must** be created before the action that triggers the new page
    /// (e.g. `new_page()` or a user action that opens a popup) to avoid a race
    /// condition.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Timeout in milliseconds. Defaults to 30 000 ms if `None`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Timeout`] if no page is created within the timeout.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-wait-for-event>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn expect_page(&self, timeout: Option<f64>) -> Result<EventWaiter<Page>> {
        let rx = self.page_events.wait();
        Ok(EventWaiter::new(rx, timeout.or(Some(30_000.0))))
    }

    /// Waits for this browser context to be closed.
    ///
    /// Creates a one-shot waiter that resolves when the `close` event fires.
    /// The waiter **must** be created before the action that closes the context
    /// to avoid a race condition.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Timeout in milliseconds. Defaults to 30 000 ms if `None`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Timeout`] if the context is not closed within the timeout.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-wait-for-event>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn expect_close(&self, timeout: Option<f64>) -> Result<EventWaiter<()>> {
        let rx = self.close_events.wait();
        Ok(EventWaiter::new(rx, timeout.or(Some(30_000.0))))
    }

    /// Waits for a console message from any page in this context.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-event-console>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn expect_console_message(
        &self,
        timeout: Option<f64>,
    ) -> Result<EventWaiter<crate::protocol::ConsoleMessage>> {
        self.subscribe_if_idle(&self.console).await;
        let rx = self.console.wait();
        Ok(EventWaiter::new(rx, timeout.or(Some(30_000.0))))
    }

    /// Waits for the given event to fire and returns a typed `EventValue`.
    ///
    /// This is the generic version of the specific `expect_*` methods. It matches
    /// the playwright-python / playwright-js `context.expect_event(event_name)` API.
    ///
    /// The waiter **must** be created before the action that triggers the event.
    ///
    /// # Supported event names
    ///
    /// `"page"`, `"close"`, `"console"`, `"request"`, `"response"`,
    /// `"weberror"`, `"serviceworker"`
    ///
    /// # Arguments
    ///
    /// * `event` - Event name (case-sensitive, matches Playwright protocol names).
    /// * `timeout` - Timeout in milliseconds. Defaults to 30 000 ms if `None`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::InvalidArgument`] for unknown event names.
    /// Returns [`crate::error::Error::Timeout`] if the event does not fire within the timeout.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-wait-for-event>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn expect_event(
        &self,
        event: &str,
        timeout: Option<f64>,
    ) -> crate::error::Result<EventWaiter<crate::protocol::EventValue>> {
        let timeout_ms = timeout.or(Some(30_000.0));

        let waiter = |rx| EventWaiter::new(rx, timeout_ms);
        match event {
            "page" => Ok(waiter(bridge(self.page_events.wait(), EventValue::Page))),
            "close" => Ok(waiter(bridge(self.close_events.wait(), |()| {
                EventValue::Close
            }))),
            "console" => {
                self.subscribe_if_idle(&self.console).await;
                Ok(waiter(bridge(
                    self.console.wait(),
                    EventValue::ConsoleMessage,
                )))
            }
            "request" => {
                self.subscribe_if_idle(&self.request).await;
                Ok(waiter(bridge(self.request.wait(), EventValue::Request)))
            }
            "response" => {
                self.subscribe_if_idle(&self.response).await;
                Ok(waiter(bridge(self.response.wait(), EventValue::Response)))
            }
            "weberror" => Ok(waiter(bridge(self.weberror.wait(), EventValue::WebError))),
            "serviceworker" => {
                let (inner_tx, inner_rx) = oneshot::channel();
                self.serviceworker_waiters.lock().unwrap().push(inner_tx);
                Ok(waiter(bridge(inner_rx, EventValue::Worker)))
            }
            other => Err(crate::error::Error::InvalidArgument(format!(
                "Unknown event name '{}'. Supported: page, close, console, request, response, \
                 weberror, serviceworker",
                other
            ))),
        }
    }
}

/// A context-level handler for an event forwarded from each page.
type ForwardedHandler<T> = Arc<dyn Fn(T) -> CtxHandlerFuture + Send + Sync>;
/// The handlers registered for one forwarded event.
type Forwarded<T> = Arc<Mutex<Vec<ForwardedHandler<T>>>>;

/// Context-level handlers for the events forwarded from each page.
///
/// These are not wire events on the context channel. Each page's own event
/// is forwarded to the handlers here, matching how the upstream clients
/// synthesize them. Every page is wired once, when the context learns of
/// it, so registering a handler never touches a page and there is no
/// check-then-wire window for two registrations or a page arriving mid-way
/// to fall into.
#[derive(Clone, Default)]
pub(super) struct Forwarders {
    pub(super) download: Forwarded<Download>,
    pub(super) frame_attached: Forwarded<Frame>,
    pub(super) frame_detached: Forwarded<Frame>,
    pub(super) frame_navigated: Forwarded<Frame>,
    pub(super) page_load: Forwarded<Page>,
    pub(super) page_close: Forwarded<Page>,
}

impl Forwarders {
    /// Forwards each of `page`'s lifecycle events to these handlers.
    pub(super) async fn wire(&self, page: &Page) {
        _ = page.on_download(forward_to(&self.download)).await;
        _ = page
            .on_frameattached(forward_to(&self.frame_attached))
            .await;
        _ = page
            .on_framedetached(forward_to(&self.frame_detached))
            .await;
        _ = page
            .on_framenavigated(forward_to(&self.frame_navigated))
            .await;
        let (load, p) = (forward_to(&self.page_load), page.clone());
        _ = page.on_load(move || load(p.clone())).await;
        let (close, p) = (forward_to(&self.page_close), page.clone());
        _ = page.on_close(move || close(p.clone())).await;
    }
}

/// Registers `handler` for a forwarded event.
fn add_forwarded<T, F, Fut>(handlers: &Forwarded<T>, handler: F)
where
    F: Fn(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    handlers
        .lock()
        .unwrap()
        .push(Arc::new(move |value| Box::pin(handler(value))));
}

/// A page-level handler that runs every handler in `handlers` with the
/// event's value.
fn forward_to<T: Clone + Send + 'static>(
    handlers: &Forwarded<T>,
) -> impl Fn(T) -> CtxHandlerFuture + Send + Sync + 'static {
    let handlers = handlers.clone();
    move |value| {
        let handlers = handlers.clone();
        Box::pin(async move {
            run_all(&handlers, value).await;
            Ok(())
        })
    }
}

/// Runs the handlers registered at call time, in registration order. An
/// error is logged so one failing handler does not stop the rest.
async fn run_all<T: Clone>(handlers: &Forwarded<T>, value: T) {
    let handlers = handlers.lock().unwrap().clone();
    for handler in handlers {
        if let Err(e) = handler(value.clone()).await {
            tracing::warn!("context handler error: {e}");
        }
    }
}

/// The answer the client gives a dialog that reached it but no handler heard.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum UnheardDialogAnswer {
    Accept,
    Dismiss,
}

/// How to answer a delivered dialog: not at all if the context or the page
/// heard it, otherwise the driver's own rule for a dialog nobody subscribed
/// to, which accepts `beforeunload` and dismisses the rest.
pub(super) fn unheard_dialog_answer(
    heard_by_context: bool,
    heard_by_page: bool,
    dialog_type: &str,
) -> Option<UnheardDialogAnswer> {
    if heard_by_context || heard_by_page {
        None
    } else if dialog_type == "beforeunload" {
        Some(UnheardDialogAnswer::Accept)
    } else {
        Some(UnheardDialogAnswer::Dismiss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_dialog_the_context_or_the_page_heard_is_left_to_its_handler() {
        assert_eq!(unheard_dialog_answer(true, false, "confirm"), None);
        assert_eq!(unheard_dialog_answer(false, true, "confirm"), None);
        assert_eq!(unheard_dialog_answer(true, true, "beforeunload"), None);
    }

    #[test]
    fn an_unheard_beforeunload_is_accepted() {
        assert_eq!(
            unheard_dialog_answer(false, false, "beforeunload"),
            Some(UnheardDialogAnswer::Accept)
        );
    }

    #[test]
    fn every_other_unheard_dialog_is_dismissed() {
        for dialog_type in ["alert", "confirm", "prompt"] {
            assert_eq!(
                unheard_dialog_answer(false, false, dialog_type),
                Some(UnheardDialogAnswer::Dismiss),
                "{dialog_type}"
            );
        }
    }

    #[tokio::test]
    async fn run_all_runs_every_handler_in_order_past_a_failure() {
        let handlers: Forwarded<u8> = Forwarded::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        for i in 0..3u8 {
            let (seen, calls) = (seen.clone(), calls.clone());
            add_forwarded(&handlers, move |v: u8| {
                let (seen, calls) = (seen.clone(), calls.clone());
                async move {
                    seen.lock().unwrap().push((i, v));
                    calls.fetch_add(1, Ordering::SeqCst);
                    if i == 1 {
                        Err(crate::error::Error::ProtocolError("boom".into()))
                    } else {
                        Ok(())
                    }
                }
            });
        }
        run_all(&handlers, 9).await;
        assert_eq!(*seen.lock().unwrap(), vec![(0, 9), (1, 9), (2, 9)]);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn forward_to_reaches_handlers_added_after_wiring() {
        let handlers: Forwarded<u8> = Forwarded::default();
        let page_side = forward_to(&handlers);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        add_forwarded(&handlers, move |v: u8| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(v);
                Ok(())
            }
        });
        page_side(4).await.unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![4]);
    }
}
