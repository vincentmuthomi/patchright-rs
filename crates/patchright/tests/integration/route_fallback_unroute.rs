use crate::test_server::TestServer;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn test_route_fallback_basic() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    // Set up route that calls fallback (should send request through to network)
    page.route("**/*", |route| async move { route.fallback(None).await })
        .await
        .expect("Failed to set up route");

    let response = page
        .goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate")
        .expect("Expected a response");

    assert_eq!(
        response.status(),
        200,
        "Fallback should allow request through"
    );

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn test_route_fallback_handler_chaining() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let handler_order = Arc::new(Mutex::new(Vec::<String>::new()));

    // First handler (registered first, checked last due to reverse iteration)
    let order1 = handler_order.clone();
    page.route("**/*", move |route| {
        let order = order1.clone();
        async move {
            order.lock().unwrap().push("handler1".to_string());
            route.continue_(None).await
        }
    })
    .await
    .expect("Failed to set up route 1");

    // Second handler (registered last, checked first)
    // This one calls fallback(), so handler1 should also run
    let order2 = handler_order.clone();
    page.route("**/*", move |route| {
        let order = order2.clone();
        async move {
            order.lock().unwrap().push("handler2".to_string());
            route.fallback(None).await
        }
    })
    .await
    .expect("Failed to set up route 2");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");

    let order = handler_order.lock().unwrap().clone();
    assert!(
        order.contains(&"handler2".to_string()),
        "Handler 2 (last registered) should run first"
    );
    assert!(
        order.contains(&"handler1".to_string()),
        "Handler 1 should run after handler 2 calls fallback()"
    );

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn test_page_unroute_removes_handler() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    // Set up route to abort all image requests
    page.route("**/*.png", |route| async move { route.abort(None).await })
        .await
        .expect("Failed to set up route");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");

    // Verify abort works
    let result = page
        .evaluate_value(
            r#"
        fetch('/image.png')
            .then(() => 'success')
            .catch(() => 'failed')
        "#,
        )
        .await
        .expect("Failed to evaluate");
    assert_eq!(result, "failed", "Image should be aborted");

    // Now unroute the pattern
    page.unroute("**/*.png").await.expect("Failed to unroute");

    // Fetch should now succeed (or at least not be aborted by our handler)
    // Note: the server may 404, but the fetch itself shouldn't be aborted
    let result = page
        .evaluate_value(
            r#"
        fetch('/image.png')
            .then(r => 'status:' + r.status)
            .catch(() => 'failed')
        "#,
        )
        .await
        .expect("Failed to evaluate");
    assert!(
        result.to_string().starts_with("status:"),
        "After unroute, fetch should not be aborted (got: {})",
        result
    );

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn test_page_unroute_all_clears_all() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    // Set up multiple abort routes
    page.route("**/*.png", |route| async move { route.abort(None).await })
        .await
        .expect("Failed to set up route 1");
    page.route("**/*.css", |route| async move { route.abort(None).await })
        .await
        .expect("Failed to set up route 2");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");

    // Verify abort works
    let result = page
        .evaluate_value(
            r#"
        fetch('/image.png')
            .then(() => 'success')
            .catch(() => 'failed')
        "#,
        )
        .await
        .expect("Failed to evaluate");
    assert_eq!(result, "failed", "Image should be aborted");

    // Clear all routes
    page.unroute_all(None).await.expect("Failed to unroute all");

    // Both types should now pass through
    let result = page
        .evaluate_value(
            r#"
        fetch('/image.png')
            .then(r => 'status:' + r.status)
            .catch(() => 'failed')
        "#,
        )
        .await
        .expect("Failed to evaluate");
    assert!(
        result.to_string().starts_with("status:"),
        "After unroute_all, png fetch should not be aborted (got: {})",
        result
    );

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn a_fulfill_after_a_fallback_reaches_the_page() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let fulfilled = Arc::new(Mutex::new(None));
    let sink = fulfilled.clone();
    page.route("**/*", move |route| {
        let sink = sink.clone();
        async move {
            let options = patchright_rs::protocol::FulfillOptions::builder()
                .body_string("<p id=\"from\">handler</p>")
                .content_type("text/html")
                .build();
            let result = route.fulfill(Some(options)).await;
            *sink.lock().unwrap() = Some(result.as_ref().map_err(|e| e.to_string()).cloned());
            result
        }
    })
    .await
    .expect("Failed to set up the fulfilling route");
    page.route("**/*", |route| async move { route.fallback(None).await })
        .await
        .expect("Failed to set up the falling-back route");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");
    let delivered = read_from(&page).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(
        fulfilled.lock().unwrap().clone(),
        Some(Ok(())),
        "fulfill() after a fallback() succeeds"
    );
    assert_eq!(
        delivered, "handler",
        "the page shows the fulfilled body, not the server's"
    );
}

