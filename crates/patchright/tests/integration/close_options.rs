use patchright_rs::protocol::{
    BrowserCloseOptions, BrowserContextCloseOptions, Page, PageCloseOptions,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Starts an evaluation that never settles and returns once the page is
/// running it, so a close lands while the driver has it in flight.
async fn hang_an_evaluation(
    page: &Page,
) -> Result<tokio::task::JoinHandle<patchright_rs::Result<String>>, Box<dyn std::error::Error>> {
    let hung = page.clone();
    let pending = tokio::spawn(async move {
        hung.evaluate_value("window.__hung = true, new Promise(() => {})")
            .await
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while page
        .evaluate_value("String(window.__hung === true)")
        .await?
        != "true"
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the hung evaluation never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(pending)
}

async fn interrupted_error(
    pending: tokio::task::JoinHandle<patchright_rs::Result<String>>,
) -> String {
    match tokio::time::timeout(Duration::from_secs(10), pending).await {
        Ok(Ok(Err(e))) => e.to_string(),
        Ok(Ok(Ok(v))) => panic!("the hung evaluation settled with {v:?}"),
        Ok(Err(join)) => panic!("the evaluation task failed: {join}"),
        Err(_) => panic!("the close did not interrupt the hung evaluation"),
    }
}

#[tokio::test]
async fn a_page_close_reason_reaches_the_operation_it_interrupts()
-> Result<(), Box<dyn std::error::Error>> {
    let (_pw, browser, page) = crate::common::setup().await;
    let pending = hang_an_evaluation(&page).await?;

    page.close_with_options(PageCloseOptions::default().reason("page done here"))
        .await?;
    let error = interrupted_error(pending).await;

    browser.close().await?;
    assert!(error.contains("page done here"), "got: {error}");
    Ok(())
}

#[tokio::test]
async fn a_context_close_reason_reaches_the_operation_it_interrupts()
-> Result<(), Box<dyn std::error::Error>> {
    let (_pw, browser, context) = crate::common::setup_context().await;
    let page = context.new_page().await?;
    let pending = hang_an_evaluation(&page).await?;

    context
        .close_with_options(BrowserContextCloseOptions::default().reason("context done here"))
        .await?;
    let error = interrupted_error(pending).await;

    browser.close().await?;
    assert!(error.contains("context done here"), "got: {error}");
    Ok(())
}

#[tokio::test]
async fn a_browser_close_reason_reaches_the_operation_it_interrupts()
-> Result<(), Box<dyn std::error::Error>> {
    let (_pw, browser, page) = crate::common::setup().await;
    let pending = hang_an_evaluation(&page).await?;

    browser
        .close_with_options(BrowserCloseOptions::default().reason("browser done here"))
        .await?;
    let error = interrupted_error(pending).await;

    assert!(error.contains("browser done here"), "got: {error}");
    Ok(())
}

async fn page_that_asks_before_unloading() -> Result<
    (
        patchright_rs::protocol::Playwright,
        patchright_rs::protocol::Browser,
        Page,
    ),
    Box<dyn std::error::Error>,
> {
    let (pw, browser, page) = crate::common::setup().await;
    page.set_content(
        "<body>leave?<script>window.addEventListener('beforeunload', e => { \
         e.preventDefault(); e.returnValue = ''; });</script></body>",
        None,
    )
    .await?;
    // Chromium shows a beforeunload prompt only after a user gesture.
    page.locator("body").click(None).await?;
    Ok((pw, browser, page))
}

#[tokio::test]
async fn run_before_unload_with_no_handler_accepts_the_prompt_and_closes()
-> Result<(), Box<dyn std::error::Error>> {
    // The dismissing test below proves this page raises the prompt. A
    // `dialogclosed` listener cannot show it here: the page is disposed as the
    // accepted prompt lets it close, often before the event's dialog resolves.
    let (_pw, browser, page) = page_that_asks_before_unloading().await?;

    page.close_with_options(PageCloseOptions::default().run_before_unload(true))
        .await?;
    let closed = crate::common::poll_until(Duration::from_secs(10), || page.is_closed()).await;

    browser.close().await?;
    assert!(closed, "the accepted prompt lets the page close");
    Ok(())
}

#[tokio::test]
async fn run_before_unload_leaves_the_page_open_when_the_prompt_is_dismissed()
-> Result<(), Box<dyn std::error::Error>> {
    let (_pw, browser, page) = page_that_asks_before_unloading().await?;
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    page.on_dialog(move |dialog| {
        let sink = sink.clone();
        async move {
            sink.lock().unwrap().push(dialog.type_().to_string());
            dialog.dismiss().await
        }
    })
    .await?;

    page.close_with_options(PageCloseOptions::default().run_before_unload(true))
        .await?;
    crate::common::poll_until(Duration::from_secs(10), || !seen.lock().unwrap().is_empty()).await;
    let still_usable = page
        .evaluate_value("document.body.firstChild.textContent")
        .await;
    let closed = page.is_closed();

    browser.close().await?;
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["beforeunload".to_string()]
    );
    assert!(!closed, "a dismissed prompt keeps the page open");
    assert_eq!(still_usable?, "leave?");
    Ok(())
}
