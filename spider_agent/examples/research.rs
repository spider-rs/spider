//! Research example using spider_agent.
//!
//! Run with:
//! ```sh
//! OPENAI_API_KEY=your-key SERPER_API_KEY=your-key cargo run --example research --features "openai search_serper"
//! ```

use spider_agent::{Agent, ResearchOptions, SearchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let openai_key = std::env::var("OPENAI_API_KEY")
        .expect("OPENAI_API_KEY environment variable required");
    let serper_key = std::env::var("SERPER_API_KEY")
        .expect("SERPER_API_KEY environment variable required");

    let agent = Agent::builder()
        .with_openai(openai_key, "gpt-4o-mini")
        .with_search_serper(serper_key)
        .build()?;

    println!("Researching: How do Tokio and async-std compare?\n");
    println!("This will search, fetch pages, extract data, and synthesize findings...\n");

    let research = agent
        .research(
            "How do Tokio and async-std compare for Rust async programming?",
            ResearchOptions::new()
                .with_max_pages(3)
                .with_search_options(SearchOptions::new().with_limit(5))
                .with_extraction_prompt(
                    "Extract key differences, pros, cons, and use cases for the async runtimes mentioned.",
                )
                .with_synthesize(true),
        )
        .await?;

    println!("=== Search Results ===");
    println!("Found {} results\n", research.search_results.len());

    println!("=== Extracted Data ===");
    for (i, extraction) in research.extractions.iter().enumerate() {
        println!("\n{}. {} ({})", i + 1, extraction.title, extraction.url);
        println!(
            "   Extracted: {}",
            serde_json::to_string(&extraction.extracted)
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        );
    }

    if let Some(summary) = research.summary {
        println!("\n=== Summary ===");
        println!("{}", summary);
    }

    println!("\n=== Token Usage ===");
    println!(
        "Prompt: {}, Completion: {}, Total: {}",
        research.usage.prompt_tokens,
        research.usage.completion_tokens,
        research.usage.total_tokens
    );

    Ok(())
}