#[tokio::test]
async fn overrides_from_a_fallback_reach_the_server_once_every_handler_falls_back() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    page.route("**/echo-headers", |route| async move {
        route.fallback(None).await
    })
    .await
    .expect("Failed to set up the outer route");
    page.route("**/echo-headers", |route| async move {
        let headers =
            std::collections::HashMap::from([("x-fallback".to_string(), "recorded".to_string())]);
        let overrides = patchright_rs::protocol::ContinueOptions::builder()
            .headers(headers)
            .build();
        route.fallback(Some(overrides)).await
    })
    .await
    .expect("Failed to set up the inner route");

    page.goto(&format!("{}/echo-headers", server.url()), None)
        .await
        .expect("Failed to navigate");
    let received = page
        .evaluate_value(
            "JSON.parse(document.getElementById('headers').textContent)['x-fallback'] ?? 'missing'",
        )
        .await
        .expect("Failed to read the echoed headers");

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(received, "recorded");
}

#[tokio::test]
async fn a_page_fallback_hands_the_request_to_the_context_routes() {
    let server = TestServer::start().await;
    let (_pw, browser, context) = crate::common::setup_context().await;
    let page = context.new_page().await.expect("Failed to create a page");

    context
        .route("**/*", |route| async move {
            let options = patchright_rs::protocol::FulfillOptions::builder()
                .body_string("<p id=\"from\">context</p>")
                .content_type("text/html")
                .build();
            route.fulfill(Some(options)).await
        })
        .await
        .expect("Failed to set up the context route");
    page.route("**/*", |route| async move { route.fallback(None).await })
        .await
        .expect("Failed to set up the page route");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");
    let delivered = read_from(&page).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(delivered, "context");
}

#[tokio::test]
async fn fallback_on_a_handled_route_is_an_error() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let late_fallback = Arc::new(Mutex::new(None));
    let sink = late_fallback.clone();
    page.route("**/*", move |route| {
        let sink = sink.clone();
        async move {
            route.continue_(None).await?;
            *sink.lock().unwrap() = Some(route.fallback(None).await.is_err());
            Ok(())
        }
    })
    .await
    .expect("Failed to set up the route");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(
        *late_fallback.lock().unwrap(),
        Some(true),
        "fallback() after continue_() is refused"
    );
}

fn rewrite_to(url: String) -> patchright_rs::protocol::ContinueOptions {
    let headers =
        std::collections::HashMap::from([("X-Fallback".to_string(), "recorded".to_string())]);
    patchright_rs::protocol::ContinueOptions::builder()
        .url(url)
        .method("PUT".to_string())
        .headers(headers)
        .post_data("rewritten".to_string())
        .build()
}

fn fulfill_html(body: &str) -> patchright_rs::protocol::FulfillOptions {
    patchright_rs::protocol::FulfillOptions::builder()
        .body_string(body)
        .content_type("text/html")
        .build()
}

#[tokio::test]
async fn a_later_handler_sees_the_request_as_an_earlier_fallback_left_it() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let seen = Arc::new(Mutex::new(None));
    let sink = seen.clone();
    page.route("**/*", move |route| {
        let sink = sink.clone();
        async move {
            let request = route.request();
            let headers = request.headers();
            *sink.lock().unwrap() = Some((
                request.url(),
                request.method(),
                headers.get("x-fallback").cloned(),
                request.post_data(),
                request.header_value("x-fallback").await?,
            ));
            route.fulfill(fulfill_html("<p>done</p>")).await
        }
    })
    .await
    .expect("Failed to set up the outer route");
    let rewritten = format!("{}/rewritten", server.url());
    let target = rewritten.clone();
    page.route("**/original", move |route| {
        let target = target.clone();
        async move { route.fallback(Some(rewrite_to(target))).await }
    })
    .await
    .expect("Failed to set up the inner route");

    page.goto(&format!("{}/original", server.url()), None)
        .await
        .expect("Failed to navigate");

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some((
            rewritten,
            "PUT".to_string(),
            Some("recorded".to_string()),
            Some("rewritten".to_string()),
            Some("recorded".to_string()),
        ))
    );
}

#[tokio::test]
async fn a_page_handler_is_matched_against_the_url_a_fallback_rewrote() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    page.route("**/rewritten", |route| async move {
        route
            .fulfill(fulfill_html("<p id=\"from\">rewritten</p>"))
            .await
    })
    .await
    .expect("Failed to set up the rewritten route");
    let target = format!("{}/rewritten", server.url());
    page.route("**/original", move |route| {
        let target = target.clone();
        async move { route.fallback(Some(rewrite_to(target))).await }
    })
    .await
    .expect("Failed to set up the original route");

    page.goto(&format!("{}/original", server.url()), None)
        .await
        .expect("Failed to navigate");
    let delivered = read_from(&page).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(delivered, "rewritten");
}

