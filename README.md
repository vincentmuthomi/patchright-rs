# patchright-rs

**Patchright for Rust**: an undetected, drop-in version of Playwright for Rust,
the way [patchright-python](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright-python)
and [patchright-nodejs](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright-nodejs)
are for their languages.

It is [playwright-rust](https://github.com/padamson/playwright-rust)'s full
Playwright API, with two changes:

1. It runs the **patched Patchright driver** (`patchright-core` from npm), not
   Microsoft's `playwright-core`. All of patchright's stealth patches live in that
   driver.
2. It carries **patchright's client-side patches**, ported to Rust: evaluation in
   isolated worlds by default, init scripts delivered through document responses,
   and binding names the patched driver accepts.

> [!IMPORTANT]
> As with every Patchright package, only **Chromium-based** browsers are patched.
> Firefox and WebKit still launch, but get no stealth patches.

## Setup

1. **Install Rust 1.95 or newer.**

   ```sh
   rustup update stable
   rustc --version
   ```

2. **Get this repository.** The crate is not on crates.io yet, so it is used
   from a checkout.

   ```sh
   git clone https://github.com/vincentmuthomi/patchright-rs
   ```

3. **Add it to your project** as a path dependency, next to tokio:

   ```toml
   [dependencies]
   patchright-rs = { path = "../patchright-rs/crates/patchright" }
   tokio = { version = "1", features = ["full"] }
   ```

4. **Build once with network access.** The first `cargo build` downloads the
   driver: `patchright-core` and a pinned Node.js, both checked against pinned
   digests. It goes in `<user cache>/patchright-rust/<version>/`
   (`%LOCALAPPDATA%` on Windows, `~/.cache` on Linux, `~/Library/Caches` on
   macOS) and is reused by every project on the machine, so later builds work
   offline.

5. **Install the matching Chromium.** Run this inside the patchright-rs
   checkout, once per driver version:

   ```sh
   cargo run -p patchright-rs --features cli --bin patchright-rs -- install chromium
   # or: <driver dir>/node <driver dir>/package/cli.js install chromium
   ```

6. **Check that it works.** Still in the checkout:

   ```sh
   cargo run -p patchright-rs --example basic          # prints the title of example.com
   cargo test -p patchright-rs --test patchright       # the patchright behaviour, end to end
   ```

For the recommended undetected configuration, also install Google Chrome and
see [the undetected setup](#best-practice-the-undetected-setup) below.

## Using the API

**The API is playwright-rs's API, used directly.** Every public type, function
and option of [playwright-rs](https://github.com/padamson/playwright-rust)
0.20.1 exists here with the same name and signature, so its
[documentation](https://docs.rs/playwright-rs) and examples apply as written.
The only change in your code is the crate name in the import:

```rust
// use playwright_rs::protocol::Playwright;
use patchright_rs::protocol::Playwright;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let playwright = Playwright::launch().await?;
    let browser = playwright.chromium().launch().await?;
    let page = browser.new_page().await?;
    page.goto("https://example.com", None).await?;
    println!("{}", page.title().await?);
    browser.close().await?;
    Ok(())
}
```

To move an existing playwright-rs project over, replace the `playwright-rs`
dependency with `patchright-rs` and rename `playwright_rs` to `patchright_rs`
in your `use` lines. [`crates/patchright/examples/`](crates/patchright/examples)
holds playwright-rs's examples unchanged apart from that import.

What differs is behaviour, not the API, and only where patchright itself
differs from Playwright:

- `evaluate` and its relatives run in an isolated world by default; see
  [`isolated_context`](#isolated_context-choosing-the-execution-context).
- `page.on_console` and page-error events do not fire.
- Init scripts reach `http(s)` documents only; see [Init scripts](#init-scripts).
- WebSocket routes do not work with the patched driver.

Patchright adds two things on top:
[`IsolatedContextExt`](#isolated_context-choosing-the-execution-context) and
[`focus_control`](#focus_control-the-pages-real-focus-state).

### Best practice: the undetected setup

Patchright recommends real Chrome, run headed, with a persistent profile, no viewport
override, and **no** custom user agent or headers.
[`examples/stealth.rs`](crates/patchright/examples/stealth.rs) shows the setup:

```rust
let context = playwright
    .chromium()
    .launch_persistent_context_with_options(
        "/path/to/profile",
        BrowserContextOptions::builder()
            .channel("chrome".to_string())
            .headless(false)
            .no_viewport(true)
            .build(),
    )
    .await?;
```

## The extended Patchright API

### `isolated_context`: choosing the execution context

Every `evaluate`, `evaluate_handle`, `evaluate_all` and `evaluate_value` on `Page`,
`Frame`, `Locator`, `Worker`, `JSHandle` and `ElementHandle` runs in an **isolated
world** by default. This is patchright's `isolatedContext = true`. Page scripts
can't see these evaluations, but the evaluations can't see the page's JavaScript
globals either. Both worlds share the DOM.

To run code in the page's **main world**, wrap the call with
`IsolatedContextExt`:

```rust
use patchright_rs::IsolatedContextExt;

// Isolated (default): sees the DOM, not `window.appState`.
let title: String = page.evaluate_value("document.title").await?;

// Main world: sees the page's own globals.
let state: String = page
    .evaluate_value("JSON.stringify(window.appState)")
    .main_world()
    .await?;

// Equivalent to patchright-python's `isolated_context=False`:
let n: i64 = page.evaluate::<(), i64>("() => window.counter", None)
    .isolated_context(false)
    .await?;
```

The scope applies to every evaluation the wrapped future makes, so one `async`
block can cover several calls. Tasks spawned inside it are not covered.

Python and Node.js add an extra `isolated_context` argument to every evaluate
method instead. Rust has no default arguments, so here it is one switch that
covers all of them.

### `focus_control`: the page's real focus state

By default the driver emulates focus, so every page reports itself focused.
`focus_control(true)` on `BrowserContextOptions` turns that emulation off for
the context. It is patchright's `focus_control` option and works with
`new_context_with_options` and `launch_persistent_context_with_options`:

```rust
let context = browser
    .new_context_with_options(BrowserContextOptions::builder().focus_control(true).build())
    .await?;
```

## What you get from the driver

These come from the patched driver, unchanged from patchright:

- **No `Runtime.enable` leak.** JavaScript runs in execution contexts patchright
  creates itself.
- **No `Console.enable` leak.** The console domain is never enabled, so
  **`page.on_console` events do not fire**. This is the same trade-off as every
  Patchright package.
- **Launch flags:** adds `--disable-blink-features=AutomationControlled`; removes
  `--disable-popup-blocking`, `--disable-component-update`,
  `--disable-default-apps`, `--disable-extensions`,
  `--disable-client-side-phishing-detection`,
  `--disable-component-extensions-with-background-pages`,
  `--allow-pre-commit-input`, `--disable-ipc-flooding-protection`,
  `--metrics-recording-only`, `--unsafely-disable-devtools-self-xss-warnings`
  and `--disable-back-forward-cache`.
- **Closed shadow roots:** normal locators, XPath included, reach into closed
  shadow roots.
- **General leaks:** the other fixes in [patchright's
  README](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright#patches).

### Init scripts

Without `Runtime.enable`, the driver injects init scripts into HTML document
responses. The first `add_init_script`, `expose_function`, `expose_binding`,
`clock().install()` or `tracing().start()` on a page or context installs a
catch-all `route("**/*")`. That route flags `http(s)` document requests for
injection and passes every request on with `fallback()`.

This is patchright's own mechanism, so it has the same
[caveats](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright#bugs):

- Init scripts only reach `http(s)` documents. `set_content`, `data:` and
  `about:blank` pages don't get them.
- If your own route handler fulfills or continues a document request itself,
  the init scripts are not injected into it.

## How it differs from playwright-rust

The whole client diff is small and marked with `PATCHRIGHT` comments
(`grep -rn PATCHRIGHT crates/`):

| Where | Change | Upstream patchright equivalent |
|---|---|---|
| `src/patchright.rs` | `isolatedContext` injection, `IsolatedContextExt`, injection route handler | `client_patches/{frame,page,locator,worker,jsHandle}Patch.ts`, `installInjectRoute` |
| `src/server/channel.rs` | `Channel::send` adds `isolatedContext` to `evaluateExpression`, `evaluateExpressionHandle` and `evalOnSelectorAll` | the same `client_patches` |
| `src/protocol/{page,browser_context}/{bindings,network}.rs` | install the injection route before init scripts and bindings | `browserContextPatch.ts`, `pagePatch.ts` |
| `src/protocol/{clock,tracing}.rs` | same, for `clock.install` and `tracing.start` | `clockPatch.ts`, `tracingPatch.ts` |
| `src/protocol/{route,route_params}.rs` | the `patchrightInitScript` override on `Route.continue` | `networkPatch.ts` |
| `src/protocol/request.rs`, `src/protocol/page/lifecycle.rs` | `all_headers()` fails once `close()` was called on the page | `networkPatch.ts` (`allHeaders`) |
| `src/protocol/browser_context/options.rs` | the `focus_control` context option | patchright-python's `focus_control`; the driver's `focusControl` |
| `src/protocol/page/evaluate.rs` | callback bindings named `f` + 32 hex digits | `jsHandlePatch.ts`, `utilityScriptSerializersPatch.ts` |
| `src/protocol/browser_type.rs` | Windows "CI stability" flags (`--no-sandbox`, `--disable-web-security`, …) need `PATCHRIGHT_WINDOWS_CI_FLAGS`, not `CI` | — (Rust-only leak) |
| `build.rs`, `src/build_support/` | driver assembled from `patchright-core`, cached under `patchright-rust/` | — |

These upstream client patches needed no port:

- **Auto-dismissing dialogs nobody listens to:** playwright-rust already does this.
- **`initScriptSourceWithExposedFunctions`:** JS-only (function values in init-script args).
- **`addSourceUrlToScript`:** playwright-rust never appends `//# sourceURL=`.
- **The `waitForURL` tweak:** playwright-rust's `wait_for_url` already checks the current URL first.

Unchanged from playwright-rust: the crate layout, the `playwright-rs-macros`
(`locator!`) and `playwright-rs-trace` crates, the environment variables
(`PLAYWRIGHT_DRIVER_PATH`, `PLAYWRIGHT_NODE_EXE`, `PLAYWRIGHT_CLI_JS`,
`PLAYWRIGHT_BROWSERS_PATH`, …) and the whole public API.

## Versions

| | |
|---|---|
| Driver | `patchright-core` **1.64.3** (Playwright 1.64.1 + patchright), Chromium 156 |
| Client | forked from playwright-rust `0.20.1` ([`fd24e29`](https://github.com/padamson/playwright-rust/commit/fd24e29a725939fe0cc432e3ddece1158e7cd267)), built for driver 1.65.0 |
| Node | 26.13.0 |
| Rust | 1.95+ (MSRV inherited from playwright-rust) |

The client is one minor version ahead of the driver. playwright-rust's own
1.64→1.65 protocol diff found a single change, an optional
`supportsLoneSurrogates` on `initialize` that the client never sends, so the pair
is wire-compatible. When `patchright-core` 1.65 ships, bump it
(see below).

### Updating

- **New `patchright-core`:** run `cargo xtask update-driver`. It looks up the
  latest stable release on npm and updates the pinned version, its npm
  integrity, and the vendored [`protocol-spec/`](protocol-spec/README.md) in
  one step (`--version <v>` pins a specific release instead). It commits
  nothing: review `git diff protocol-spec`, then run
  `cargo test -p patchright-rs --lib`, reinstall Chromium, and run
  `cargo test -p patchright-rs --test patchright`.
- **New playwright-rust:** merge the upstream release into `crates/`, keeping
  the `PATCHRIGHT` hunks. Then check that every evaluate still goes through
  `Channel::send` (`grep -rn '"evaluateExpression\|"evalOnSelectorAll' crates/patchright/src`).

## Testing

```sh
cargo test -p patchright-rs --lib                 # unit tests, no browser
cargo test -p patchright-rs --test patchright     # patchright behaviour, end to end
cargo test -p patchright-rs --test integration    # playwright-rust's full suite
```

`tests/patchright.rs` checks the behaviour that makes this patchright:

- the isolated world is the default
- init scripts and exposed functions reach `http` documents
- callbacks survive the patched serializer
- `navigator.webdriver` is unset
- locators reach into closed shadow roots

Four of these tests fail against stock `playwright-core`, which shows they
measure the patches.

## License

Apache-2.0, like both upstream projects. See [NOTICE](NOTICE).
This project is not affiliated with Microsoft, playwright-rust or Patchright.
