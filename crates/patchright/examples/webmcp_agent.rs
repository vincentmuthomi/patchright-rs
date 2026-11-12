// WebMCP agent loop - act on a page through the tools it publishes
//
// Shows: the two ways an agent driving a browser can act on a page. A page
// that publishes WebMCP tools hands the agent structured tools (a name, a
// description and an input JSON Schema, the same three fields a model's tool
// list takes), so the agent calls a tool instead of reading the page. A page
// that publishes none falls back to an `ai`-mode aria snapshot, whose refs
// `get_by_ref` turns back into locators.
//
// No model or API key is involved: where an agent would send the tool list to
// a model and run the call it chooses, this example prints the list and makes
// the call a model would plausibly make.
//
// WebMCP is experimental: Chromium needs `--enable-features=WebMCP`, Firefox
// the `dom.modelcontext.enabled` and `dom.modelcontext.testing.enabled`
// preferences, and WebKit does not support it.
//
// Usage: cargo run --example webmcp_agent

use patchright_rs::protocol::{AriaSnapshotMode, AriaSnapshotOptions, Page, Playwright};
use patchright_rs::{FulfillOptions, LaunchOptions};
use serde_json::{Value, json};

/// A to-do app that publishes two tools for agents.
const TODO_APP: &str = r#"<!doctype html>
<title>Todos</title>
<ul id="todos"></ul>
<script>
  const list = document.getElementById('todos');
  const modelContext = document.modelContext || navigator.modelContext;
  modelContext?.registerTool({
    name: 'add_todo',
    description: 'Add an item to the to-do list.',
    inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
    async execute({ text }) {
      const item = document.createElement('li');
      item.textContent = text;
      list.append(item);
      return { content: [{ type: 'text', text: `Added "${text}".` }] };
    },
  });
  modelContext?.registerTool({
    name: 'list_todos',
    description: 'The items on the to-do list.',
    annotations: { readOnlyHint: true },
    async execute() {
      const items = [...list.children].map(li => li.textContent);
      return { content: [{ type: 'text', text: JSON.stringify(items) }] };
    },
  });
</script>"#;

/// The same kind of page with no tools: a plain form an agent has to read.
const PLAIN_APP: &str = r#"<!doctype html>
<title>Newsletter</title>
<button onclick="this.textContent = 'Subscribed'">Subscribe</button>"#;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let playwright = Playwright::launch().await?;
    let browser = playwright
        .chromium()
        .launch_with_options(LaunchOptions::new().args(vec!["--enable-features=WebMCP".into()]))
        .await?;

    println!("== A page that publishes tools ==");
    let page = browser.new_page().await?;
    open(&page, TODO_APP).await?;
    act(&page).await?;

    println!("\n== A page that publishes none ==");
    let page = browser.new_page().await?;
    open(&page, PLAIN_APP).await?;
    act(&page).await?;

    browser.close().await?;
    Ok(())
}

/// One step of an agent: use the page's tools when it has any, and read the
/// page when it has none.
async fn act(page: &Page) -> Result<(), Box<dyn std::error::Error>> {
    let tools = page.webmcp().tools(None).await?;
    if tools.is_empty() {
        return act_through_the_page(page).await;
    }

    // What the agent would hand its model: the page's tools, field for field.
    let tool_list: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema.clone().unwrap_or(json!({ "type": "object" })),
            })
        })
        .collect();
    println!(
        "Tools for the model:\n{}",
        serde_json::to_string_pretty(&tool_list)?
    );

    // The calls a model would make for "remember to buy milk, then show me
    // the list".
    let added: Value = page
        .webmcp()
        .call_tool("add_todo", &json!({ "text": "Buy milk" }), None)
        .await?;
    println!("add_todo -> {}", added["content"][0]["text"]);
    let listed: Value = page.webmcp().call_tool("list_todos", &(), None).await?;
    println!("list_todos -> {}", listed["content"][0]["text"]);

    // The tool acted on the page itself, as a user's click would have.
    println!(
        "The page now lists: {:?}",
        page.locator("#todos li").all_text_contents().await?
    );
    Ok(())
}

/// The fallback: read the page as an `ai`-mode snapshot, pick an element by
/// the ref the snapshot gave it, and act on it.
async fn act_through_the_page(page: &Page) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = page
        .aria_snapshot(AriaSnapshotOptions::default().mode(AriaSnapshotMode::Ai))
        .await?;
    println!("No tools, so the agent reads the page:\n{snapshot}");

    // A model would pick the ref; here, the Subscribe button's.
    let subscribe = snapshot
        .lines()
        .find(|line| line.contains(r#"button "Subscribe""#))
        .and_then(|line| line.split("[ref=").nth(1))
        .and_then(|rest| rest.split(']').next())
        .ok_or("the snapshot names the Subscribe button")?;
    page.get_by_ref(subscribe).click(None).await?;
    println!(
        "Clicked [ref={subscribe}]; the button now reads {:?}",
        page.locator("button").text_content().await?
    );
    Ok(())
}

/// Serve `html` at a local origin (WebMCP needs a secure context, which
/// `127.0.0.1` is) and open it.
async fn open(page: &Page, html: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    page.route("**/app", move |route| async move {
        route
            .fulfill(
                FulfillOptions::builder()
                    .content_type("text/html")
                    .body_string(html)
                    .build(),
            )
            .await
    })
    .await?;
    page.goto("http://127.0.0.1/app", None).await?;
    Ok(())
}
