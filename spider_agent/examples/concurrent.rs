//! Concurrent execution example using spider_agent.
//!
//! Run with:
//! ```sh
//! OPENAI_API_KEY=your-key SERPER_API_KEY=your-key cargo run --example concurrent --features "openai search_serper"
//! ```

use spider_agent::Agent;
use std::sync::Arc;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let openai_key = std::env::var("OPENAI_API_KEY")
        .expect("OPENAI_API_KEY environment variable required");
    let serper_key = std::env::var("SERPER_API_KEY")
        .expect("SERPER_API_KEY environment variable required");

    // Create agent with Arc for sharing across tasks
    let agent = Arc::new(
        Agent::builder()
            .with_openai(openai_key, "gpt-4o-mini")
            .with_search_serper(serper_key)
            .with_max_concurrent_llm_calls(10)
            .build()?,
    );

    let queries = vec![
        "rust async programming",
        "rust web frameworks comparison",
        "rust database libraries",
        "rust error handling best practices",
        "rust testing strategies",
    ];

    println!("Executing {} searches concurrently...\n", queries.len());
    let start = Instant::now();

    // Spawn concurrent search tasks
    let mut handles = Vec::new();

    for query in queries {
        let agent = agent.clone();
        let query = query.to_string();

        handles.push(tokio::spawn(async move {
            let result = agent.search(&query).await;
            (query, result)
        }));
    }

    // Collect results
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await?);
    }

    let elapsed = start.elapsed();

    // Print results
    for (query, result) in results {
        match result {
            Ok(search_results) => {
                println!("'{}' -> {} results", query, search_results.len());
                if let Some(first) = search_results.results.first() {
                    println!("   Top result: {}", first.title);
                }
            }
            Err(e) => {
                println!("'{}' -> Error: {}", query, e);
            }
        }
    }

    println!("\nCompleted in {:?}", elapsed);

    // Now do concurrent extraction
    println!("\n---\n");
    println!("Fetching and extracting from multiple URLs concurrently...\n");

    let urls = vec![
        "https://example.com",
        "https://httpbin.org/html",
    ];

    let start = Instant::now();
    let mut handles = Vec::new();

    for url in urls {
        let agent = agent.clone();
        let url = url.to_string();

        handles.push(tokio::spawn(async move {
            // Fetch
            let fetch_result = agent.fetch(&url).await?;

            // Extract (this will use the LLM semaphore)
            let extracted = agent
                .extract(&fetch_result.html, "Extract the main content and any headings")
                .await?;

            Ok::<_, spider_agent::AgentError>((url, extracted))
        }));
    }

    // Collect results
    for handle in handles {
        match handle.await? {
            Ok((url, data)) => {
                println!("Extracted from {}:", url);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&data)
                        .unwrap_or_default()
                        .chars()
                        .take(300)
                        .collect::<String>()
                );
                println!();
            }
            Err(e) => {
                println!("Error: {}", e);
            }
        }
    }

    println!("Extraction completed in {:?}", start.elapsed());

    Ok(())
}
