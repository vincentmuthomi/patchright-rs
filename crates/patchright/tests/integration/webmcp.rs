use patchright_rs::protocol::{BrowserType, Page, Playwright};
use patchright_rs::{FulfillOptions, LaunchOptions, WebMCPTool};
use serde_json::{Value, json};
use std::collections::HashMap;

const ADD_TOOL: &str = r#"
  modelContext.registerTool({
    name: 'add',
    description: 'Adds two numbers',
    inputSchema: { type: 'object', properties: { a: { type: 'number' }, b: { type: 'number' } }, required: ['a', 'b'] },
    annotations: { readOnlyHint: true },
    async execute(input) { return { content: [{ type: 'text', text: String(input.a + input.b) }] }; },
  });
"#;

const URL: &str = "http://127.0.0.1/webmcp.html";

/// A browser of `browser_type` with WebMCP switched on, the way each engine
/// gates it.
async fn launch_with_webmcp(browser_type: &BrowserType) -> patchright_rs::protocol::Browser {
    crate::common::init_tracing();
    let options = if browser_type.name() == "firefox" {
        LaunchOptions::new().firefox_user_prefs(HashMap::from([
            ("dom.modelcontext.enabled".to_string(), json!(true)),
            ("dom.modelcontext.testing.enabled".to_string(), json!(true)),
        ]))
    } else {
        LaunchOptions::new().args(vec!["--enable-features=WebMCP".to_string()])
    };
    browser_type
        .launch_with_options(options)
        .await
        .expect("launch with WebMCP enabled")
}

/// Serves a page that registers `tools` and opens it.
async fn open_with_tools(page: &Page, tools: &str) {
    let html = format!(
        "<title>WebMCP</title><script>\
           const modelContext = document.modelContext || navigator.modelContext;\
           {tools}\
         </script>"
    );
    page.route("**/webmcp.html", move |route| {
        let html = html.clone();
        async move {
            route
                .fulfill(
                    FulfillOptions::builder()
                        .content_type("text/html")
                        .body_string(html)
                        .build(),
                )
                .await
        }
    })
    .await
    .expect("route");
    page.goto(URL, None).await.expect("goto");
}

async fn lists_and_calls_tools(browser_type: &BrowserType) {
    let browser = launch_with_webmcp(browser_type).await;
    let page = browser.new_page().await.expect("page");
    open_with_tools(
        &page,
        &format!(
            "{ADD_TOOL}
             modelContext.registerTool({{
               name: 'noop', description: 'Does nothing', async execute() {{}},
             }});
             modelContext.registerTool({{
               name: 'broken', description: 'Always fails',
               async execute() {{ return {{ content: [{{ type: 'text', text: 'nope' }}], isError: true }}; }},
             }});"
        ),
    )
    .await;

    let mut tools = page.webmcp().tools(None).await.expect("tools");
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["add", "broken", "noop"]);
    let add: &WebMCPTool = &tools[0];
    assert_eq!(add.description, "Adds two numbers");
    assert_eq!(
        add.input_schema,
        Some(json!({
            "type": "object",
            "properties": {"a": {"type": "number"}, "b": {"type": "number"}},
            "required": ["a", "b"]
        }))
    );
    assert_eq!(
        add.annotations.as_ref().and_then(|a| a.read_only),
        Some(true)
    );
    assert_eq!(
        (
            tools[2].input_schema.as_ref(),
            tools[2].annotations.as_ref()
        ),
        (None, None)
    );
    let from_frame = page
        .main_frame()
        .await
        .expect("main frame")
        .webmcp()
        .tools(None)
        .await
        .expect("frame tools");
    assert_eq!(from_frame.len(), tools.len());

    let sum: Value = page
        .webmcp()
        .call_tool("add", &json!({"a": 2, "b": 40}), None)
        .await
        .expect("call add");
    assert_eq!(sum, json!({"content": [{"type": "text", "text": "42"}]}));

    let failed: Value = page
        .webmcp()
        .call_tool("broken", &(), None)
        .await
        .expect("a tool's own failure is a result");
    assert_eq!(
        failed,
        json!({"content": [{"type": "text", "text": "nope"}], "isError": true})
    );

    let err = page
        .webmcp()
        .call_tool::<_, Value>("missing", &(), None)
        .await
        .expect_err("an unregistered tool");
    assert!(
        err.to_string()
            .contains(r#"No WebMCP tool named "missing""#),
        "{err}"
    );

    browser.close().await.expect("close");
}

#[tokio::test]
async fn webmcp_lists_and_calls_page_tools_in_chromium() {
    let playwright = Playwright::launch().await.expect("launch");
    lists_and_calls_tools(playwright.chromium()).await;
}

#[tokio::test]
#[ignore = "launches Firefox; runs in CI's --run-ignored lane"]
async fn webmcp_lists_and_calls_page_tools_in_firefox() {
    let playwright = Playwright::launch().await.expect("launch");
    lists_and_calls_tools(playwright.firefox()).await;
}

#[tokio::test]
async fn webmcp_reports_no_tools_for_a_page_without_any() {
    let playwright = Playwright::launch().await.expect("launch");
    let browser = launch_with_webmcp(playwright.chromium()).await;
    let page = browser.new_page().await.expect("page");
    open_with_tools(&page, "").await;

    assert!(page.webmcp().tools(None).await.expect("tools").is_empty());

    browser.close().await.expect("close");
}

#[tokio::test]
async fn webmcp_says_how_to_enable_it_when_it_is_off() {
    let (_pw, browser, page) = crate::common::setup().await;
    open_with_tools(&page, "").await;

    let err = page
        .webmcp()
        .tools(None)
        .await
        .expect_err("WebMCP is off without the feature flag");
    assert!(
        err.to_string().contains("--enable-features=WebMCP"),
        "{err}"
    );
    assert!(err.to_string().contains("WebMCP is not enabled"), "{err}");

    browser.close().await.expect("close");
}

#[tokio::test]
#[ignore = "launches WebKit; runs in CI's --run-ignored lane"]
async fn webmcp_is_refused_in_webkit() {
    let playwright = Playwright::launch().await.expect("launch");
    let browser = playwright.webkit().launch().await.expect("webkit");
    let page = browser.new_page().await.expect("page");
    open_with_tools(&page, "").await;

    let err = page
        .webmcp()
        .tools(None)
        .await
        .expect_err("WebKit has no WebMCP");
    assert!(err.to_string().contains("not supported in WebKit"), "{err}");

    browser.close().await.expect("close");
}
