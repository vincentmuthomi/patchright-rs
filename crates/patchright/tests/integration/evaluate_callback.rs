use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

#[tokio::test]
async fn test_evaluate_with_callback_round_trips_through_rust() {
    let (_pw, browser, page) = crate::common::setup().await;
    page.set_content("<div>cb</div>", None)
        .await
        .expect("set_content");

    // The expression awaits the Rust closure and builds on its result, so a
    // pass proves the whole loop: JS call -> Rust args -> Rust return -> JS.
    let result: i64 = page
        .evaluate_with_callback("async cb => (await cb(20, 22)) + 1", |args| async move {
            let a = args[0].as_i64().unwrap_or(0);
            let b = args[1].as_i64().unwrap_or(0);
            serde_json::json!(a + b)
        })
        .await
        .expect("evaluate_with_callback");

    assert_eq!(result, 43);
    browser.close().await.expect("close");
}

#[tokio::test]
async fn test_evaluate_with_callback_is_not_installed_on_window() {
    let (_pw, browser, page) = crate::common::setup().await;
    page.set_content("<div>cb</div>", None)
        .await
        .expect("set_content");

    // The binding is registered noGlobal: the page reaches the closure only
    // through the argument. Registering any binding installs Playwright's
    // bindings-controller infrastructure globals, so the precise claim is
    // that no window property carries the callback's name, not that the
    // global count is unchanged.
    let received: bool = page
        .evaluate_with_callback("cb => typeof cb === 'function'", |_args| async move {
            serde_json::json!(null)
        })
        .await
        .expect("evaluate_with_callback");
    assert!(received, "the expression should receive a function");

    let named_globals: i64 = page
        .evaluate(
            "() => Object.getOwnPropertyNames(window).filter(k => k.startsWith('__pw_fn_rs_')).length",
            None::<&()>,
        )
        .await
        .expect("probe window for the callback name");
    assert_eq!(
        named_globals, 0,
        "noGlobal must keep the callback off window"
    );
    browser.close().await.expect("close");
}

#[tokio::test]
async fn test_evaluate_with_callback_survives_for_later_calls() {
    let (_pw, browser, page) = crate::common::setup().await;
    page.set_content("<div>cb</div>", None)
        .await
        .expect("set_content");

    let seen = Arc::new(AtomicI64::new(0));
    let seen_in_cb = seen.clone();

    // Stash the function; the binding must outlive the evaluate call that
    // delivered it, because that is what event-listener use looks like.
    let _: bool = page
        .evaluate_with_callback("cb => { window.__stash = cb; return true; }", move |args| {
            let seen = seen_in_cb.clone();
            async move {
                seen.store(args[0].as_i64().unwrap_or(0), Ordering::SeqCst);
                serde_json::json!(null)
            }
        })
        .await
        .expect("stash the callback");

    let _: serde_json::Value = page
        .evaluate("() => window.__stash(7)", None::<&()>)
        .await
        .expect("invoke the stashed callback");

    let delivered = crate::common::poll_until(std::time::Duration::from_secs(5), || {
        seen.load(Ordering::SeqCst) == 7
    })
    .await;
    assert!(delivered, "the stashed callback should still reach Rust");
    browser.close().await.expect("close");
}
