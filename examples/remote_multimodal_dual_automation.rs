//! Dual-model multi-round automation example.
//!
//! Shows dual-model routing for multi-round browser automation (not just
//! extraction). A vision model handles the first round and stagnation,
//! while a cheaper text model drives mid-round actions via HTML context.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example remote_multimodal_dual_automation --features "spider/sync spider/chrome spider/agent_chrome"
//! ```

extern crate spider;

use spider::features::automation::{ModelEndpoint, RemoteMultimodalConfigs, VisionRouteMode};
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    env_logger::init();

    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    // A page that requires multi-round interaction
    let url = "https://books.toscrape.com/";

    // ── Dual-model routing for automation ─────────────────────────────
    //
    // VisionFirst: vision model for rounds 0-1 (screenshot), then text model
    // for stable mid-rounds, upgrading back to vision on stagnation/stuck.
    //
    // You can also use different providers for each model by setting
    // `api_url` and `api_key` on the ModelEndpoint:
    //
    //   ModelEndpoint::new("gpt-4o")
    //       .with_api_url("https://api.openai.com/v1/chat/completions")
    //       .with_api_key("sk-openai-...")
    //
    // Fields left as None inherit from the parent RemoteMultimodalConfigs.

    let mm_config = RemoteMultimodalConfigs::new(
        "https://openrouter.ai/api/v1/chat/completions",
        "qwen/qwen-2.5-vl-72b-instruct",
    )
    .with_api_key(&api_key)
    .with_dual_models(
        ModelEndpoint::new("qwen/qwen-2.5-vl-72b-instruct"), // vision rounds
        ModelEndpoint::new("qwen/qwen-2.5-72b-instruct"),    // text rounds
    )
    .with_vision_route_mode(VisionRouteMode::VisionFirst);

    // ── Multi-round automation config ─────────────────────────────────
    let mut mm_config = mm_config;
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 4; // Multi-round: full automation prompt
    mm_config.cfg.request_json_object = true;
    mm_config.cfg.extraction_prompt = Some(
        "Navigate to the first book in the catalog. Extract its title, price, and availability."
            .to_string(),
    );

    // Optional: set the user instruction for the automation task
    mm_config.user_message_extra = Some(
        "Click on the first book link to navigate to its detail page, then extract the book data."
            .to_string(),
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
                println!("\n=== AI Results ===");
                for (i, result) in ai_data.iter().enumerate() {
                    println!("Result {}:", i + 1);
                    println!("  Content: {}", result.content_output);
                    if let Some(ref usage) = result.usage {
                        println!(
                            "  Tokens: {} prompt + {} completion = {} total ({} LLM calls)",
                            usage.prompt_tokens,
                            usage.completion_tokens,
                            usage.total_tokens,
                            usage.llm_calls,
                        );
                    }
                }
            }
        }
    });

    let start = tokio::time::Instant::now();
    website.crawl().await;
    website.unsubscribe();
    let _ = join_handle.await;

    println!("\n=== Completed in {:?} ===", start.elapsed());
}
