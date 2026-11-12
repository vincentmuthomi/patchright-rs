//! End-to-end tests of patchright's behaviour: the patched driver together
//! with this crate's client-side patches (`src/patchright.rs`).
//!
//! Needs the bundled driver and its Chromium (`patchright-rs install
//! chromium`, or `node <driver>/package/cli.js install chromium`).

#![allow(clippy::disallowed_methods)]

use axum::Router;
use axum::response::Html;
use axum::routing::get;
use patchright_rs::IsolatedContextExt;
use patchright_rs::protocol::{Browser, Page, Playwright};

const PAGE: &str = r#"<!doctype html>
<html><head><title>patchright-rs</title>
<script>window.pageGlobal = "main";</script>
</head><body><div id="greeting">hello</div></body></html>"#;

/// An `http://` page: init-script injection only applies to http(s)
/// documents, which `set_content` and `about:blank` are not.
async fn serve() -> String {
    let app = Router::new().route("/", get(|| async { Html(PAGE) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

async fn launch() -> (Playwright, Browser) {
    let playwright = Playwright::launch()
        .await
        .expect("launch the patchright driver");
    let browser = playwright
        .chromium()
        .launch()
        .await
        .expect("launch Chromium (installed for the bundled driver?)");
    (playwright, browser)
}

async fn eval<R: serde::de::DeserializeOwned>(page: &Page, expression: &str) -> R {
    page.evaluate::<(), R>(expression, None)
        .await
        .unwrap_or_else(|e| panic!("evaluate {expression}: {e}"))
}

#[tokio::test]
async fn evaluate_runs_in_an_isolated_world_by_default() {
    let (_pw, browser) = launch().await;
    let page = browser.new_page().await.unwrap();
    page.goto(&serve().await, None).await.unwrap();

    // The isolated world shares the DOM...
    let text: String = eval(
        &page,
        "() => document.querySelector('#greeting').textContent",
    )
    .await;
    assert_eq!(text, "hello");
    // ...but not the page's JavaScript globals.
    let isolated: String = eval(&page, "() => typeof window.pageGlobal").await;
    assert_eq!(isolated, "undefined");

    let main: String = page
        .evaluate::<(), String>("() => typeof window.pageGlobal", None)
        .main_world()
        .await
        .unwrap();
    assert_eq!(main, "string");

    // Locator::evaluate goes through the same switch.
    let via_locator: String = page
        .locator("#greeting")
        .evaluate::<String, ()>("e => typeof window.pageGlobal", None)
        .main_world()
        .await
        .unwrap();
    assert_eq!(via_locator, "string");

    browser.close().await.unwrap();
}

#[tokio::test]
async fn init_scripts_reach_http_documents() {
    let (_pw, browser) = launch().await;
    let context = browser.new_context().await.unwrap();
    context
        .add_init_script("window.contextInit = 42;")
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.add_init_script("window.pageInit = 'yes';")
        .await
        .unwrap();
    page.goto(&serve().await, None).await.unwrap();

    let (from_context, from_page): (i64, String) = async {
        let a = eval(&page, "() => window.contextInit").await;
        let b = eval(&page, "() => window.pageInit").await;
        (a, b)
    }
    .main_world()
    .await;
    assert_eq!(from_context, 42);
    assert_eq!(from_page, "yes");

    browser.close().await.unwrap();
}

#[tokio::test]
async fn exposed_functions_are_callable_from_the_page() {
    let (_pw, browser) = launch().await;
    let page = browser.new_page().await.unwrap();
    page.expose_function("rsAdd", |args| async move {
        let sum: i64 = args.iter().filter_map(|v| v.as_i64()).sum();
        serde_json::json!(sum)
    })
    .await
    .unwrap();
    page.goto(&serve().await, None).await.unwrap();

    let sum: i64 = page
        .evaluate::<(), i64>("async () => await window.rsAdd(2, 3)", None)
        .main_world()
        .await
        .unwrap();
    assert_eq!(sum, 5);

    browser.close().await.unwrap();
}

#[tokio::test]
async fn evaluate_with_callback_round_trips() {
    let (_pw, browser) = launch().await;
    let page = browser.new_page().await.unwrap();
    page.goto(&serve().await, None).await.unwrap();

    let result: i64 = page
        .evaluate_with_callback("async cb => (await cb(20, 22)) + 1", |args| async move {
            let sum: i64 = args.iter().filter_map(|v| v.as_i64()).sum();
            serde_json::json!(sum)
        })
        .await
        .unwrap();
    assert_eq!(result, 43);

    browser.close().await.unwrap();
}

#[tokio::test]
async fn navigator_webdriver_is_not_set() {
    let (_pw, browser) = launch().await;
    let page = browser.new_page().await.unwrap();
    page.goto(&serve().await, None).await.unwrap();

    let webdriver: bool = page
        .evaluate::<(), bool>("() => navigator.webdriver === true", None)
        .main_world()
        .await
        .unwrap();
    assert!(!webdriver, "navigator.webdriver leaks automation");

    browser.close().await.unwrap();
}

#[tokio::test]
async fn locators_reach_into_closed_shadow_roots() {
    let (_pw, browser) = launch().await;
    let page = browser.new_page().await.unwrap();
    page.goto(&serve().await, None).await.unwrap();
    page.evaluate::<(), ()>(
        r#"() => {
            const host = document.createElement("div");
            document.body.appendChild(host);
            const root = host.attachShadow({ mode: "closed" });
            root.innerHTML = '<span id="secret">inside closed root</span>';
        }"#,
        None,
    )
    .main_world()
    .await
    .unwrap();

    let text = page.locator("#secret").text_content().await.unwrap();
    assert_eq!(text.as_deref(), Some("inside closed root"));

    browser.close().await.unwrap();
}
