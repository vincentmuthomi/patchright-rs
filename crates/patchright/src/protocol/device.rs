// Copyright 2026 Paul Adamson
// Licensed under the Apache License, Version 2.0
//
// Device descriptor types for browser emulation.
//
// See: <https://playwright.dev/docs/emulation>

use serde::Deserialize;

/// Viewport dimensions for a device descriptor.
#[derive(Clone, Debug, Deserialize)]
pub struct DeviceViewport {
    /// The viewport width in CSS pixels.
    pub width: u32,
    /// The viewport height in CSS pixels.
    pub height: u32,
}

/// Describes a device for browser emulation.
///
/// Use with `BrowserContext::new_context()` options to emulate a specific device,
/// matching the behavior of `playwright.devices["iPhone 13"]` in Python/JS.
///
/// Device descriptors are accessed by name from [`crate::protocol::Playwright::devices`].
/// The name is the map key, not a field of the descriptor itself.
///
/// # Example
///
/// ```no_run
/// use patchright_rs::protocol::Playwright;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let playwright = Playwright::launch().await?;
///
///     let iphone = &playwright.devices()["iPhone 13"];
///     assert!(iphone.is_mobile);
///     assert!(iphone.has_touch);
///     assert_eq!(iphone.default_browser_type, "webkit");
///
///     playwright.shutdown().await?;
///     Ok(())
/// }
/// ```
///
/// See: <https://playwright.dev/docs/api/class-playwright#playwright-devices>
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct DeviceDescriptor {
    /// The user-agent string for the device.
    pub user_agent: String,
    /// The viewport dimensions.
    pub viewport: DeviceViewport,
    /// The screen size the device reports through `window.screen`, larger
    /// than the viewport on a device whose browser chrome takes room. Pass
    /// it as [`BrowserContextOptions::screen`](crate::protocol::BrowserContextOptions::screen)
    /// to emulate the device fully. Not every descriptor has one.
    pub screen: Option<DeviceViewport>,
    /// The device pixel ratio (e.g., `3.0` for Retina displays).
    pub device_scale_factor: f64,
    /// Whether the device is a mobile device.
    pub is_mobile: bool,
    /// Whether the device supports touch input.
    pub has_touch: bool,
    /// The default browser type for the device: `"chromium"`, `"firefox"`, or `"webkit"`.
    pub default_browser_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(screen: Option<serde_json::Value>) -> DeviceDescriptor {
        let mut json = serde_json::json!({
            "userAgent": "ua",
            "viewport": {"width": 390, "height": 664},
            "deviceScaleFactor": 3,
            "isMobile": true,
            "hasTouch": true,
            "defaultBrowserType": "webkit"
        });
        if let Some(screen) = screen {
            json["screen"] = screen;
        }
        serde_json::from_value(json).expect("descriptor deserializes")
    }

    #[test]
    fn a_descriptor_carries_its_screen_when_it_has_one() {
        let with = descriptor(Some(serde_json::json!({"width": 390, "height": 844})));
        let screen = with.screen.expect("screen is read");
        assert_eq!((screen.width, screen.height), (390, 844));
        assert!(descriptor(None).screen.is_none());
    }
}
