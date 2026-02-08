//! Multimodal agent example with usage tracking.
//!
//! This example demonstrates:
//! - Concurrent search and extraction
//! - Usage statistics tracking (tokens, calls)
//! - Memory for storing session state
//!
//! Run with:
//! ```sh
//! OPENAI_API_KEY=your-key SERPER_API_KEY=your-key cargo run --example multimodal --features "openai search_serper"
//! ```

use spider_agent::{Agent, SearchOptions};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let openai_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY environment variable required");
    let serper_key =
        std::env::var("SERPER_API_KEY").expect("SERPER_API_KEY environment variable required");

    // Create agent with Arc for concurrent access
    let agent = Arc::new(
        Agent::builder()
            .with_openai(&openai_key, "gpt-4o-mini")
            .with_search_serper(&serper_key)
            .with_max_concurrent_llm_calls(5)
            .build()?,
    );

    println!("=== Multimodal Agent Example ===\n");

    // Store query in memory for session tracking
    agent.memory_set(
        "session_id",
        serde_json::json!(uuid::Uuid::new_v4().to_string()),
    );
    agent.memory_set("queries_processed", serde_json::json!(0));

    // Concurrent searches
    let queries = vec![
        "Rust async runtime comparison 2024",
        "Best Rust web frameworks",
        "Rust database libraries",
    ];

    println!("Running {} concurrent searches...\n", queries.len());

    let mut handles = Vec::new();
    for query in queries.clone() {
        let agent = agent.clone();
        let query = query.to_string();
        handles.push(tokio::spawn(async move {
            let opts = SearchOptions::new().with_limit(3);
            let result = agent.search_with_options(&query, opts).await;
            (query, result)
        }));
    }

    // Collect results
    let mut all_urls = Vec::new();
    for handle in handles {
        let (query, result) = handle.await?;
        match result {
            Ok(results) => {
                println!("'{}': {} results", query, results.len());
                for r in results.results.iter().take(2) {
                    all_urls.push(r.url.clone());
                }
            }
            Err(e) => println!("'{}': Error - {}", query, e),
        }
    }

    // Update memory
    let count = agent
        .memory_get("queries_processed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    agent.memory_set(
        "queries_processed",
        serde_json::json!(count + queries.len() as u64),
    );

    // Extract from a few pages concurrently
    println!(
        "\n=== Extracting from {} pages ===\n",
        all_urls.len().min(3)
    );

    let mut extract_handles = Vec::new();
    for url in all_urls.into_iter().take(3) {
        let agent = agent.clone();
        extract_handles.push(tokio::spawn(async move {
            match agent.fetch(&url).await {
                Ok(fetch) => {
                    let extraction = agent
                        .extract(
                            &fetch.html,
                            "Extract: title, main topic, key points (max 3)",
                        )
                        .await;
                    (url, Some(fetch.status), extraction.ok())
                }
                Err(_) => (url, None, None),
            }
        }));
    }

    for handle in extract_handles {
        let (url, status, extraction) = handle.await?;
        println!("URL: {}", url);
        if let Some(status) = status {
            println!("  Status: {}", status);
        }
        if let Some(data) = extraction {
            println!(
                "  Extracted: {}",
                serde_json::to_string(&data)
                    .unwrap_or_default()
                    .chars()
                    .take(150)
                    .collect::<String>()
            );
        }
        println!();
    }

    // Print usage statistics
    println!("=== Usage Statistics ===\n");
    let usage = agent.usage();
    println!("LLM Calls:        {}", usage.llm_calls);
    println!("Prompt Tokens:    {}", usage.prompt_tokens);
    println!("Completion Tokens:{}", usage.completion_tokens);
    println!("Total Tokens:     {}", usage.total_tokens());
    println!("Search Calls:     {}", usage.search_calls);
    println!("Fetch Calls:      {}", usage.fetch_calls);
    println!("Tool Calls:       {}", usage.tool_calls);

    // Print memory state
    println!("\n=== Session Memory ===\n");
    if let Some(session) = agent.memory_get("session_id") {
        println!("Session ID: {}", session);
    }
    if let Some(count) = agent.memory_get("queries_processed") {
        println!("Queries Processed: {}", count);
    }

    Ok(())
}

// Simple UUID generation (in real code, use uuid crate)
mod uuid {
    pub struct Uuid([u8; 16]);

    impl Uuid {
        pub fn new_v4() -> Self {
            let mut bytes = [0u8; 16];
            for b in &mut bytes {
                *b = fastrand::u8(..);
            }
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            Self(bytes)
        }

        #[allow(clippy::inherent_to_string)]
        pub fn to_string(&self) -> String {
            format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                self.0[0], self.0[1], self.0[2], self.0[3],
                self.0[4], self.0[5], self.0[6], self.0[7],
                self.0[8], self.0[9], self.0[10], self.0[11],
                self.0[12], self.0[13], self.0[14], self.0[15]
            )
        }
    }
}
