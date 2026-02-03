//! Extraction example using spider_agent.
//!
//! Run with:
//! ```sh
//! OPENAI_API_KEY=your-key cargo run --example extract --features openai
//! ```

use spider_agent::Agent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let api_key = std::env::var("OPENAI_API_KEY")
        .expect("OPENAI_API_KEY environment variable required");

    let agent = Agent::builder()
        .with_openai(api_key, "gpt-4o-mini")
        .build()?;

    // Fetch a page
    println!("Fetching https://example.com...\n");
    let result = agent.fetch("https://example.com").await?;
    println!("Status: {}", result.status);
    println!("Content-Type: {}", result.content_type);
    println!("HTML length: {} bytes\n", result.html.len());

    // Extract data
    println!("Extracting data...\n");
    let data = agent
        .extract(
            &result.html,
            "Extract the main heading and any links on the page. Return as JSON with 'heading' and 'links' fields.",
        )
        .await?;

    println!("Extracted data:");
    println!("{}", serde_json::to_string_pretty(&data)?);

    // Extract with schema
    println!("\nExtracting with schema...\n");
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "title": { "type": "string", "description": "Page title or main heading" },
            "description": { "type": "string", "description": "Page description or summary" },
            "links": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "url": { "type": "string" }
                    }
                }
            }
        }
    });

    let structured_data = agent.extract_structured(&result.html, &schema).await?;
    println!("Structured data:");
    println!("{}", serde_json::to_string_pretty(&structured_data)?);

    Ok(())
}
