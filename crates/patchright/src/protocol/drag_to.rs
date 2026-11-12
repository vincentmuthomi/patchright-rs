// DragToOptions and related types
//
// Provides configuration for drag_to actions, matching Playwright's API.

use crate::protocol::action_options::Scroll;
use crate::protocol::click::Position;

/// Options for [`Locator::drag_to()`](crate::protocol::Locator::drag_to).
///
/// Configuration for dragging a source element onto a target element.
///
/// Use the builder pattern to construct options:
///
/// # Example
///
/// ```no_run
/// use patchright_rs::{DragToOptions, Position};
///
/// // Drag with custom source and target positions
/// let options = DragToOptions::builder()
///     .source_position(Position { x: 10.0, y: 10.0 })
///     .target_position(Position { x: 60.0, y: 60.0 })
///     .build();
///
/// // Force drag (bypass actionability checks)
/// let options = DragToOptions::builder()
///     .force(true)
///     .build();
///
/// // Trial run (actionability checks only, don't actually drag)
/// let options = DragToOptions::builder()
///     .trial(true)
///     .build();
/// ```
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-drag-to>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct DragToOptions {
    /// Whether to bypass actionability checks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Don't wait for navigation after the action
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_wait_after: Option<bool>,
    /// Maximum time in milliseconds
    #[serde(serialize_with = "crate::protocol::serialize_timeout_or_default")]
    pub timeout: Option<f64>,
    /// Perform actionability checks without dragging
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<bool>,
    /// Where to click on the source element (relative to top-left corner)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_position: Option<Position>,
    /// Where to drop on the target element (relative to top-left corner)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_position: Option<Position>,
    /// Whether the action may scroll the element into view first
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll: Option<Scroll>,
}

impl DragToOptions {
    /// Create a new builder for DragToOptions
    pub fn builder() -> DragToOptionsBuilder {
        DragToOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("DragToOptions serialization cannot fail")
    }
}

/// Builder for DragToOptions
///
/// Provides a fluent API for constructing drag_to options.
#[derive(Debug, Clone, Default)]
pub struct DragToOptionsBuilder {
    force: Option<bool>,
    no_wait_after: Option<bool>,
    timeout: Option<f64>,
    trial: Option<bool>,
    source_position: Option<Position>,
    target_position: Option<Position>,
    scroll: Option<Scroll>,
}

impl DragToOptionsBuilder {
    /// Bypass actionability checks
    pub fn force(mut self, force: bool) -> Self {
        self.force = Some(force);
        self
    }

    /// Don't wait for navigation after the action
    pub fn no_wait_after(mut self, no_wait_after: bool) -> Self {
        self.no_wait_after = Some(no_wait_after);
        self
    }

    /// Set timeout in milliseconds
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Perform actionability checks without dragging
    pub fn trial(mut self, trial: bool) -> Self {
        self.trial = Some(trial);
        self
    }

    /// Set where to click on the source element (relative to top-left corner)
    pub fn source_position(mut self, source_position: Position) -> Self {
        self.source_position = Some(source_position);
        self
    }

    /// Set where to drop on the target element (relative to top-left corner)
    pub fn target_position(mut self, target_position: Position) -> Self {
        self.target_position = Some(target_position);
        self
    }

    /// Opt out of scrolling the element into view (`Scroll::None`), or keep
    /// Playwright's default (`Scroll::Auto`)
    pub fn scroll(mut self, scroll: Scroll) -> Self {
        self.scroll = Some(scroll);
        self
    }

    /// Build the DragToOptions
    pub fn build(self) -> DragToOptions {
        DragToOptions {
            force: self.force,
            no_wait_after: self.no_wait_after,
            timeout: self.timeout,
            trial: self.trial,
            source_position: self.source_position,
            target_position: self.target_position,
            scroll: self.scroll,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drag_to_options_default() {
        let options = DragToOptions::builder().build();
        let json = options.to_json();
        // timeout has a default value
        assert!(json["timeout"].is_number());
        // other optional fields are absent
        assert!(json.get("force").is_none());
        assert!(json.get("trial").is_none());
        assert!(json.get("sourcePosition").is_none());
        assert!(json.get("targetPosition").is_none());
    }

    #[test]
    fn test_drag_to_options_force() {
        let options = DragToOptions::builder().force(true).build();
        let json = options.to_json();
        assert_eq!(json["force"], true);
    }

    #[test]
    fn test_drag_to_options_timeout() {
        let options = DragToOptions::builder().timeout(5000.0).build();
        let json = options.to_json();
        assert_eq!(json["timeout"], 5000.0);
    }

    #[test]
    fn test_drag_to_options_trial() {
        let options = DragToOptions::builder().trial(true).build();
        let json = options.to_json();
        assert_eq!(json["trial"], true);
    }

    #[test]
    fn test_drag_to_options_positions() {
        let options = DragToOptions::builder()
            .source_position(Position { x: 5.0, y: 10.0 })
            .target_position(Position { x: 50.0, y: 60.0 })
            .build();
        let json = options.to_json();
        assert_eq!(json["sourcePosition"]["x"], 5.0);
        assert_eq!(json["sourcePosition"]["y"], 10.0);
        assert_eq!(json["targetPosition"]["x"], 50.0);
        assert_eq!(json["targetPosition"]["y"], 60.0);
    }

    #[test]
    fn test_drag_to_options_no_wait_after() {
        let options = DragToOptions::builder().no_wait_after(true).build();
        let json = options.to_json();
        assert_eq!(json["noWaitAfter"], true);
    }
}
