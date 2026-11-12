//! Patchright's client-side behaviour, ported from
//! [patchright-nodejs](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright-nodejs)'s
//! `client_patches/`.
//!
//! Most of patchright lives in the driver: this crate runs the patched
//! `patchright-core` server (see `build.rs`), which drops `Runtime.enable` and
//! `Console.enable`, fixes the leaky launch flags, and reaches into closed
//! shadow roots. The client has to cooperate in two places, and both are
//! here:
//!
//! 1. **Execution context.** The patched server evaluates JavaScript in the
//!    page's main world unless a call carries `isolatedContext: true`. Upstream
//!    patchright makes the isolated world the default for every `evaluate`,
//!    `evaluate_handle`, and `evaluate_all` on `Page`, `Frame`, `Locator`,
//!    `Worker`, `JSHandle` and `ElementHandle`, so page scripts cannot observe
//!    the automation. This crate does the same: [`Channel::send`] adds the
//!    parameter to every such call. Use [`IsolatedContextExt::main_world`] to
//!    run one in the main world, for example to read a page's global
//!    variables.
//!
//! 2. **Init scripts.** Without `Runtime.enable` the server cannot install
//!    init scripts through the usual CDP hooks. It injects them into HTML
//!    document responses instead, and it does so for a request only when the
//!    client continues it with `patchrightInitScript: true`. So the first
//!    `add_init_script`, `expose_function`, `expose_binding`, `clock().install`
//!    or `tracing().start` on a page or context installs a catch-all
//!    `route("**/*")` that flags `http(s)` document requests and passes every
//!    request on with `fallback()`. Your own routes still run first (handlers
//!    run newest-first), and a request your handler answers itself is not
//!    injected, exactly as in patchright for Python and Node.js.
//!
//! [`Channel::send`]: crate::server::channel::Channel::send

use crate::error::Result;
use crate::protocol::{ContinueOptions, Route};
use serde_json::Value;
use std::future::Future;

tokio::task_local! {
    /// The execution-context choice of the enclosing
    /// [`IsolatedContextExt::isolated_context`] scope, if any.
    static ISOLATED_CONTEXT: bool;
}

/// Protocol methods that take patchright's `isolatedContext` parameter (the
/// patched driver declares it on `Frame`, `JSHandle` and `Worker`).
const ISOLATABLE_METHODS: [&str; 3] = [
    "evaluateExpression",
    "evaluateExpressionHandle",
    "evalOnSelectorAll",
];

/// The execution context a JavaScript evaluation in the current task uses:
/// `true` (the isolated world, patchright's default) unless an enclosing
/// [`IsolatedContextExt`] scope says otherwise.
pub fn current_isolated_context() -> bool {
    ISOLATED_CONTEXT
        .try_with(|isolated| *isolated)
        .unwrap_or(true)
}

/// Adds `isolatedContext` to the params of an evaluate-style call, unless the
/// call site set it already. Called from [`Channel::send`] for every outgoing
/// call, so no evaluate path can miss it.
///
/// [`Channel::send`]: crate::server::channel::Channel::send
pub(crate) fn apply_isolated_context(method: &str, params: &mut Value) {
    if !ISOLATABLE_METHODS.contains(&method) {
        return;
    }
    let isolated = Value::Bool(current_isolated_context());
    match params {
        Value::Object(map) => {
            map.entry("isolatedContext").or_insert(isolated);
        }
        Value::Null => {
            *params = serde_json::json!({ "isolatedContext": isolated });
        }
        _ => {}
    }
}

