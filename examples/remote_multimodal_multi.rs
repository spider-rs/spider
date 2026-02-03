//! Remote multimodal automation example with multi-page extraction.
//!
//! This example demonstrates how to use a remote multimodal model (via OpenRouter)
//! to extract structured data from multiple pages by starting from a category page
//! and following links to individual book detail pages.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example remote_multimodal_multi --features "spider/sync spider/chrome spider/agent_chrome"
//! ```

// EXAMPLE output
// === Page Received ===
// URL: https://books.toscrape.com/catalogue/shakespeares-sonnets_989/index.html
// === AI Extraction Results ===
// Result 1:
//   Input prompt: extract_book_details
//   Content output: {"book":{"title":"Shakespeare's Sonnets","price":"£20.66",...}}
//
// === Page Received ===
// URL: https://books.toscrape.com/catalogue/our-band-could-be-your-life-scenes-from-the-american-indie-underground-1981-1991_985/index.html
// === AI Extraction Results ===
// Result 1:
//   Input prompt: extract_book_details
//   Content output: {"book":{"title":"Our Band Could Be Your Life",...}}

extern crate spider;

use spider::features::automation::RemoteMultimodalConfigs;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    // Get API key from environment variable
    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    // Start from the poetry category page - will follow links to individual books
    let url = "https://books.toscrape.com/catalogue/category/books/poetry_23/index.html";

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
         Extract book details including title, price, availability, description, and any other relevant information. \
         If this is a category/listing page rather than a book detail page, return {\"page_type\": \"category\"}."
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
    // - Start from category page
    // - depth(2) to follow links from category to book pages
    // - limit(3) to process category page + 2 book detail pages
    let mut website: Website = Website::new(url)
        .with_depth(2)
        .with_limit(3)
        .with_remote_multimodal(Some(mm_config))
        .build()
        .unwrap();

    // Subscribe to receive pages
    let mut rx = website.subscribe(16).unwrap();

    // Spawn task to handle received pages
    let join_handle = tokio::spawn(async move {
        let mut book_count = 0;
        while let Ok(page) = rx.recv().await {
            let url = page.get_url();
            println!("\n=== Page Received ===");
            println!("URL: {}", url);

            // Check if this is a book detail page (not a category page)
            let is_book_page = !url.contains("/category/");

            if is_book_page {
                book_count += 1;
            }

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

            println!("\n--- Book pages extracted: {} ---", book_count);
        }
    });

    // Start crawling
    let start = tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    // Unsubscribe to close the channel and let the spawned task exit
    website.unsubscribe();

    // Wait for the spawned task to complete
    let _ = join_handle.await;

    println!("\n=== Completed ===");
    println!("Time elapsed: {:?}", duration);
    println!(
        "Total pages visited: {:?}",
        website.get_all_links_visited().await.len()
    );
}
