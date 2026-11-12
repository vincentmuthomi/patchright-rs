// Copyright 2026 Paul Adamson
// Licensed under the Apache License, Version 2.0
//
// WebMCP: the tools a page registers through the browser's experimental
// model-context API, listed and called the way an agent would.
//
// See: https://playwright.dev/docs/api/class-webmcp

use crate::error::{Error, Result};
use crate::protocol::Frame;
use crate::protocol::evaluate_conversion::{parse_result, serialize_argument};
use crate::server::channel_owner::ChannelOwner;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// The tools a page registers through the experimental
/// [WebMCP](https://github.com/webmachinelearning/webmcp) browser API, so a
/// test can list and call them as an agent would.
///
/// Reached through [`Page::webmcp`](crate::protocol::Page::webmcp) (the main
/// frame's tools) or [`Frame::webmcp`].
///
/// The API is behind a flag in every browser that has it. Launch Chromium
/// with `--enable-features=WebMCP` in `args`, or Firefox with the
/// `dom.modelcontext.enabled` and `dom.modelcontext.testing.enabled`
/// preferences set in `firefox_user_prefs`. Without them, [`tools`](Self::tools)
/// fails with `WebMCP is not enabled`, followed by how to enable it; WebKit
/// does not support it at all and fails with `WebMCP is not supported in
/// WebKit`.
///
/// # Example
///
/// ```no_run
/// use patchright_rs::LaunchOptions;
/// use patchright_rs::protocol::Playwright;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let playwright = Playwright::launch().await?;
/// let browser = playwright
///     .chromium()
///     .launch_with_options(LaunchOptions::new().args(vec!["--enable-features=WebMCP".into()]))
///     .await?;
/// let page = browser.new_page().await?;
/// page.goto("https://example.com/app", None).await?;
///
/// for tool in page.webmcp().tools(None).await? {
///     println!("{}: {}", tool.name, tool.description);
/// }
/// let sum: serde_json::Value = page
///     .webmcp()
///     .call_tool("add", &serde_json::json!({ "a": 2, "b": 40 }), None)
///     .await?;
/// # Ok(())
/// # }
/// ```
///
/// See: <https://playwright.dev/docs/api/class-webmcp>
#[derive(Clone, Debug)]
pub struct WebMCP {
    frame: Frame,
}

/// One tool a page registered, as [`WebMCP::tools`] reports it.
///
/// See: <https://playwright.dev/docs/api/class-webmcp#webmcp-tools>
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WebMCPTool {
    /// The name [`WebMCP::call_tool`] takes.
    pub name: String,
    /// What the page says the tool does.
    pub description: String,
    /// The JSON Schema of the tool's input, when the page gave one.
    pub input_schema: Option<Value>,
    /// The hints the page gave about the tool, when it gave any.
    pub annotations: Option<WebMCPToolAnnotations>,
}

/// Hints a page gives about a tool. Each is reported only when the page set
/// it to true.
///
/// See: <https://playwright.dev/docs/api/class-webmcp#webmcp-tools>
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WebMCPToolAnnotations {
    /// The tool does not change anything (the page's `readOnlyHint`).
    pub read_only: Option<bool>,
    /// The tool's output may hold content the page does not vouch for.
    pub untrusted_content: Option<bool>,
    /// The tool has effects beyond the page.
    pub consequential: Option<bool>,
}

/// Options for [`WebMCP::tools`] and [`WebMCP::call_tool`].
///
/// See: <https://playwright.dev/docs/api/class-webmcp#webmcp-call-tool>
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct WebMCPOptions {
    /// Maximum time in milliseconds. Defaults to the page's default timeout
    /// (see `Page::set_default_timeout`), and to
    /// [`DEFAULT_TIMEOUT_MS`](crate::DEFAULT_TIMEOUT_MS) without one.
    pub timeout: Option<f64>,
}

impl WebMCPOptions {
    /// Maximum time in milliseconds.
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// The timeout a call sends: the option, else the page's default, else the
/// crate's. The driver treats an absent timeout as no deadline at all.
fn timeout_of(options: Option<WebMCPOptions>, page_default: Option<f64>) -> f64 {
    options
        .and_then(|o| o.timeout)
        .or(page_default)
        .unwrap_or(crate::DEFAULT_TIMEOUT_MS)
}

impl WebMCP {
    pub(crate) fn new(frame: Frame) -> Self {
        Self { frame }
    }

