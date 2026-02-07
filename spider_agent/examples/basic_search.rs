//! Basic search example using spider_agent.
//!
//! Run with:
//! ```sh
//! SERPER_API_KEY=your-key cargo run --example basic_search --features search_serper
//! ```

use spider_agent::{Agent, SearchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let api_key =
        std::env::var("SERPER_API_KEY").expect("SERPER_API_KEY environment variable required");

    let agent = Agent::builder().with_search_serper(api_key).build()?;

    // Basic search
    println!("Searching for 'rust web frameworks'...\n");
    let results = agent.search("rust web frameworks").await?;

    println!("Found {} results:\n", results.len());
    for result in &results.results {
        println!("{}. {}", result.position, result.title);
        println!("   URL: {}", result.url);
        if let Some(ref snippet) = result.snippet {
            println!("   {}", snippet);
        }
        println!();
    }

    // Search with options
    println!("\nSearching with limit of 3 results...\n");
    let results = agent
        .search_with_options(
            "rust async runtime comparison",
            SearchOptions::new()
                .with_limit(3)
                .with_country("us")
                .with_language("en"),
        )
        .await?;

    for result in &results.results {
        println!("- {} ({})", result.title, result.url);
    }

    Ok(())
}
