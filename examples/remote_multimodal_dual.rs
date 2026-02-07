//! Dual-model routing example with separate endpoints.
//!
//! Demonstrates vision + text model routing where each model can have its own
//! API URL and API key. A cheap text model handles most rounds while a
//! vision model kicks in for round 0 and stagnation recovery.
//!
//! This example shows two setups:
//! 1. Same provider (both on OpenRouter, different models)
//! 2. Cross-provider (vision on OpenAI, text on OpenRouter)
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example remote_multimodal_dual --features "spider/sync spider/chrome spider/agent_chrome"
//! ```
//!
//! EXAMPLE output
//! === Page Received ===
//! URL: https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html
//!
//! === AI Extraction Results ===
//! Result 1:
//!   Content: {"title":"A Light in the Attic","price":"£51.77","availability":"In stock (22 available)","upc":"a897fe39b1053632"}
//!   Usage: AutomationUsage { prompt_tokens: 2860, completion_tokens: 168, total_tokens: 3028, llm_calls: 1, search_calls: 0, fetch_calls: 0, webbrowser_calls: 0, custom_tool_calls: {}, api_calls: 1 }
//!
//! === Completed in 2.48s ===

extern crate spider;

use spider::features::automation::{ModelEndpoint, RemoteMultimodalConfigs, VisionRouteMode};
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    env_logger::init();

    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    let url = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

    // ── Setup 1: Same provider, different models ──────────────────────
    //
    // Both models share the parent API URL and key from RemoteMultimodalConfigs.
    // Only the model_name differs per endpoint.
    //
    // let mm_config = RemoteMultimodalConfigs::new(
    //     "https://openrouter.ai/api/v1/chat/completions",
    //     "qwen/qwen-2.5-vl-72b-instruct",
    // )
    // .with_api_key(&api_key)
    // .with_dual_models(
    //     ModelEndpoint::new("qwen/qwen-2.5-vl-72b-instruct"), // vision
    //     ModelEndpoint::new("qwen/qwen-2.5-72b-instruct"),     // text (cheaper)
    // )
    // .with_vision_route_mode(VisionRouteMode::TextFirst);

    // ── Setup 2: Cross-provider, each model has its own URL + key ─────
    //
    // Vision model → OpenRouter (Qwen VL, vision-capable)
    // Text model   → OpenRouter (different model, cheaper)
    //
    // Each ModelEndpoint can override api_url and api_key independently.
    // Fields left as None inherit from the parent RemoteMultimodalConfigs.

    let mm_config = RemoteMultimodalConfigs::new(
        "https://openrouter.ai/api/v1/chat/completions", // default/fallback URL
        "qwen/qwen-2.5-vl-72b-instruct",                 // default/fallback model
    )
    .with_api_key(&api_key)
    .with_vision_model(
        // Vision model: explicit URL + key (or omit to inherit from parent)
        ModelEndpoint::new("qwen/qwen-2.5-vl-72b-instruct")
            .with_api_url("https://openrouter.ai/api/v1/chat/completions")
            .with_api_key(&api_key),
    )
    .with_text_model(
        // Text model: can point to a completely different provider
        // Here we use OpenRouter too, but you could use Groq, Together, etc:
        //   .with_api_url("https://api.groq.com/openai/v1/chat/completions")
        //   .with_api_key("gsk-your-groq-key")
        ModelEndpoint::new("qwen/qwen-2.5-72b-instruct")
            .with_api_url("https://openrouter.ai/api/v1/chat/completions")
            .with_api_key(&api_key),
    )
    .with_vision_route_mode(VisionRouteMode::TextFirst);

    // VisionRouteMode::TextFirst means:
    //   - Round 0: vision model (screenshot + HTML)
    //   - Subsequent rounds: text model (HTML only, no screenshot → saves ~35k tokens)
    //   - Stagnation or stuck ≥ 3: upgrade back to vision model

    // ── Configure extraction settings ─────────────────────────────────
    let mut mm_config = mm_config;
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 1;
    mm_config.cfg.request_json_object = true;
    mm_config.cfg.extraction_prompt =
        Some("Extract book details: title, price, availability, UPC, description.".to_string());

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
