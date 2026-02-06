//! Dual-model routing example using OpenRouter.
//!
//! Demonstrates vision + text model routing: a cheap text model handles most
//! rounds while a vision model kicks in for round 0 and stagnation recovery.
//! This saves cost by only sending screenshots when they're actually needed.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example remote_multimodal_dual --features "spider/sync spider/chrome spider/agent_chrome"
//! ```

// EXAMPLE output
// === Page Received ===
// URL: https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html
//
// === AI Extraction Results ===
// Result 1:
//   Content output: {"book":{"title":"A Light in the Attic","price":"£51.77",...}}
//   Usage: AutomationUsage { prompt_tokens: 1200, completion_tokens: 250, ... }
//
// === Completed ===

extern crate spider;

use spider::features::automation::{
    ModelEndpoint, RemoteMultimodalConfigs, VisionRouteMode,
};
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    env_logger::init();

    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    let url = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

    // ── Configure dual-model routing ──────────────────────────────────
    //
    // Primary model = vision-capable (used as fallback and for vision rounds)
    // Text model    = cheaper, text-only (used for most extraction rounds)
    //
    // VisionRouteMode::TextFirst means:
    //   - Round 0: vision (screenshot + HTML)
    //   - Subsequent rounds: text-only (HTML only, no screenshot)
    //   - Stagnation or stuck ≥ 3: upgrade back to vision
    //
    // This saves ~35k tokens per text-only round by skipping screenshots.

    let mm_config = RemoteMultimodalConfigs::new(
        "https://openrouter.ai/api/v1/chat/completions",
        "qwen/qwen-2.5-vl-72b-instruct", // primary (vision-capable)
    )
    .with_api_key(&api_key)
    // Set up the dual-model routing
    .with_dual_models(
        // Vision model — used for round 0 and stagnation recovery
        ModelEndpoint::new("qwen/qwen-2.5-vl-72b-instruct"),
        // Text model — used for all other rounds (cheaper, no screenshot needed)
        ModelEndpoint::new("qwen/qwen-2.5-72b-instruct"),
    )
    .with_vision_route_mode(VisionRouteMode::TextFirst);

    // ── Configure extraction settings ─────────────────────────────────
    let mut mm_config = mm_config;
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 1; // Single extraction pass
    mm_config.cfg.request_json_object = true;
    mm_config.cfg.extraction_prompt = Some(
        "Extract book details: title, price, availability, UPC, description.".to_string(),
    );

    // ── Run ───────────────────────────────────────────────────────────
    let mut website: Website = Website::new(url)
        .with_limit(1)
        .with_remote_multimodal(Some(mm_config))
        .build()
        .unwrap();

    let mut rx = website.subscribe(16).unwrap();

    let join_handle = tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            println!("=== Page Received ===");
            println!("URL: {}", page.get_url());

            if let Some(ref ai_data) = page.extra_remote_multimodal_data {
                println!("\n=== AI Extraction Results ===");
                for (i, result) in ai_data.iter().enumerate() {
                    println!("Result {}:", i + 1);
                    println!("  Content: {}", result.content_output);
                    if let Some(ref err) = result.error {
                        println!("  Error: {}", err);
                    }
                    if let Some(ref usage) = result.usage {
                        println!("  Usage: {:?}", usage);
                    }
                }
            } else {
                println!("\nNo AI extraction data available.");
            }
        }
    });

    let start = tokio::time::Instant::now();
    website.crawl().await;
    website.unsubscribe();
    let _ = join_handle.await;

    println!("\n=== Completed in {:?} ===", start.elapsed());
}
