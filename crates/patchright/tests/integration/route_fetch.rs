use crate::test_server::TestServer;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn test_route_fetch_basic() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let fetch_status = Arc::new(Mutex::new(0u16));
    let status_clone = fetch_status.clone();

    page.route("**/", move |route| {
        let status = status_clone.clone();
        async move {
            // Fetch the actual response via APIRequestContext
            let response = route.fetch(None).await?;
            *status.lock().unwrap() = response.status();

            // Fulfill with the fetched status
            route
                .fulfill(Some(
                    patchright_rs::FulfillOptions::builder()
                        .status(response.status())
                        .build(),
                ))
                .await
        }
    })
    .await
    .expect("Failed to set up route");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");

    let status = *fetch_status.lock().unwrap();
    assert_eq!(status, 200, "Fetched response should have status 200");

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn test_route_fetch_response_methods() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let response_ok = Arc::new(Mutex::new(false));
    let has_body = Arc::new(Mutex::new(false));
    let ok_clone = response_ok.clone();
    let body_clone = has_body.clone();

    page.route("**/", move |route| {
        let ok = ok_clone.clone();
        let body = body_clone.clone();
        async move {
            let response = route.fetch(None).await?;

            // Test FetchResponse methods
            *ok.lock().unwrap() = response.ok();
            *body.lock().unwrap() = !response.body().is_empty();

            assert!(response.ok(), "200 response should be ok()");
            assert!(!response.headers().is_empty(), "Should have headers");

            route
                .fulfill(Some(
                    patchright_rs::FulfillOptions::builder()
                        .status(response.status())
                        .build(),
                ))
                .await
        }
    })
    .await
    .expect("Failed to set up route");

    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");

    assert!(*response_ok.lock().unwrap(), "Response should have been ok");
    assert!(*has_body.lock().unwrap(), "Response should have a body");

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn test_route_fetch_with_context_route() {
    let server = TestServer::start().await;
    let (_pw, browser, context) = crate::common::setup_context().await;

    let fetch_status = Arc::new(Mutex::new(0u16));
    let status_clone = fetch_status.clone();

    // Use context-level route with fetch
    context
        .route("**/", move |route| {
            let status = status_clone.clone();
            async move {
                let response = route.fetch(None).await?;
                *status.lock().unwrap() = response.status();
                route
                    .fulfill(Some(
                        patchright_rs::FulfillOptions::builder()
                            .status(response.status())
                            .build(),
                    ))
                    .await
            }
        })
        .await
        .expect("Failed to set up context route");

    let page = context.new_page().await.expect("Failed to create page");
    page.goto(&format!("{}/", server.url()), None)
        .await
        .expect("Failed to navigate");

    let status = *fetch_status.lock().unwrap();
    assert_eq!(
        status, 200,
        "Context-level route.fetch() should get status 200"
    );

    context.close().await.expect("Failed to close context");
    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

fn handler_gave_up() -> patchright_rs::Error {
    patchright_rs::Error::ProtocolError("the handler gave up".to_string())
}

#[tokio::test]
async fn a_page_handler_that_fails_after_fetch_aborts_the_request() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    page.route("**/", |route| async move {
        route.fetch(None).await?;
        Err(handler_gave_up())
    })
    .await
    .expect("Failed to set up route");

    crate::common::navigate_and_expect_an_abort(&page, &format!("{}/", server.url())).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn a_context_handler_that_fails_after_fetch_aborts_the_request() {
    let server = TestServer::start().await;
    let (_pw, browser, context) = crate::common::setup_context().await;
    let page = context.new_page().await.expect("Failed to create a page");

    context
        .route("**/", |route| async move {
            route.fetch(None).await?;
            Err(handler_gave_up())
        })
        .await
        .expect("Failed to set up route");

    crate::common::navigate_and_expect_an_abort(&page, &format!("{}/", server.url())).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

async fn fetch_and_keep(route: patchright_rs::protocol::Route) -> patchright_rs::Result<()> {
    route.fetch(None).await?;
    Ok(())
}

async fn fetch_then_fall_back(route: patchright_rs::protocol::Route) -> patchright_rs::Result<()> {
    route.fetch(None).await?;
    route.fallback(None).await
}

async fn answer_from_the_next_handler(
    route: patchright_rs::protocol::Route,
) -> patchright_rs::Result<()> {
    route
        .fulfill(patchright_rs::FulfillOptions::builder().status(203).build())
        .await
}

async fn navigate_and_expect_the_next_handler(page: &patchright_rs::protocol::Page, url: &str) {
    let response = crate::common::goto_within(page, url, 5)
        .await
        .expect("the next handler answers the request")
        .expect("a navigation has a response");
    assert_eq!(response.status(), 203);
}

#[tokio::test]
async fn a_handler_can_fetch_then_fall_back_to_the_next_page_handler() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    page.route("**/", answer_from_the_next_handler)
        .await
        .expect("Failed to set up the later route");
    page.route("**/", fetch_then_fall_back)
        .await
        .expect("Failed to set up the fetching route");

    navigate_and_expect_the_next_handler(&page, &format!("{}/", server.url())).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

#[tokio::test]
async fn a_page_handler_can_fetch_then_fall_back_to_the_context() {
    let server = TestServer::start().await;
    let (_pw, browser, context) = crate::common::setup_context().await;
    let page = context.new_page().await.expect("Failed to create a page");

    context
        .route("**/", answer_from_the_next_handler)
        .await
        .expect("Failed to set up the context route");
    page.route("**/", fetch_then_fall_back)
        .await
        .expect("Failed to set up the fetching route");

    navigate_and_expect_the_next_handler(&page, &format!("{}/", server.url())).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
}

// The first handler's fallback must not carry over to the second, whose
// fetch keeps the route so the browser does not send the request twice.
#[tokio::test]
async fn a_handler_that_fetches_takes_the_route_over_after_an_earlier_fallback() {
    let server = TestServer::start().await;
    let (_pw, browser, page) = crate::common::setup().await;

    let last_ran = Arc::new(Mutex::new(false));
    let sink = last_ran.clone();
    page.route("**/", move |route| {
        let sink = sink.clone();
        async move {
            *sink.lock().unwrap() = true;
            route.continue_(None).await
        }
    })
    .await
    .expect("Failed to set up the last route");
    page.route("**/", fetch_and_keep)
        .await
        .expect("Failed to set up the fetching route");
    page.route("**/", fetch_then_fall_back)
        .await
        .expect("Failed to set up the falling-back route");

    let navigation = crate::common::goto_within(&page, &format!("{}/", server.url()), 2).await;

    browser.close().await.expect("Failed to close browser");
    server.shutdown();
    assert!(
        matches!(navigation, Err(patchright_rs::Error::Timeout(_))),
        "a fetch the handler never answers leaves the request waiting: {navigation:?}"
    );
    assert!(
        !*last_ran.lock().unwrap(),
        "no later handler runs, so the request is not sent a second time"
    );
}
