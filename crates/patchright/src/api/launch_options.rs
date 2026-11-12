// Launch options for BrowserType::launch()
//
// This module provides options for launching browsers, matching the Playwright API exactly.
// See: https://playwright.dev/docs/api/class-browsertype#browser-type-launch

use crate::protocol::proxy::ProxySettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Options for launching a browser
///
/// All options are optional and will use Playwright's defaults if not specified.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct LaunchOptions {
    /// Additional arguments to pass to browser instance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    /// Directory to write artifacts (traces, video, downloads) to.
    /// Surfaces Playwright's `artifactsDir` option; if unset, Playwright
    /// chooses a temp directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts_dir: Option<String>,

    /// Browser distribution channel (e.g., "chrome", "msedge")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,

    /// Enable Chromium sandboxing (default: false on Linux)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromium_sandbox: Option<bool>,

    /// Auto-open DevTools.
    ///
    /// Playwright 1.58 removed this option. The driver drops undeclared
    /// parameters without erroring, so setting it has no effect and no longer
    /// reaches the wire. Nor does it imply a headed browser any more. For
    /// Chromium, pass `--auto-open-devtools-for-tabs` in `args` and set
    /// `headless(false)`.
    #[deprecated(
        since = "0.20.0",
        note = "removed from the protocol in Playwright 1.58; setting it has no effect. For Chromium, pass `--auto-open-devtools-for-tabs` in `args` instead"
    )]
    #[serde(skip_serializing)]
    pub devtools: Option<bool>,

    /// Directory to save downloads
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads_path: Option<String>,

    /// Environment variables for browser process
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,

    /// Path to custom browser executable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,

    /// Firefox user preferences (Firefox only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firefox_user_prefs: Option<HashMap<String, Value>>,

    /// Close browser on SIGHUP (default: true)
    #[serde(rename = "handleSIGHUP", skip_serializing_if = "Option::is_none")]
    pub handle_sighup: Option<bool>,

    /// Close browser on SIGINT/Ctrl-C (default: true)
    #[serde(rename = "handleSIGINT", skip_serializing_if = "Option::is_none")]
    pub handle_sigint: Option<bool>,

    /// Close browser on SIGTERM (default: true)
    #[serde(rename = "handleSIGTERM", skip_serializing_if = "Option::is_none")]
    pub handle_sigterm: Option<bool>,

    /// Run in headless mode (default: true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headless: Option<bool>,

    /// Filter or disable default browser arguments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_default_args: Option<IgnoreDefaultArgs>,

    /// Network proxy settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,

    /// Slow down operations by N milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slow_mo: Option<f64>,

    /// Timeout for browser launch in milliseconds (default:
    /// [`DEFAULT_LAUNCH_TIMEOUT_MS`](crate::DEFAULT_LAUNCH_TIMEOUT_MS))
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,

    /// Directory to save traces
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traces_dir: Option<String>,
}

/// Filter or disable default browser arguments
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum IgnoreDefaultArgs {
    /// Ignore all default arguments
    Bool(bool),
    /// Filter specific default arguments
    Array(Vec<String>),
}