/// Chooses the JavaScript execution context for the evaluations inside a
/// future: patchright's `isolatedContext` option.
///
/// Every `evaluate*` call runs in an isolated world by default, which page
/// scripts cannot see into, and which cannot see the page's own JavaScript
/// globals either (it shares the DOM only). Wrap a call in
/// [`main_world`](Self::main_world) when it needs the page's globals:
///
/// ```no_run
/// use patchright_rs::IsolatedContextExt;
/// # async fn example(page: &patchright_rs::Page) -> patchright_rs::Result<()> {
/// // Isolated world (the default): sees the DOM, not `window.appState`.
/// let title: String = page.evaluate_value("document.title").await?;
///
/// // Main world: sees the page's own globals.
/// let state: String = page
///     .evaluate_value("JSON.stringify(window.appState)")
///     .main_world()
///     .await?;
/// # Ok(()) }
/// ```
///
/// The choice applies to every evaluation the wrapped future makes on its
/// own task, so a block of calls can share one scope:
///
/// ```no_run
/// use patchright_rs::IsolatedContextExt;
/// # async fn example(page: &patchright_rs::Page) -> patchright_rs::Result<()> {
/// async {
///     page.evaluate_expression("window.a = 1").await?;
///     page.evaluate_expression("window.b = 2").await?;
///     Ok::<_, patchright_rs::Error>(())
/// }
/// .main_world()
/// .await?;
/// # Ok(()) }
/// ```
///
/// Evaluations in a task spawned from inside the future are not covered;
/// wrap the spawned future itself.
pub trait IsolatedContextExt: Future + Sized {
    /// Runs this future's evaluations in the isolated world (`true`, the
    /// default) or in the page's main world (`false`).
    fn isolated_context(self, isolated: bool) -> tokio::task::futures::TaskLocalFuture<bool, Self> {
        ISOLATED_CONTEXT.scope(isolated, self)
    }

    /// Runs this future's evaluations in the page's main world, where the
    /// page's own JavaScript globals are visible. Shorthand for
    /// `isolated_context(false)`.
    fn main_world(self) -> tokio::task::futures::TaskLocalFuture<bool, Self> {
        self.isolated_context(false)
    }
}

impl<F: Future> IsolatedContextExt for F {}

/// The route handler patchright installs on the first init script or
/// binding of a page or context (`installInjectRoute` upstream): it flags
/// `http(s)` document requests for init-script injection and passes every
/// request on to the next handler.
pub(crate) async fn inject_route_handler(route: Route) -> Result<()> {
    let request = route.request();
    let inject = request.resource_type() == "document" && request.url().starts_with("http");
    let overrides = inject.then(|| ContinueOptions {
        patchright_init_script: Some(true),
        ..ContinueOptions::default()
    });
    // Upstream wraps the flagged fallback in try/catch and falls back plainly
    // when it throws.
    match route.fallback(overrides).await {
        Ok(()) => Ok(()),
        Err(_) => route.fallback(None).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn evaluate_calls_default_to_the_isolated_world() {
        for method in ISOLATABLE_METHODS {
            let mut params = json!({ "expression": "1" });
            apply_isolated_context(method, &mut params);
            assert_eq!(params["isolatedContext"], json!(true), "{method}");
        }
    }

    #[test]
    fn other_calls_are_left_alone() {
        let mut params = json!({ "url": "https://example.com" });
        apply_isolated_context("goto", &mut params);
        assert_eq!(params, json!({ "url": "https://example.com" }));
        let mut params = json!({ "selector": "div", "expression": "e => e" });
        apply_isolated_context("evalOnSelector", &mut params);
        assert!(params.get("isolatedContext").is_none());
    }

    #[test]
    fn an_explicit_choice_at_the_call_site_wins() {
        let mut params = json!({ "expression": "1", "isolatedContext": false });
        apply_isolated_context("evaluateExpression", &mut params);
        assert_eq!(params["isolatedContext"], json!(false));
    }

    #[tokio::test]
    async fn main_world_scope_turns_isolation_off() {
        let inside = async {
            let mut params = json!({ "expression": "1" });
            apply_isolated_context("evaluateExpression", &mut params);
            params["isolatedContext"].clone()
        }
        .main_world()
        .await;
        assert_eq!(inside, json!(false));
        assert!(current_isolated_context());
    }

    #[tokio::test]
    async fn the_innermost_scope_wins() {
        let inner = async {
            async { current_isolated_context() }
                .isolated_context(true)
                .await
        }
        .main_world()
        .await;
        assert!(inner);
    }
}
