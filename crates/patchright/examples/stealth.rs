//! Patchright's recommended undetected setup: real Chrome, headed, a
//! persistent profile, no viewport override, and no custom user agent or
//! headers.
//!
//! ```sh
//! cargo run --example stealth            # needs Google Chrome installed
//! cargo run --example stealth -- chromium  # or the bundled Chromium
//! ```

use patchright_rs::IsolatedContextExt;
use patchright_rs::protocol::{BrowserContextOptions, Playwright};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let use_bundled_chromium = std::env::args().nth(1).as_deref() == Some("chromium");

    let playwright = Playwright::launch().await?;
    let profile = std::env::temp_dir().join("patchright-rs-profile");

    let mut options = BrowserContextOptions::builder()
        .headless(false)
        .no_viewport(true);
    if !use_bundled_chromium {
        options = options.channel("chrome".to_string());
    }
    let context = playwright
        .chromium()
        .launch_persistent_context_with_options(profile.to_string_lossy(), options.build())
        .await?;

    let page = context.new_page().await?;
    page.goto("https://bot.sannysoft.com/", None).await?;

    // Evaluations run in an isolated world by default, invisible to the
    // page's scripts. Reading a page global needs the main world.
    let webdriver: String = page
        .evaluate_value("String(navigator.webdriver)")
        .main_world()
        .await?;
    println!("navigator.webdriver = {webdriver}");

    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    context.close().await?;
    Ok(())
}
