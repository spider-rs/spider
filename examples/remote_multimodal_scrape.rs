//! Remote multimodal automation example using OpenRouter.
//!
//! This example demonstrates how to use a remote multimodal model (via OpenRouter)
//! to extract structured data from web pages.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example remote_multimodal_scrape --features "spider/sync spider/chrome"
//! ```

// EXAMPLE output
// === Page Received ===
// URL: https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html

// === AI Extraction Results ===
// Result 1:
//   Input prompt: extract_book_details
//   Content output: {"book":{"availability":"In stock (22 available)","description":"It's hard to imagine a world without A Light in the Attic. This now-classic collection of poetry and drawings from Shel Silverstein celebrates its 20th anniversary with this special edition. Silverstein's humorous and creative verse can amuse the dowdiest of readers. Lemon-faced adults and fidgety kids sit still and read these rhythmic words and laugh and smile and love that Silverstein. Need proof of his genius? RockabyeRockabye baby, in the treetopDon't you know a treetopIs no safe place to rock?And who put you up there,And your cradle, too?Baby, I think someone down here'sGot it in for you. Shel, you never sounded so good.","number_of_reviews":"0","price":"£51.77","price_excl_tax":"£51.77","price_incl_tax":"£51.77","product_type":"Books","tax":"£0.00","title":"A Light in the Attic","upc":"a897fe39b1053632"}}
//   Usage: AutomationUsage { prompt_tokens: 3223, completion_tokens: 312, total_tokens: 3535, llm_calls: 1, search_calls: 0, fetch_calls: 0, webbrowser_calls: 0, custom_tool_calls: {}, api_calls: 1 }

// === Remote Multimodal Usage ===
// Usage 1: AutomationUsage { prompt_tokens: 3223, completion_tokens: 312, total_tokens: 3535, llm_calls: 1, search_calls: 0, fetch_calls: 0, webbrowser_calls: 0, custom_tool_calls: {}, api_calls: 1 }

extern crate spider;

use spider::features::automation::RemoteMultimodalConfigs;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    // Get API key from environment variable
    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    // Target URL - a book details page
    let url = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

    // Configure remote multimodal with OpenRouter
    let mut mm_config = RemoteMultimodalConfigs::new(
        "https://openrouter.ai/api/v1/chat/completions",
        "qwen/qwen-2-vl-72b-instruct", // OpenRouter model identifier
    );

    // Set the API key
    mm_config.api_key = Some(api_key);

    // Set up extraction prompt
    mm_config.system_prompt = Some(
        "Extract the following data from the webpage content and return as valid JSON: \
         Extract book details including title, price, availability, description, and any other relevant information."
            .to_string(),
    );

    // Configure extraction settings
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 1; // Single extraction pass
    mm_config.cfg.request_json_object = true;

    // Create website with remote multimodal config
    let mut website: Website = Website::new(url)
        .with_limit(1)
        .with_remote_multimodal(Some(mm_config))
        .build()
        .unwrap();

    // Start scraping - scrape() handles subscription internally and collects pages
    let start = tokio::time::Instant::now();
    website.scrape().await;
    let duration = start.elapsed();

    // Access collected pages after scrape completes
    if let Some(pages) = website.get_pages() {
        for page in pages {
            println!("=== Page Received ===");
            println!("URL: {}", page.get_url());

            // Check for remote multimodal extraction results
            if let Some(ref ai_data) = page.extra_remote_multimodal_data {
                println!("\n=== AI Extraction Results ===");
                for (i, result) in ai_data.iter().enumerate() {
                    println!("Result {}:", i + 1);
                    println!("  Input prompt: {}", result.input);
                    println!("  Content output: {}", result.content_output);
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

            // Check for remote multimodal usage statistics
            if let Some(ref usage) = page.remote_multimodal_usage {
                println!("\n=== Remote Multimodal Usage ===");
                for (i, u) in usage.iter().enumerate() {
                    println!("Usage {}: {:?}", i + 1, u);
                }
            }

            println!("\n=== Raw HTML (truncated) ===");
            let html = page.get_html();
            if html.len() > 500 {
                println!("{}...", &html[..500]);
            } else {
                println!("{}", html);
            }
        }
    }

    println!("\n=== Completed ===");
    println!("Time elapsed: {:?}", duration);
}