    fn timeout(&self, options: Option<WebMCPOptions>) -> f64 {
        timeout_of(options, self.frame.page().map(|p| p.default_timeout_ms()))
    }

    /// Lists the tools the frame's page has registered, in the browser's
    /// order.
    ///
    /// # Errors
    ///
    /// Returns error if WebMCP is not enabled in this browser (see the type
    /// docs), or the frame has been detached.
    ///
    /// See: <https://playwright.dev/docs/api/class-webmcp#webmcp-tools>
    #[tracing::instrument(level = "debug", skip_all, fields(count = tracing::field::Empty))]
    pub async fn tools(
        &self,
        options: impl Into<Option<WebMCPOptions>>,
    ) -> Result<Vec<WebMCPTool>> {
        #[derive(Deserialize)]
        struct ToolsResponse {
            tools: Vec<WebMCPTool>,
        }
        let response: ToolsResponse = self
            .frame
            .channel()
            .send(
                "webmcpTools",
                json!({ "timeout": self.timeout(options.into()) }),
            )
            .await?;
        tracing::Span::current().record("count", response.tools.len());
        Ok(response.tools)
    }

    /// Calls the tool registered as `name` with `input`, and returns what it
    /// returned, deserialized as `R` (use [`serde_json::Value`] to take it
    /// as it is). A tool reports its own failure in its result, with
    /// `isError` set, rather than raising one, so such a result comes back
    /// like any other.
    ///
    /// `input` is serialized the way an `evaluate` argument is; pass `&()`
    /// for a tool that takes none.
    ///
    /// # Errors
    ///
    /// Returns error if no tool is registered as `name`
    /// (`No WebMCP tool named "..."`), WebMCP is not enabled, or the result
    /// does not deserialize as `R`.
    ///
    /// See: <https://playwright.dev/docs/api/class-webmcp#webmcp-call-tool>
    #[tracing::instrument(level = "debug", skip_all, fields(name = %name))]
    pub async fn call_tool<I, R>(
        &self,
        name: &str,
        input: &I,
        options: impl Into<Option<WebMCPOptions>>,
    ) -> Result<R>
    where
        I: serde::Serialize + ?Sized,
        R: DeserializeOwned,
    {
        // Serialize up front: the shared argument encoder turns a value
        // serde rejects into null, which would call the tool with no input.
        let input = serde_json::to_value(input).map_err(|e| {
            Error::InvalidArgument(format!("WebMCP tool input does not serialize: {e}"))
        })?;
        #[derive(Deserialize)]
        struct CallToolResponse {
            result: Value,
        }
        let response: CallToolResponse = self
            .frame
            .channel()
            .send(
                "webmcpCallTool",
                json!({
                    "name": name,
                    "input": serialize_argument(&input),
                    "timeout": self.timeout(options.into()),
                }),
            )
            .await?;
        serde_json::from_value(parse_result(&response.result)).map_err(|e| {
            Error::ProtocolError(format!("WebMCP tool result deserialization failed: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_reads_back_whatever_the_page_gave_it() {
        let tools: Vec<WebMCPTool> = serde_json::from_value(json!([
            {
                "name": "add",
                "description": "Adds two numbers",
                "inputSchema": {"type": "object"},
                "annotations": {"readOnly": true}
            },
            {"name": "noop", "description": "Does nothing"}
        ]))
        .unwrap();

        assert_eq!(tools[0].input_schema, Some(json!({"type": "object"})));
        assert_eq!(
            tools[0].annotations,
            Some(WebMCPToolAnnotations {
                read_only: Some(true),
                ..Default::default()
            })
        );
        assert_eq!(
            (tools[1].input_schema.clone(), tools[1].annotations.clone()),
            (None, None)
        );
    }

    #[test]
    fn the_timeout_is_the_option_else_the_page_default_else_the_crate_default() {
        let given = Some(WebMCPOptions::default().timeout(5.0));
        assert_eq!(timeout_of(given, Some(2.0)), 5.0);
        assert_eq!(timeout_of(Some(WebMCPOptions::default()), Some(2.0)), 2.0);
        assert_eq!(timeout_of(None, None), crate::DEFAULT_TIMEOUT_MS);
    }
}
