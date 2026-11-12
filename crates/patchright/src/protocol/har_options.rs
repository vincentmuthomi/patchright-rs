// HAR recording options for Tracing::start_har.
//
// Kept in its own module (rather than in tracing.rs) so the pure
// RecordHarOptions serialization is covered by mutation testing, while the
// integration-only start_har/stop_har stay out of scope.

use serde::Serialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::protocol::RecordHar;

/// How resource bodies are stored in a recorded HAR.
///
/// See: <https://playwright.dev/docs/api/class-tracing#tracing-start-har-option-content>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum HarContent {
    /// Do not store bodies (smallest HAR).
    Omit,
    /// Inline bodies into the HAR as base64 (the default for a non-`.zip` path).
    Embed,
    /// Store bodies as separate files / zip entries (the default for a `.zip` path).
    Attach,
}

/// Level of detail recorded in a HAR.
///
/// See: <https://playwright.dev/docs/api/class-tracing#tracing-start-har-option-mode>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum HarMode {
    /// Record everything (default).
    Full,
    /// Record only essentials (size, timing) and omit headers/bodies/cookies.
    Minimal,
}

/// Options for [`Tracing::start_har`](crate::protocol::Tracing::start_har).
///
/// # Example
///
/// ```
/// use patchright_rs::{StartHarOptions, HarContent, HarMode};
///
/// let opts = StartHarOptions::default()
///     .content(HarContent::Attach)
///     .mode(HarMode::Minimal)
///     .url_filter("**/api/**");
/// ```
///
/// See: <https://playwright.dev/docs/api/class-tracing#tracing-start-har>
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct StartHarOptions {
    /// How resource bodies are stored. Defaults to `Attach` for a `.zip` path,
    /// `Embed` otherwise.
    pub content: Option<HarContent>,
    /// Level of detail. Defaults to [`HarMode::Full`].
    pub mode: Option<HarMode>,
    /// Glob pattern; only requests whose URL matches are recorded.
    pub url_filter: Option<String>,
    /// Directory to store `attach`-mode resource files in (for non-zip paths).
    pub resources_dir: Option<String>,
}

impl StartHarOptions {
    /// How resource bodies are stored (`Attach` / `Embed` / `Omit`).
    pub fn content(mut self, content: HarContent) -> Self {
        self.content = Some(content);
        self
    }
    /// Level of detail (`Full` / `Minimal`).
    pub fn mode(mut self, mode: HarMode) -> Self {
        self.mode = Some(mode);
        self
    }
    /// Glob pattern; only requests whose URL matches are recorded.
    pub fn url_filter(mut self, url_filter: impl Into<String>) -> Self {
        self.url_filter = Some(url_filter.into());
        self
    }
    /// Directory to store `attach`-mode resource files in (for non-zip paths).
    pub fn resources_dir(mut self, resources_dir: impl Into<String>) -> Self {
        self.resources_dir = Some(resources_dir.into());
        self
    }

    /// Build the protocol `RecordHarOptions` object for the given output path.
    ///
    /// `harPath` is intentionally omitted: setting it makes the driver write its
    /// own archive at that path (appending `.zip`), which would duplicate the
    /// file we already produce via `harExport` + unzip in `stop_har`. The path
    /// here only selects the default `content` mode.
    pub(crate) fn to_record_har_json(&self, path: &str) -> Value {
        let is_zip = path.ends_with(".zip");
        let content = self.content.unwrap_or(if is_zip {
            HarContent::Attach
        } else {
            HarContent::Embed
        });
        let mode = self.mode.unwrap_or(HarMode::Full);

        let mut o = serde_json::json!({});
        o["content"] = serde_json::to_value(content).expect("serialize HarContent cannot fail");
        o["mode"] = serde_json::to_value(mode).expect("serialize HarMode cannot fail");
        if let Some(glob) = &self.url_filter {
            o["urlGlob"] = serde_json::json!(glob);
        }
        if let Some(dir) = &self.resources_dir {
            o["resourcesDir"] = serde_json::json!(dir);
        }
        o
    }
}

impl StartHarOptions {
    /// The recording a context's `record_har` option stands for: the same
    /// `harStart` as [`Tracing::start_har`](crate::protocol::Tracing::start_har),
    /// with the same content and mode defaults. `omit_content` means
    /// [`HarContent::Omit`] unless `content` names a policy explicitly.
    pub(crate) fn from_record_har(record: &RecordHar) -> Result<Self> {
        let content = match record.content.as_deref() {
            Some(name) => Some(parse_content(name)?),
            None if record.omit_content == Some(true) => Some(HarContent::Omit),
            None => None,
        };
        let mode = record.mode.as_deref().map(parse_mode).transpose()?;
        Ok(Self {
            content,
            mode,
            url_filter: record.url_filter.clone(),
            resources_dir: None,
        })
    }
}

