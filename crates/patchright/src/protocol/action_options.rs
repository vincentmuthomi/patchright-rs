// Action options for various Locator methods
//
// Provides configuration for fill, press, check, hover, and select actions.

use super::click::{KeyboardModifier, Position};

/// Fill options
///
/// Configuration options for fill() action.
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-fill>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct FillOptions {
    /// Whether to bypass actionability checks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Maximum time in milliseconds
    #[serde(serialize_with = "crate::protocol::serialize_timeout_or_default")]
    pub timeout: Option<f64>,
}

impl FillOptions {
    /// Create a new builder for FillOptions
    pub fn builder() -> FillOptionsBuilder {
        FillOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("FillOptions serialization cannot fail")
    }
}

/// Builder for FillOptions
#[derive(Debug, Clone, Default)]
pub struct FillOptionsBuilder {
    force: Option<bool>,
    timeout: Option<f64>,
}

impl FillOptionsBuilder {
    /// Bypass actionability checks
    pub fn force(mut self, force: bool) -> Self {
        self.force = Some(force);
        self
    }

    /// Set timeout in milliseconds
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Build the FillOptions
    pub fn build(self) -> FillOptions {
        FillOptions {
            force: self.force,
            timeout: self.timeout,
        }
    }
}

/// Press options
///
/// Configuration options for press() action.
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-press>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct PressOptions {
    /// Time to wait between keydown and keyup in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    /// Maximum time in milliseconds
    #[serde(serialize_with = "crate::protocol::serialize_timeout_or_default")]
    pub timeout: Option<f64>,
}

impl PressOptions {
    /// Create a new builder for PressOptions
    pub fn builder() -> PressOptionsBuilder {
        PressOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("PressOptions serialization cannot fail")
    }
}

/// Builder for PressOptions
#[derive(Debug, Clone, Default)]
pub struct PressOptionsBuilder {
    delay: Option<f64>,
    timeout: Option<f64>,
}

impl PressOptionsBuilder {
    /// Set delay between keydown and keyup in milliseconds
    pub fn delay(mut self, delay: f64) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Set timeout in milliseconds
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Build the PressOptions
    pub fn build(self) -> PressOptions {
        PressOptions {
            delay: self.delay,
            timeout: self.timeout,
        }
    }
}

/// Check options
///
/// Configuration options for check() and uncheck() actions.
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-check>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CheckOptions {
    /// Whether to bypass actionability checks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Position to click relative to element top-left corner
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    /// Maximum time in milliseconds
    #[serde(serialize_with = "crate::protocol::serialize_timeout_or_default")]
    pub timeout: Option<f64>,
    /// Perform actionability checks without checking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<bool>,
    /// Whether the action may scroll the element into view first
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll: Option<Scroll>,
}

impl CheckOptions {
    /// Create a new builder for CheckOptions
    pub fn builder() -> CheckOptionsBuilder {
        CheckOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("CheckOptions serialization cannot fail")
    }
}

/// Builder for CheckOptions
#[derive(Debug, Clone, Default)]
pub struct CheckOptionsBuilder {
    force: Option<bool>,
    position: Option<Position>,
    timeout: Option<f64>,
    trial: Option<bool>,
    scroll: Option<Scroll>,
}

impl CheckOptionsBuilder {
    /// Bypass actionability checks
    pub fn force(mut self, force: bool) -> Self {
        self.force = Some(force);
        self
    }

    /// Set position to click relative to element top-left corner
    pub fn position(mut self, position: Position) -> Self {
        self.position = Some(position);
        self
    }

    /// Set timeout in milliseconds
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Perform actionability checks without checking
    pub fn trial(mut self, trial: bool) -> Self {
        self.trial = Some(trial);
        self
    }

    /// Opt out of scrolling the element into view (`Scroll::None`), or keep
    /// Playwright's default (`Scroll::Auto`)
    pub fn scroll(mut self, scroll: Scroll) -> Self {
        self.scroll = Some(scroll);
        self
    }

    /// Build the CheckOptions
    pub fn build(self) -> CheckOptions {
        CheckOptions {
            force: self.force,
            position: self.position,
            timeout: self.timeout,
            trial: self.trial,
            scroll: self.scroll,
        }
    }
}

/// Hover options
///
/// Configuration options for hover() action.
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-hover>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct HoverOptions {
    /// Whether to bypass actionability checks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Modifier keys to press during hover
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<Vec<KeyboardModifier>>,
    /// Position to hover relative to element top-left corner
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    /// Maximum time in milliseconds
    #[serde(serialize_with = "crate::protocol::serialize_timeout_or_default")]
    pub timeout: Option<f64>,
    /// Perform actionability checks without hovering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<bool>,
    /// Whether the action may scroll the element into view first
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll: Option<Scroll>,
}

impl HoverOptions {
    /// Create a new builder for HoverOptions
    pub fn builder() -> HoverOptionsBuilder {
        HoverOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("HoverOptions serialization cannot fail")
    }
}

