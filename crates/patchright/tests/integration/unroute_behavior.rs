use crate::common::poll_until;
use crate::test_server::TestServer;
use patchright_rs::UnrouteBehavior;
use patchright_rs::protocol::Route;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

type HandlerFuture = Pin<Box<dyn Future<Output = patchright_rs::Result<()>> + Send>>;

/// A handler that takes long enough to still be running when the test
/// calls `unroute_all`, recording when it starts and when it finishes.
fn slow_handler() -> (
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    impl Fn(Route) -> HandlerFuture + Send + Sync + 'static,
) {
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let (s, f) = (started.clone(), finished.clone());
    let handler = move |route: Route| {
        let (s, f) = (s.clone(), f.clone());
        Box::pin(async move {
            s.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(400)).await;
            f.store(true, Ordering::SeqCst);
            route.continue_(None).await
        }) as HandlerFuture
    };
    (started, finished, handler)
}

async fn fire_request(page: &patchright_rs::protocol::Page, started: &AtomicBool) {
    page.evaluate_value("() => { fetch('/slow.txt'); }")
        .await
        .expect("evaluate should start the fetch");
    assert!(
        poll_until(Duration::from_secs(5), || started.load(Ordering::SeqCst)).await,
        "the route handler should have started"
    );
}

#[tokio::test]
async fn test_context_unroute_all_wait_returns_after_the_running_handler() {
    let (_pw, browser, context) = crate::common::setup_context().await;
    let server = TestServer::start().await;
    let (started, finished, handler) = slow_handler();
    context.route("**/slow.txt", handler).await.unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&format!("{}/", server.url()), None)
        .await
        .unwrap();
    fire_request(&page, &started).await;

    context
        .unroute_all(Some(UnrouteBehavior::Wait))
        .await
        .unwrap();

    assert!(
        finished.load(Ordering::SeqCst),
        "unroute_all(Wait) returned before the running handler finished"
    );
    browser.close().await.unwrap();
    server.shutdown();
}

#[tokio::test]
async fn test_context_unroute_all_default_does_not_wait() {
    let (_pw, browser, context) = crate::common::setup_context().await;
    let server = TestServer::start().await;
    let (started, finished, handler) = slow_handler();
    context.route("**/slow.txt", handler).await.unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&format!("{}/", server.url()), None)
        .await
        .unwrap();
    fire_request(&page, &started).await;

    context.unroute_all(None).await.unwrap();

    assert!(
        !finished.load(Ordering::SeqCst),
        "unroute_all(None) waited for the running handler"
    );
    assert!(poll_until(Duration::from_secs(5), || finished.load(Ordering::SeqCst)).await);
    browser.close().await.unwrap();
    server.shutdown();
}

#[tokio::test]
async fn test_page_unroute_all_wait_returns_after_the_running_handler() {
    let (_pw, browser, page) = crate::common::setup().await;
    let server = TestServer::start().await;
    let (started, finished, handler) = slow_handler();
    page.route("**/slow.txt", handler).await.unwrap();
    page.goto(&format!("{}/", server.url()), None)
        .await
        .unwrap();
    fire_request(&page, &started).await;

    page.unroute_all(Some(UnrouteBehavior::Wait)).await.unwrap();

    assert!(
        finished.load(Ordering::SeqCst),
        "unroute_all(Wait) returned before the running handler finished"
    );
    browser.close().await.unwrap();
    server.shutdown();
}

#[tokio::test]
async fn test_page_unroute_all_default_does_not_wait() {
    let (_pw, browser, page) = crate::common::setup().await;
    let server = TestServer::start().await;
    let (started, finished, handler) = slow_handler();
    page.route("**/slow.txt", handler).await.unwrap();
    page.goto(&format!("{}/", server.url()), None)
        .await
        .unwrap();
    fire_request(&page, &started).await;

    page.unroute_all(None).await.unwrap();

    assert!(
        !finished.load(Ordering::SeqCst),
        "unroute_all(None) waited for the running handler"
    );
    assert!(poll_until(Duration::from_secs(5), || finished.load(Ordering::SeqCst)).await);
    browser.close().await.unwrap();
    server.shutdown();
}