/// A context's `record_har` option, checked before the context is created so
/// a bad content or mode name fails without launching anything.
pub(crate) struct OptionHar {
    pub(crate) path: String,
    pub(crate) options: StartHarOptions,
}

impl OptionHar {
    pub(crate) fn new(record: RecordHar) -> Result<Self> {
        let options = StartHarOptions::from_record_har(&record)?;
        Ok(Self {
            path: record.path,
            options,
        })
    }
}

fn parse_content(name: &str) -> Result<HarContent> {
    match name {
        "omit" => Ok(HarContent::Omit),
        "embed" => Ok(HarContent::Embed),
        "attach" => Ok(HarContent::Attach),
        other => Err(Error::InvalidArgument(format!(
            "record_har content must be \"omit\", \"embed\" or \"attach\", not {other:?}"
        ))),
    }
}

fn parse_mode(name: &str) -> Result<HarMode> {
    match name {
        "full" => Ok(HarMode::Full),
        "minimal" => Ok(HarMode::Minimal),
        other => Err(Error::InvalidArgument(format!(
            "record_har mode must be \"full\" or \"minimal\", not {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_har_defaults_leave_content_and_mode_to_start_har() {
        let options = StartHarOptions::from_record_har(&RecordHar::new("run.har")).unwrap();
        assert_eq!(options.content, None);
        assert_eq!(options.mode, None);
        assert_eq!(options.url_filter, None);
    }

    #[test]
    fn record_har_omit_content_means_omit_unless_content_is_named() {
        let omit = RecordHar::new("run.har").omit_content(true);
        assert_eq!(
            StartHarOptions::from_record_har(&omit).unwrap().content,
            Some(HarContent::Omit)
        );
        let named = RecordHar::new("run.har")
            .omit_content(true)
            .content("attach");
        assert_eq!(
            StartHarOptions::from_record_har(&named).unwrap().content,
            Some(HarContent::Attach)
        );
        let kept = RecordHar::new("run.har").omit_content(false);
        assert_eq!(
            StartHarOptions::from_record_har(&kept).unwrap().content,
            None
        );
    }

    #[test]
    fn record_har_names_map_to_their_variants() {
        for (name, content) in [
            ("omit", HarContent::Omit),
            ("embed", HarContent::Embed),
            ("attach", HarContent::Attach),
        ] {
            let record = RecordHar::new("run.har").content(name);
            assert_eq!(
                StartHarOptions::from_record_har(&record).unwrap().content,
                Some(content)
            );
        }
        for (name, mode) in [("full", HarMode::Full), ("minimal", HarMode::Minimal)] {
            let record = RecordHar::new("run.har").mode(name);
            assert_eq!(
                StartHarOptions::from_record_har(&record).unwrap().mode,
                Some(mode)
            );
        }
        let filtered = RecordHar::new("run.har").url_filter("**/api/**");
        assert_eq!(
            StartHarOptions::from_record_har(&filtered)
                .unwrap()
                .url_filter
                .as_deref(),
            Some("**/api/**")
        );
    }

    #[test]
    fn record_har_rejects_an_unknown_content_or_mode() {
        let content = RecordHar::new("run.har").content("zip");
        assert!(matches!(
            StartHarOptions::from_record_har(&content),
            Err(Error::InvalidArgument(msg)) if msg.contains("\"zip\"")
        ));
        let mode = RecordHar::new("run.har").mode("partial");
        assert!(matches!(
            StartHarOptions::from_record_har(&mode),
            Err(Error::InvalidArgument(msg)) if msg.contains("\"partial\"")
        ));
    }

    #[test]
    fn test_start_har_options_zip_defaults_to_attach() {
        let json = StartHarOptions::default().to_record_har_json("run.har.zip");
        assert_eq!(json["content"], "attach");
        assert_eq!(json["mode"], "full");
        // harPath is deliberately not sent (avoids the driver double-writing).
        assert!(json.get("harPath").is_none());
    }

    #[test]
    fn test_start_har_options_plain_defaults_to_embed() {
        let json = StartHarOptions::default().to_record_har_json("run.har");
        assert_eq!(json["content"], "embed");
    }

    #[test]
    fn test_start_har_options_setters() {
        let opts = StartHarOptions::default()
            .content(HarContent::Omit)
            .mode(HarMode::Minimal)
            .url_filter("**/api/**");
        let json = opts.to_record_har_json("run.har");
        assert_eq!(json["content"], "omit");
        assert_eq!(json["mode"], "minimal");
        assert_eq!(json["urlGlob"], "**/api/**");
        assert!(json.get("resourcesDir").is_none());
    }

    #[test]
    fn test_start_har_options_resources_dir_setter() {
        let json = StartHarOptions::default()
            .resources_dir("/tmp/har-resources")
            .to_record_har_json("run.har");
        assert_eq!(json["resourcesDir"], "/tmp/har-resources");
    }
}