/// Builder for HoverOptions
#[derive(Debug, Clone, Default)]
pub struct HoverOptionsBuilder {
    force: Option<bool>,
    modifiers: Option<Vec<KeyboardModifier>>,
    position: Option<Position>,
    timeout: Option<f64>,
    trial: Option<bool>,
    scroll: Option<Scroll>,
}

impl HoverOptionsBuilder {
    /// Bypass actionability checks
    pub fn force(mut self, force: bool) -> Self {
        self.force = Some(force);
        self
    }

    /// Set modifier keys to press during hover
    pub fn modifiers(mut self, modifiers: Vec<KeyboardModifier>) -> Self {
        self.modifiers = Some(modifiers);
        self
    }

    /// Set position to hover relative to element top-left corner
    pub fn position(mut self, position: Position) -> Self {
        self.position = Some(position);
        self
    }

    /// Set timeout in milliseconds
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Perform actionability checks without hovering
    pub fn trial(mut self, trial: bool) -> Self {
        self.trial = Some(trial);
        self
    }

    /// Opt out of scrolling the element into view (`Scroll::None`), or keep
    /// Playwright's default (`Scroll::Auto`)
    pub fn scroll(mut self, scroll: Scroll) -> Self {
        self.scroll = Some(scroll);
        self
    }

    /// Build the HoverOptions
    pub fn build(self) -> HoverOptions {
        HoverOptions {
            force: self.force,
            modifiers: self.modifiers,
            position: self.position,
            timeout: self.timeout,
            trial: self.trial,
            scroll: self.scroll,
        }
    }
}

/// Options for [`Locator::press_sequentially()`](crate::protocol::Locator::press_sequentially).
///
/// Controls timing between key presses when typing characters one by one.
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-press-sequentially>
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PressSequentiallyOptions {
    /// Delay between key presses in milliseconds. Defaults to 0.
    pub delay: Option<f64>,
}

impl PressSequentiallyOptions {
    /// Create a new builder for PressSequentiallyOptions
    pub fn builder() -> PressSequentiallyOptionsBuilder {
        PressSequentiallyOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({});

        if let Some(delay) = self.delay {
            json["delay"] = serde_json::json!(delay);
        }

        // Timeout is required in Playwright 1.56.1+
        json["timeout"] = serde_json::json!(crate::DEFAULT_TIMEOUT_MS);

        json
    }
}

/// Builder for PressSequentiallyOptions
#[derive(Debug, Clone, Default)]
pub struct PressSequentiallyOptionsBuilder {
    delay: Option<f64>,
}

impl PressSequentiallyOptionsBuilder {
    /// Set delay between key presses in milliseconds
    pub fn delay(mut self, delay: f64) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Build the PressSequentiallyOptions
    pub fn build(self) -> PressSequentiallyOptions {
        PressSequentiallyOptions { delay: self.delay }
    }
}

/// Select options
///
/// Configuration options for select_option() action.
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-select-option>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SelectOptions {
    /// Whether to bypass actionability checks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Maximum time in milliseconds
    #[serde(serialize_with = "crate::protocol::serialize_timeout_or_default")]
    pub timeout: Option<f64>,
}

impl SelectOptions {
    /// Create a new builder for SelectOptions
    pub fn builder() -> SelectOptionsBuilder {
        SelectOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("SelectOptions serialization cannot fail")
    }
}

/// Builder for SelectOptions
#[derive(Debug, Clone, Default)]
pub struct SelectOptionsBuilder {
    force: Option<bool>,
    timeout: Option<f64>,
}

impl SelectOptionsBuilder {
    /// Bypass actionability checks
    pub fn force(mut self, force: bool) -> Self {
        self.force = Some(force);
        self
    }

    /// Set timeout in milliseconds
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Build the SelectOptions
    pub fn build(self) -> SelectOptions {
        SelectOptions {
            force: self.force,
            timeout: self.timeout,
        }
    }
}

/// Keyboard options
///
/// Configuration options for keyboard.press() and keyboard.type_text() methods.
///
/// See: <https://playwright.dev/docs/api/class-keyboard#keyboard-press>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct KeyboardOptions {
    /// Time to wait between key presses in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
}

impl KeyboardOptions {
    /// Create a new builder for KeyboardOptions
    pub fn builder() -> KeyboardOptionsBuilder {
        KeyboardOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("KeyboardOptions serialization cannot fail")
    }
}

/// Builder for KeyboardOptions
#[derive(Debug, Clone, Default)]
pub struct KeyboardOptionsBuilder {
    delay: Option<f64>,
}

impl KeyboardOptionsBuilder {
    /// Set delay between key presses in milliseconds
    pub fn delay(mut self, delay: f64) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Build the KeyboardOptions
    pub fn build(self) -> KeyboardOptions {
        KeyboardOptions { delay: self.delay }
    }
}

/// Mouse options
///
/// Configuration options for mouse methods.
///
/// See: <https://playwright.dev/docs/api/class-mouse>
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct MouseOptions {
    /// Mouse button to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<super::click::MouseButton>,
    /// Number of clicks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub click_count: Option<u32>,
    /// Time to wait between mousedown and mouseup in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    /// Number of intermediate mousemove events (for move operations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
}

impl MouseOptions {
    /// Create a new builder for MouseOptions
    pub fn builder() -> MouseOptionsBuilder {
        MouseOptionsBuilder::default()
    }

