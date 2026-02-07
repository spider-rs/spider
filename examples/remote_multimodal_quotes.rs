//! Remote multimodal extraction example for quotes.
//!
//! This example extracts structured quote data (quote text, author, tags)
//! from a page and prints model usage metrics.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example remote_multimodal_quotes --features "spider/sync spider/chrome spider/agent_chrome"
//! ```
//!
//! EXAMPLE output
//! === Page Received ===
//! URL: https://quotes.toscrape.com/
//!
//! === AI Extraction Results ===
//! Result 1:
//!   Input prompt: extract_quotes
//!   Content output: {"quotes":[{"text":"The world as we have created it is a process of our thinking.","author":"Albert Einstein","tags":["change","deep-thoughts","thinking","world"]}]}
//!   Usage: AutomationUsage { prompt_tokens: 2410, completion_tokens: 196, total_tokens: 2606, llm_calls: 1, search_calls: 0, fetch_calls: 0, webbrowser_calls: 0, custom_tool_calls: {}, api_calls: 1 }

extern crate spider;

use spider::features::automation::RemoteMultimodalConfigs;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    let url = "https://quotes.toscrape.com/";

    let mut mm_config = RemoteMultimodalConfigs::new(
        "https://openrouter.ai/api/v1/chat/completions",
        "qwen/qwen-2-vl-72b-instruct",
    );
    mm_config.api_key = Some(api_key);
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 1;
    mm_config.cfg.request_json_object = true;
    mm_config.cfg.extraction_prompt = Some(
        "extract_quotes: return JSON with key `quotes` containing up to 5 entries. \
         Each entry should include `text`, `author`, and `tags`."
            .to_string(),
    );

    let mut website: Website = Website::new(url)
        .with_limit(1)
        .with_remote_multimodal(Some(mm_config))
        .build()
        .unwrap();

    website.scrape().await;

    if let Some(pages) = website.get_pages() {
        for page in pages {
            println!("=== Page Received ===");
            println!("URL: {}", page.get_url());

            if let Some(ref ai_data) = page.extra_remote_multimodal_data {
                println!("\n=== AI Extraction Results ===");
                for (i, result) in ai_data.iter().enumerate() {
                    println!("Result {}:", i + 1);
                    println!("  Input prompt: {}", result.input);
                    println!("  Content output: {}", result.content_output);
                    if let Some(ref usage) = result.usage {
                        println!("  Usage: {:?}", usage);
                    }
                    if let Some(ref err) = result.error {
                        println!("  Error: {}", err);
                    }
                }
            } else {
                println!("\nNo AI extraction data available.");
            }
        }
    }
}