impl LaunchOptions {
    /// Creates a new LaunchOptions with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set additional arguments to pass to browser instance
    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = Some(args);
        self
    }

    /// Set the directory to write artifacts (traces, video, downloads) to
    pub fn artifacts_dir(mut self, path: String) -> Self {
        self.artifacts_dir = Some(path);
        self
    }

    /// Set browser distribution channel
    pub fn channel(mut self, channel: String) -> Self {
        self.channel = Some(channel);
        self
    }

    /// Enable or disable Chromium sandboxing
    pub fn chromium_sandbox(mut self, enabled: bool) -> Self {
        self.chromium_sandbox = Some(enabled);
        self
    }

    /// Auto-open DevTools. Playwright 1.58 removed the option, so this has no effect.
    #[deprecated(
        since = "0.20.0",
        note = "removed from the protocol in Playwright 1.58; setting it has no effect. For Chromium, pass `--auto-open-devtools-for-tabs` in `args` instead"
    )]
    pub fn devtools(mut self, enabled: bool) -> Self {
        #[allow(deprecated)]
        {
            self.devtools = Some(enabled);
        }
        self
    }

    /// Set directory to save downloads
    pub fn downloads_path(mut self, path: String) -> Self {
        self.downloads_path = Some(path);
        self
    }

    /// Set environment variables for browser process
    pub fn env(mut self, env: HashMap<String, String>) -> Self {
        self.env = Some(env);
        self
    }

    /// Set path to custom browser executable
    pub fn executable_path(mut self, path: String) -> Self {
        self.executable_path = Some(path);
        self
    }

    /// Set Firefox user preferences (Firefox only)
    pub fn firefox_user_prefs(mut self, prefs: HashMap<String, Value>) -> Self {
        self.firefox_user_prefs = Some(prefs);
        self
    }

    /// Set whether to close browser on SIGHUP
    pub fn handle_sighup(mut self, enabled: bool) -> Self {
        self.handle_sighup = Some(enabled);
        self
    }

    /// Set whether to close browser on SIGINT/Ctrl-C
    pub fn handle_sigint(mut self, enabled: bool) -> Self {
        self.handle_sigint = Some(enabled);
        self
    }

    /// Set whether to close browser on SIGTERM
    pub fn handle_sigterm(mut self, enabled: bool) -> Self {
        self.handle_sigterm = Some(enabled);
        self
    }

    /// Run in headless mode
    pub fn headless(mut self, enabled: bool) -> Self {
        self.headless = Some(enabled);
        self
    }

    /// Filter or disable default browser arguments
    pub fn ignore_default_args(mut self, args: IgnoreDefaultArgs) -> Self {
        self.ignore_default_args = Some(args);
        self
    }

    /// Set network proxy settings
    pub fn proxy(mut self, proxy: ProxySettings) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Slow down operations by N milliseconds
    pub fn slow_mo(mut self, ms: f64) -> Self {
        self.slow_mo = Some(ms);
        self
    }

    /// Set timeout for browser launch in milliseconds
    pub fn timeout(mut self, ms: f64) -> Self {
        self.timeout = Some(ms);
        self
    }

    /// Set directory to save traces
    pub fn traces_dir(mut self, path: String) -> Self {
        self.traces_dir = Some(path);
        self
    }

    /// Normalize options for protocol transmission
    ///
    /// This performs transformations required by the Playwright protocol:
    /// 1. Set a default timeout if none is given. `Connection::send_message`
    ///    moves it into the message metadata, and without one the server
    ///    gives the launch no deadline.
    /// 2. Convert env HashMap to array of {name, value} objects
    /// 3. Convert bool ignoreDefaultArgs to ignoreAllDefaultArgs
    ///
    /// This matches the behavior of playwright-python's parameter normalization.
    pub(crate) fn normalize(self) -> Value {
        let mut value =
            serde_json::to_value(&self).expect("serialization of LaunchOptions cannot fail");

        // A launch with no timeout has no deadline on the server.
        if value.get("timeout").is_none() {
            value["timeout"] = json!(crate::DEFAULT_LAUNCH_TIMEOUT_MS);
        }

        // Convert env HashMap to array of {name, value} objects
        if let Some(env_map) = value.get_mut("env")
            && let Some(map) = env_map.as_object()
        {
            let env_array: Vec<_> = map
                .iter()
                .map(|(k, v)| json!({"name": k, "value": v}))
                .collect();
            *env_map = json!(env_array);
        }

        // Convert bool ignoreDefaultArgs to ignoreAllDefaultArgs
        if let Some(ignore) = value.get("ignoreDefaultArgs")
            && let Some(b) = ignore.as_bool()
        {
            if b {
                value["ignoreAllDefaultArgs"] = json!(true);
            }
            value
                .as_object_mut()
                .expect("params is a JSON object")
                .remove("ignoreDefaultArgs");
        }

        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_launch_options_default() {
        let opts = LaunchOptions::default();
        assert!(opts.headless.is_none());
        assert!(opts.args.is_none());
    }

    #[test]
    fn every_key_sent_is_a_parameter_the_driver_declares() {
        use crate::protocol_spec::{BROWSER_TYPE, command_parameters, keys};

        // A literal with no `..Default::default()`, so a new field fails to
        // compile here until it is added and checked.
        let options = LaunchOptions {
            args: Some(vec![]),
            artifacts_dir: Some(String::new()),
            channel: Some(String::new()),
            chromium_sandbox: Some(false),
            #[allow(deprecated)]
            devtools: Some(false),
            downloads_path: Some(String::new()),
            env: Some(HashMap::new()),
            executable_path: Some(String::new()),
            firefox_user_prefs: Some(HashMap::new()),
            handle_sighup: Some(false),
            handle_sigint: Some(false),
            handle_sigterm: Some(false),
            headless: Some(true),
            ignore_default_args: Some(IgnoreDefaultArgs::Array(vec![])),
            proxy: Some(ProxySettings::new("http://proxy")),
            slow_mo: Some(0.0),
            timeout: Some(0.0),
            traces_dir: Some(String::new()),
        };

        let sent = keys(&options.normalize());
        let declared = command_parameters(BROWSER_TYPE, "launch");
        let undeclared: Vec<&str> = sent
            .iter()
            .filter(|key| !declared.contains(*key))
            .map(String::as_str)
            .collect();
        // timeout moves into the message metadata, where the server takes a
        // call's deadline from.
        assert_eq!(undeclared, ["timeout"]);
    }

    #[test]
    fn a_launch_without_a_timeout_gets_three_minutes() {
        assert_eq!(
            LaunchOptions::default().normalize()["timeout"],
            json!(180000.0)
        );
        assert_eq!(
            LaunchOptions::default().timeout(5000.0).normalize()["timeout"],
            json!(5000.0)
        );
    }

    #[test]
    fn devtools_still_deserializes_but_is_never_sent() {
        let options: LaunchOptions = serde_json::from_value(json!({"devtools": true})).unwrap();
        #[allow(deprecated)]
        let devtools = options.devtools;
        assert_eq!(devtools, Some(true));
        assert!(options.normalize().get("devtools").is_none());
    }

    #[test]
    fn signal_options_use_the_driver_spelling() {
        let value = LaunchOptions::default()
            .handle_sighup(false)
            .handle_sigint(true)
            .handle_sigterm(false)
            .normalize();
        assert_eq!(value["handleSIGHUP"], json!(false));
        assert_eq!(value["handleSIGINT"], json!(true));
        assert_eq!(value["handleSIGTERM"], json!(false));
    }

    #[test]
    fn test_launch_options_builder() {
        let opts = LaunchOptions::default()
            .headless(false)
            .slow_mo(100.0)
            .args(vec!["--no-sandbox".to_string()]);

        assert_eq!(opts.headless, Some(false));
        assert_eq!(opts.slow_mo, Some(100.0));
        assert_eq!(opts.args, Some(vec!["--no-sandbox".to_string()]));
    }

    #[test]
    fn test_launch_options_normalize_env() {
        let opts = LaunchOptions::default().env(HashMap::from([
            ("FOO".to_string(), "bar".to_string()),
            ("BAZ".to_string(), "qux".to_string()),
        ]));

        let normalized = opts.normalize();

        // Verify env is converted to array format
        assert!(normalized["env"].is_array());
        let env_array = normalized["env"].as_array().unwrap();
        assert_eq!(env_array.len(), 2);

        // Check that both env vars are present (order may vary)
        let names: Vec<_> = env_array
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"FOO"));
        assert!(names.contains(&"BAZ"));
    }

    #[test]
    fn test_launch_options_normalize_ignore_default_args_bool() {
        let opts = LaunchOptions::default().ignore_default_args(IgnoreDefaultArgs::Bool(true));

        let normalized = opts.normalize();

        // Verify bool is converted to ignoreAllDefaultArgs
        assert!(normalized["ignoreAllDefaultArgs"].as_bool().unwrap());
        assert!(normalized.get("ignoreDefaultArgs").is_none());
    }

    #[test]
    fn test_launch_options_normalize_ignore_default_args_array() {
        let opts = LaunchOptions::default()
            .ignore_default_args(IgnoreDefaultArgs::Array(vec!["--foo".to_string()]));

        let normalized = opts.normalize();

        // Verify array is preserved
        assert!(normalized["ignoreDefaultArgs"].is_array());
        assert_eq!(
            normalized["ignoreDefaultArgs"][0].as_str().unwrap(),
            "--foo"
        );
    }

    #[test]
    fn test_proxy_settings() {
        let proxy = ProxySettings {
            server: "http://proxy:8080".to_string(),
            bypass: Some("localhost,127.0.0.1".to_string()),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
        };

        let opts = LaunchOptions::default().proxy(proxy);
        assert!(opts.proxy.is_some());
    }

    #[test]
    fn test_builder_pattern_chaining() {
        let opts = LaunchOptions::new()
            .headless(true)
            .slow_mo(50.0)
            .timeout(60000.0)
            .args(vec![
                "--no-sandbox".to_string(),
                "--disable-gpu".to_string(),
            ])
            .channel("chrome".to_string());

        assert_eq!(opts.headless, Some(true));
        assert_eq!(opts.slow_mo, Some(50.0));
        assert_eq!(opts.timeout, Some(60000.0));
        assert_eq!(opts.args.as_ref().unwrap().len(), 2);
        assert_eq!(opts.channel, Some("chrome".to_string()));
    }
}
