use parking_lot::Mutex;
use std::io::{self, Write};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

/// Captures every line written by the tracing fmt layer.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn dump(&self) -> String {
        String::from_utf8_lossy(&self.0.lock()).into_owned()
    }
}

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn install_subscriber(level: &str) -> (Capture, tracing::subscriber::DefaultGuard) {
    let cap = Capture::default();
    // FmtSpan::CLOSE prints one line per span when it closes, including all
    // recorded fields. Without it, #[instrument] spans never produce output
    // because the methods don't emit explicit info!/debug! events inside.
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(cap.clone())
        .with_target(true)
        .with_ansi(false)
        .with_span_events(FmtSpan::CLOSE);
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new(level))
        .with(layer);
    let guard = tracing::subscriber::set_default(subscriber);
    (cap, guard)
}

#[tokio::test]
async fn test_tracing_info_span_emitted_for_top_level_op() {
    let (cap, _guard) = install_subscriber("patchright_rs=info");
    let (_pw, browser, page) = crate::common::setup().await;

    page.goto("data:text/html,<h1>hello</h1>", None)
        .await
        .expect("goto failed");

    browser.close().await.expect("close failed");

    let out = cap.dump();
    assert!(
        out.contains("page::goto") || out.contains("goto"),
        "expected a goto span in output:\n{out}"
    );
}

#[tokio::test]
async fn test_tracing_debug_span_emitted_for_interior_op() {
    let (cap, _guard) = install_subscriber("patchright_rs=debug");
    let (_pw, browser, page) = crate::common::setup().await;

    page.set_content("<html><body><p id='x'>hi</p></body></html>", None)
        .await
        .expect("set_content failed");

    let _ = page.locator("#x").text_content().await;

    browser.close().await.expect("close failed");

    let out = cap.dump();
    assert!(
        out.contains("locator") && out.contains("text_content"),
        "expected a debug-level locator/text_content span in output:\n{out}"
    );
}

#[tokio::test]
async fn test_tracing_records_completion_field_via_span_record() {
    // Locator::count always records its result via tracing::Span::current().record("count", n).
    // (Goto would also record `status`, but data: URLs return None — no Response, no record —
    // and standing up a real HTTP server here would be overkill for a single field check.)
    let (cap, _guard) = install_subscriber("patchright_rs=debug");
    let (_pw, browser, page) = crate::common::setup().await;

    page.set_content("<ul><li>a</li><li>b</li><li>c</li></ul>", None)
        .await
        .expect("set_content failed");

    let n = page.locator("li").count().await.expect("count failed");
    assert_eq!(n, 3);

    browser.close().await.expect("close failed");

    let out = cap.dump();
    assert!(
        out.contains("count=3"),
        "expected count=3 recorded on locator.count span:\n{out}"
    );
}

#[tokio::test]
async fn test_tracing_trace_spans_cover_rpc_layer() {
    let (cap, _guard) = install_subscriber("patchright_rs=trace");
    let (_pw, browser, page) = crate::common::setup().await;

    page.goto("data:text/html,<h1>hi</h1>", None)
        .await
        .expect("goto failed");

    browser.close().await.expect("close failed");

    let out = cap.dump();
    assert!(
        out.contains("send_message") && out.contains("method="),
        "expected a trace-level send_message span carrying the RPC method:\n{out}"
    );
    assert!(
        out.contains("transport_send") && out.contains("bytes_len="),
        "expected a trace-level transport_send span carrying the frame size:\n{out}"
    );
}

#[tokio::test]
async fn a_span_around_launch_closes_when_the_caller_drops_it() {
    use tracing::Instrument;

    let (cap, _guard) = install_subscriber("tracing_emission_test=info");
    let span = tracing::info_span!(target: "tracing_emission_test", "around_launch");
    let (_pw, browser, _page) = crate::common::setup().instrument(span).await;

    let out = cap.dump();
    browser.close().await.expect("close failed");

    assert!(
        out.lines()
            .any(|l| l.contains("around_launch") && l.contains(" close ")),
        "a task started by launch is still holding the caller's span open:\n{out}"
    );
}

/// Enables only the read loop's `dispatch` span (and its events), so a
/// handler that inherits any span at all can only have inherited that one.
const DISPATCH_ONLY: &str = "patchright_rs::server::connection=trace";

type SeenSpan = Arc<Mutex<Option<Option<&'static str>>>>;

fn record_current_span(seen: &SeenSpan) {
    *seen.lock() = Some(tracing::Span::current().metadata().map(|m| m.name()));
}

#[tokio::test]
async fn page_close_handler_runs_inside_the_dispatch_span() {
    let (_cap, _guard) = install_subscriber(DISPATCH_ONLY);
    let (_pw, browser, page) = crate::common::setup().await;

    let seen = SeenSpan::default();
    let sink = seen.clone();
    page.on_close(move || {
        let sink = sink.clone();
        async move {
            record_current_span(&sink);
            Ok(())
        }
    })
    .await
    .expect("on_close failed");

    page.close().await.expect("page close failed");
    crate::common::poll_until(std::time::Duration::from_secs(5), || seen.lock().is_some()).await;
    browser.close().await.expect("close failed");

    assert_eq!(*seen.lock(), Some(Some("dispatch")));
}

#[tokio::test]
async fn context_page_handler_runs_inside_the_dispatch_span() {
    let (_cap, _guard) = install_subscriber(DISPATCH_ONLY);
    let (_pw, browser, context) = crate::common::setup_context().await;

    let seen = SeenSpan::default();
    let sink = seen.clone();
    context
        .on_page(move |_page| {
            let sink = sink.clone();
            async move {
                record_current_span(&sink);
                Ok(())
            }
        })
        .await
        .expect("on_page failed");

    context.new_page().await.expect("new_page failed");
    crate::common::poll_until(std::time::Duration::from_secs(5), || seen.lock().is_some()).await;
    browser.close().await.expect("close failed");

    assert_eq!(*seen.lock(), Some(Some("dispatch")));
}