    /// Convert options to JSON value for protocol
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("MouseOptions serialization cannot fail")
    }
}

/// Builder for MouseOptions
#[derive(Debug, Clone, Default)]
pub struct MouseOptionsBuilder {
    button: Option<super::click::MouseButton>,
    click_count: Option<u32>,
    delay: Option<f64>,
    steps: Option<u32>,
}

impl MouseOptionsBuilder {
    /// Set the mouse button
    pub fn button(mut self, button: super::click::MouseButton) -> Self {
        self.button = Some(button);
        self
    }

    /// Set the number of clicks
    pub fn click_count(mut self, click_count: u32) -> Self {
        self.click_count = Some(click_count);
        self
    }

    /// Set delay between mousedown and mouseup in milliseconds
    pub fn delay(mut self, delay: f64) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Set number of intermediate mousemove events
    pub fn steps(mut self, steps: u32) -> Self {
        self.steps = Some(steps);
        self
    }

    /// Build the MouseOptions
    pub fn build(self) -> MouseOptions {
        MouseOptions {
            button: self.button,
            click_count: self.click_count,
            delay: self.delay,
            steps: self.steps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::click::MouseButton;

    #[test]
    fn test_fill_options_builder() {
        let options = FillOptions::builder().force(true).timeout(5000.0).build();

        let json = options.to_json();
        assert_eq!(json["force"], true);
        assert_eq!(json["timeout"], 5000.0);
    }

    #[test]
    fn test_press_options_builder() {
        let options = PressOptions::builder().delay(100.0).timeout(3000.0).build();

        let json = options.to_json();
        assert_eq!(json["delay"], 100.0);
        assert_eq!(json["timeout"], 3000.0);
    }

    #[test]
    fn test_check_options_builder() {
        let options = CheckOptions::builder()
            .force(true)
            .position(Position { x: 5.0, y: 10.0 })
            .timeout(2000.0)
            .trial(true)
            .build();

        let json = options.to_json();
        assert_eq!(json["force"], true);
        assert_eq!(json["position"]["x"], 5.0);
        assert_eq!(json["position"]["y"], 10.0);
        assert_eq!(json["timeout"], 2000.0);
        assert_eq!(json["trial"], true);
    }

    #[test]
    fn test_hover_options_builder() {
        let options = HoverOptions::builder()
            .force(true)
            .modifiers(vec![KeyboardModifier::Shift])
            .position(Position { x: 10.0, y: 20.0 })
            .timeout(4000.0)
            .trial(false)
            .build();

        let json = options.to_json();
        assert_eq!(json["force"], true);
        assert_eq!(json["modifiers"], serde_json::json!(["Shift"]));
        assert_eq!(json["position"]["x"], 10.0);
        assert_eq!(json["position"]["y"], 20.0);
        assert_eq!(json["timeout"], 4000.0);
        assert_eq!(json["trial"], false);
    }

    #[test]
    fn test_select_options_builder() {
        let options = SelectOptions::builder().force(true).timeout(6000.0).build();

        let json = options.to_json();
        assert_eq!(json["force"], true);
        assert_eq!(json["timeout"], 6000.0);
    }

    #[test]
    fn test_keyboard_options_builder() {
        let options = KeyboardOptions::builder().delay(50.0).build();

        let json = options.to_json();
        assert_eq!(json["delay"], 50.0);
    }

    #[test]
    fn test_mouse_options_builder() {
        let options = MouseOptions::builder()
            .button(MouseButton::Right)
            .click_count(2)
            .delay(100.0)
            .steps(10)
            .build();

        let json = options.to_json();
        assert_eq!(json["button"], "right");
        assert_eq!(json["clickCount"], 2);
        assert_eq!(json["delay"], 100.0);
        assert_eq!(json["steps"], 10);
    }
}

/// Whether an action may scroll the element into view first.
///
/// Playwright's actionability checks scroll the target into the viewport
/// before acting. `None` opts out, so the action fails instead of scrolling
/// when the element is out of view. Useful for asserting that something is
/// already visible, and for pages where scrolling itself changes layout.
///
/// See: <https://playwright.dev/docs/api/class-locator#locator-click-option-scroll>
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Scroll {
    /// Scroll the element into view if needed (Playwright's default).
    Auto,
    /// Never scroll; act only if the element is already in view.
    None,
}

#[cfg(test)]
mod scroll_tests {
    use super::{CheckOptions, HoverOptions, Scroll};

    #[test]
    fn scroll_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Scroll::Auto).unwrap(), "\"auto\"");
        assert_eq!(serde_json::to_string(&Scroll::None).unwrap(), "\"none\"");
    }

    #[test]
    fn scroll_is_omitted_unless_set() {
        // The driver defaults to "auto"; sending nothing keeps that, so an
        // unset builder must not put the key on the wire.
        let json = CheckOptions::builder().build().to_json();
        assert!(json.get("scroll").is_none());

        let json = HoverOptions::builder()
            .scroll(Scroll::None)
            .build()
            .to_json();
        assert_eq!(json["scroll"], "none");
    }
}