#[tokio::test]
async fn a_context_handler_is_matched_against_the_url_a_page_fallback_rewrote() {
    let server = TestServer::start().await;
    let (_pw, browser, context) = crate::common::setup_context().await;
    let page = context.new_page().await.expect("Failed to create a page");

    context
        .route("**/rewritten", |route| async move {
            route
                .fulfill(fulfill_html("<p id=\"from\">rewritten</p>"))
                .await
        })
        .await
        .expect("Failed to set up the context route");
    let target = format!("{}/rewritten", server.url());
    page.route("**/original", move |route| {
        let target = target.clone();
        async move { route.fallback(Some(rewrite_to(target))).await }
    })
    .await
    .expect("Failed to set up the page route");

    page.goto(&format!("{}/original", server.url()), None)
        .await
        .expect("Failed to navigate");
    let delivered = read_from(&page).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(delivered, "rewritten");
}

#[tokio::test]
async fn fetch_sends_the_request_as_an_earlier_fallback_left_it() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let echoed = Arc::new(Mutex::new(None));
    let sink = echoed.clone();
    page.route("**/*", move |route| {
        let sink = sink.clone();
        async move {
            let response = route.fetch(None).await?;
            *sink.lock().unwrap() = Some(response.json::<serde_json::Value>()?);
            route.fulfill(fulfill_html("<p>done</p>")).await
        }
    })
    .await
    .expect("Failed to set up the fetching route");
    let target = format!("{}/api/echo-request", server.url());
    page.route("**/original", move |route| {
        let target = target.clone();
        async move { route.fallback(Some(rewrite_to(target))).await }
    })
    .await
    .expect("Failed to set up the rewriting route");

    page.goto(&format!("{}/original", server.url()), None)
        .await
        .expect("Failed to navigate");

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    let echoed = echoed.lock().unwrap().clone().expect("fetch() ran");
    assert_eq!(
        (
            &echoed["method"],
            &echoed["path"],
            &echoed["headers"]["x-fallback"],
            &echoed["body"],
        ),
        (
            &serde_json::json!("PUT"),
            &serde_json::json!("/api/echo-request"),
            &serde_json::json!("recorded"),
            &serde_json::json!("rewritten"),
        )
    );
}

async fn read_from(page: &patchright_rs::protocol::Page) -> String {
    page.evaluate_value("document.getElementById('from')?.textContent ?? document.body.innerText")
        .await
        .expect("Failed to read the document")
}

/// Runs `late` in a handler right after its `fallback()`, asserts that the
/// next handler still answered the request, and returns what `late` returned.
async fn after_a_fallback<F, Fut>(late: F) -> Result<(), String>
where
    F: Fn(patchright_rs::protocol::Route) -> Fut + Send + Sync + Copy + 'static,
    Fut: std::future::Future<Output = patchright_rs::Result<()>> + Send + 'static,
{
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let recorded = Arc::new(Mutex::new(None));
    let sink = recorded.clone();
    page.route("**/*", |route| async move {
        route
            .fulfill(fulfill_html(r#"<p id="from">next</p>"#))
            .await
    })
    .await
    .expect("Failed to set up the next route");
    page.route("**/*", move |route| {
        let sink = sink.clone();
        async move {
            route.fallback(None).await?;
            let result = late(route).await;
            *sink.lock().unwrap() = Some(result.map_err(|e| e.to_string()));
            Ok(())
        }
    })
    .await
    .expect("Failed to set up the falling-back route");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");
    let delivered = read_from(&page).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert_eq!(
        delivered, "next",
        "the fallback still reaches the next handler"
    );
    recorded.lock().unwrap().clone().expect("the handler ran")
}

#[tokio::test]
async fn an_answer_after_a_fallback_in_the_same_handler_is_refused() {
    let late = after_a_fallback(|route| async move {
        route
            .fulfill(fulfill_html(r#"<p id="from">same</p>"#))
            .await
    })
    .await;
    assert!(
        matches!(&late, Err(m) if m.contains("Route is already handled!")),
        "fulfill() after fallback() in one handler is refused: {late:?}"
    );
}

#[tokio::test]
async fn a_second_fallback_in_the_same_handler_is_refused() {
    let again = after_a_fallback(|route| async move { route.fallback(None).await }).await;
    assert!(
        matches!(&again, Err(m) if m.contains("Route is already handled!")),
        "a second fallback() in one handler is refused: {again:?}"
    );
}

#[tokio::test]
async fn a_handler_that_fails_after_a_fallback_has_its_request_aborted() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    page.route("**/*", |route| async move {
        route.fallback(None).await?;
        route.fulfill(fulfill_html("<p>same</p>")).await
    })
    .await
    .expect("Failed to set up the route");

    crate::common::navigate_and_expect_an_abort(&page, &format!("{}/", server.url())).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}
