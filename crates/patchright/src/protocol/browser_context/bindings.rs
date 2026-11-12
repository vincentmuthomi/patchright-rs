use super::{BindingCallback, BindingCallbackFuture, BrowserContext};
use crate::error::Result;
use crate::server::channel_owner::ChannelOwner;
use serde_json::Value;
use std::future::Future;
use std::sync::Arc;

/// Init scripts, exposed functions and bindings.
impl BrowserContext {
    /// Adds a script which would be evaluated in one of the following scenarios:
    ///
    /// - Whenever a page is created in the browser context or is navigated.
    /// - Whenever a child frame is attached or navigated in any page in the browser context.
    ///
    /// The script is evaluated after the document was created but before any of its scripts
    /// were run. This is useful to amend the JavaScript environment, e.g. to seed Math.random.
    ///
    /// # Arguments
    ///
    /// * `script` - Script to be evaluated in all pages in the browser context.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Context has been closed
    /// - Communication with browser process fails
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-add-init-script>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid()))]
    pub async fn add_init_script(&self, script: &str) -> Result<()> {
        // PATCHRIGHT: init scripts are injected into document responses.
        self.install_inject_route().await?;
        self.channel()
            .send_no_result("addInitScript", serde_json::json!({ "source": script }))
            .await
    }

    /// Exposes a Rust function to every page in this browser context as
    /// `window[name]` in JavaScript.
    ///
    /// When JavaScript code calls `window[name](arg1, arg2, …)` the Playwright
    /// server fires a `bindingCall` event that invokes `callback` with the
    /// deserialized arguments. The return value of `callback` is serialized back
    /// to JavaScript so the `await window[name](…)` expression resolves with it.
    ///
    /// The binding is injected into every existing page and every new page
    /// created in this context.
    ///
    /// # Arguments
    ///
    /// * `name`     – JavaScript identifier that will be available as `window[name]`.
    /// * `callback` – Async closure called with `Vec<serde_json::Value>` (the JS
    ///   arguments) and returning `serde_json::Value` (the result).
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The context has been closed.
    /// - Communication with the browser process fails.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-expose-function>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid(), name = %name))]
    pub async fn expose_function<F, Fut>(&self, name: &str, callback: F) -> Result<()>
    where
        F: Fn(Vec<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = serde_json::Value> + Send + 'static,
    {
        self.expose_binding_internal(name, callback).await
    }

    /// Exposes a Rust function to every page in this browser context as
    /// `window[name]` in JavaScript.
    ///
    /// Currently identical to [`expose_function`](Self::expose_function):
    /// arguments arrive as plain serialized values. Upstream Playwright's
    /// `exposeBinding` can additionally hand the callback a source
    /// (page/frame) descriptor, which this crate does not surface yet.
    ///
    /// # Arguments
    ///
    /// * `name`     – JavaScript identifier.
    /// * `callback` – Async closure with `Vec<serde_json::Value>` → `serde_json::Value`.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The context has been closed.
    /// - Communication with the browser process fails.
    ///
    /// See: <https://playwright.dev/docs/api/class-browsercontext#browser-context-expose-binding>
    #[tracing::instrument(level = "debug", skip_all, fields(guid = %self.guid(), name = %name))]
    pub async fn expose_binding<F, Fut>(&self, name: &str, callback: F) -> Result<()>
    where
        F: Fn(Vec<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = serde_json::Value> + Send + 'static,
    {
        self.expose_binding_internal(name, callback).await
    }

    /// Internal implementation shared by expose_function and expose_binding.
    async fn expose_binding_internal<F, Fut>(&self, name: &str, callback: F) -> Result<()>
    where
        F: Fn(Vec<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = serde_json::Value> + Send + 'static,
    {
        // PATCHRIGHT: bindings ride on the init-script injection.
        self.install_inject_route().await?;
        // Wrap callback with type erasure
        let callback: BindingCallback = Arc::new(move |args: Vec<serde_json::Value>| {
            Box::pin(callback(args)) as BindingCallbackFuture
        });

        // Store the callback before sending the RPC so that a race-condition
        // where a bindingCall arrives before we finish registering is avoided.
        self.binding_callbacks
            .lock()
            .unwrap()
            .insert(name.to_string(), callback);

        // Tell the Playwright server to inject window[name] into every page.
        //
        // The protocol also accepts `noGlobal`, which suppresses that
        // injection. It is deliberately not exposed: it exists to support
        // passing functions as evaluate arguments, where the binding is
        // called through the bindings controller rather than off `window`.
        self.channel()
            .send_no_result("exposeBinding", serde_json::json!({ "name": name }))
            .await
    }

    /// Deserializes binding call arguments from Playwright's protocol format.
    ///
    /// The `args` field in the BindingCall initializer is a JSON array of
    /// type-tagged values; each is parsed into plain JSON. `Page`'s
    /// `bindingCall` dispatch shares it.
    pub(crate) fn deserialize_binding_args(raw_args: &Value) -> Vec<Value> {
        let Some(arr) = raw_args.as_array() else {
            return vec![];
        };

        arr.iter()
            .map(|arg| {
                // Each arg is a direct Playwright type-tagged value, e.g. {"n": 3} or {"s": "hello"}
                // (NOT wrapped in {"value": ..., "handles": []} — that format is only for evaluate args)
                crate::protocol::evaluate_conversion::parse_value(arg, None)
            })
            .collect()
    }
}
