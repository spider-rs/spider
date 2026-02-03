//! Standalone extraction example using spider_agent's RemoteMultimodalEngine.
//!
//! This example demonstrates extracting structured data from HTML using
//! spider_agent without requiring the full spider crate or Chrome.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example extraction -p spider_agent
//! ```

use spider_agent::automation::RemoteMultimodalEngine;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    // Get API key from environment variable
    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    // Create the engine with OpenRouter
    let mut engine = RemoteMultimodalEngine::new(
        "https://openrouter.ai/api/v1/chat/completions",
        "qwen/qwen-2-vl-72b-instruct",
        None, // Use default system prompt
    )
    .with_api_key(Some(&api_key));

    // Configure extraction settings
    engine.cfg.extra_ai_data = true;
    engine.cfg.include_html = true;
    engine.cfg.request_json_object = true;
    engine.cfg.max_tokens = 1024;

    // Set custom extraction instructions
    engine.user_message_extra = Some(
        "Extract the book details including: title, price, availability, description, and UPC code."
            .to_string(),
    );

    // Sample HTML (simulating fetched content)
    let html = r#"
    <!DOCTYPE html>
    <html>
    <head><title>A Light in the Attic | Books to Scrape</title></head>
    <body>
        <div class="product_main">
            <h1>A Light in the Attic</h1>
            <p class="price_color">£51.77</p>
            <p class="instock availability">In stock (22 available)</p>
            <table class="table table-striped">
                <tr><th>UPC</th><td>a897fe39b1053632</td></tr>
                <tr><th>Product Type</th><td>Books</td></tr>
                <tr><th>Price (excl. tax)</th><td>£51.77</td></tr>
                <tr><th>Price (incl. tax)</th><td>£51.77</td></tr>
                <tr><th>Tax</th><td>£0.00</td></tr>
                <tr><th>Number of reviews</th><td>0</td></tr>
            </table>
        </div>
        <div id="product_description">
            <p>It's hard to imagine a world without A Light in the Attic.
            This now-classic collection of poetry and drawings from Shel Silverstein
            celebrates its 20th anniversary with this special edition.</p>
        </div>
    </body>
    </html>
    "#;

    println!("=== Spider Agent Extraction Example ===\n");

    // Extract data from HTML
    let start = std::time::Instant::now();
    let result = engine
        .extract_from_html(
            html,
            "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html",
            Some("A Light in the Attic | Books to Scrape"),
        )
        .await?;
    let duration = start.elapsed();

    println!("Label: {}", result.label);
    println!("Success: {}", result.success);

    if let Some(extracted) = &result.extracted {
        println!("\nExtracted Data:");
        println!("{}", serde_json::to_string_pretty(extracted)?);
    }

    println!("\nUsage:");
    println!("  Prompt tokens: {}", result.usage.prompt_tokens);
    println!("  Completion tokens: {}", result.usage.completion_tokens);
    println!("  Total tokens: {}", result.usage.total_tokens);
    println!("  LLM calls: {}", result.usage.llm_calls);

    println!("\nTime elapsed: {:?}", duration);

    Ok(())
}
