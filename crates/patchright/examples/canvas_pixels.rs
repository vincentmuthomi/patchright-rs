// Canvas pixel assertions - verify what actually got painted
//
// Shows: asserting on rendered canvas pixels via typed evaluate(), the
// recipe for testing canvas/WebGL/wasm frontends where DOM state alone
// can't prove anything was drawn ("state set but nothing repainted").
//
// Usage: cargo run --example canvas_pixels

use patchright_rs::Playwright;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Pixel {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let playwright = Playwright::launch().await?;
    let browser = playwright.chromium().launch().await?;
    let page = browser.new_page().await?;

    page.set_content(
        r#"<canvas id="stage" width="200" height="100"></canvas>"#,
        None,
    )
    .await?;

    // Paint something, the way an app's render loop would.
    page.evaluate_expression(
        "const ctx = document.getElementById('stage').getContext('2d');
         ctx.fillStyle = 'rgb(200, 40, 0)';
         ctx.fillRect(10, 10, 50, 50);",
    )
    .await?;

    // Probe pixels with getImageData, deserializing straight into a struct.
    // Waiting a frame (requestAnimationFrame) before reading guards against
    // asserting mid-render in real apps.
    let probe = "([x, y]) => new Promise(resolve => {
        requestAnimationFrame(() => {
            const ctx = document.getElementById('stage').getContext('2d');
            const d = ctx.getImageData(x, y, 1, 1).data;
            resolve({ r: d[0], g: d[1], b: d[2], a: d[3] });
        });
    })";

    let painted: Pixel = page.evaluate(probe, Some(&[30, 30])).await?;
    assert_eq!(
        painted,
        Pixel {
            r: 200,
            g: 40,
            b: 0,
            a: 255
        }
    );
    println!("Painted pixel at (30, 30): {painted:?}");

    let blank: Pixel = page.evaluate(probe, Some(&[150, 80])).await?;
    assert_eq!(blank.a, 0, "untouched canvas should be transparent");
    println!("Untouched pixel at (150, 80): {blank:?}");

    browser.close().await?;
    Ok(())
}
