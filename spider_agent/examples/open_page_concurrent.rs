//! OpenPage concurrent spawning example.
//!
//! This example demonstrates the OpenPage action that spawns new browser pages
//! concurrently when the user asks to "go to" or "open" a new URL (rather than
//! clicking a link on the current page).
//!
//! Run with:
//! ```sh
//! OPEN_ROUTER=sk-or-your-key cargo run --example open_page_concurrent --features chrome
//! ```
//!
//! Or with OpenAI:
//! ```sh
//! OPENAI_API_KEY=your-key cargo run --example open_page_concurrent --features chrome
//! ```

use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use spider_agent::automation::{RemoteMultimodalConfig, RemoteMultimodalConfigs};
use spider_agent::{
    run_remote_multimodal_with_page, run_spawn_pages_with_options, SpawnPageOptions,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Get API configuration from environment
    let (api_url, api_key, model_name) = get_api_config()?;

    println!("=== OpenPage Concurrent Spawning Example ===\n");
    println!("API URL: {}", api_url);
    println!("Model: {}\n", model_name);

    // Launch browser
    println!("Launching browser...");
    let (browser, mut handler) = Browser::launch(BrowserConfig::builder().build()?).await?;

    // Spawn browser handler task
    let handle = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    let browser = Arc::new(browser);

    // Create a new page
    let page = browser.new_page("about:blank").await?;

    // Stay on blank page - don't navigate to example.com
    println!("Using blank page as starting point...");
    // page is already at about:blank from browser.new_page()
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Configure the remote multimodal engine
    let config = RemoteMultimodalConfigs::new(&api_url, &model_name)
        .with_api_key(&api_key)
        .with_cfg(
            RemoteMultimodalConfig::new()
                .with_max_rounds(2)
                .with_screenshot(true),
        )
        .with_system_prompt_extra(
            r#"
CRITICAL INSTRUCTION FOR THIS SESSION:
The user wants to open multiple URLs in parallel browser tabs.
When the user asks to "open", "go to", or "visit" multiple URLs, you MUST use the OpenPage action:
- Single URL: { "OpenPage": "https://example.com" }
- Multiple URLs: { "OpenPage": ["https://url1.com", "https://url2.com"] }

OpenPage spawns NEW browser tabs that run concurrently. This is different from Navigate which replaces the current page.
"#,
        )
        .with_user_message_extra(
            r#"
IGNORE THE CURRENT PAGE CONTENT. Do NOT click any links on the current page.

YOUR ONLY TASK: Use the OpenPage action to open these URLs in new browser tabs:
- https://httpbin.org/html
- https://www.rust-lang.org

REQUIRED OUTPUT FORMAT - copy this exactly but with done: false:
{
  "label": "Opening URLs in new tabs",
  "done": false,
  "steps": [
    { "OpenPage": ["https://httpbin.org/html", "https://www.rust-lang.org"] }
  ]
}

Do NOT use Click, Navigate, or any other action. ONLY use OpenPage.
"#,
        );

    // Run automation
    println!("Running automation with OpenPage instruction...\n");

    // Enable RUST_LOG=debug to see what actions the model returns
    let result = run_remote_multimodal_with_page(&config, &page, "about:blank").await?;

    println!("Automation Result:");
    println!(
        "  Full result: {:?}",
        serde_json::to_string_pretty(&serde_json::json!({
            "label": &result.label,
            "steps_executed": result.steps_executed,
            "success": result.success,
            "spawn_pages": &result.spawn_pages,
            "extracted": &result.extracted,
        }))
        .unwrap_or_default()
    );
    println!("  Label: {}", result.label);
    println!("  Steps executed: {}", result.steps_executed);
    println!("  Success: {}", result.success);

    // Check for spawn_pages
    if result.has_spawn_pages() {
        println!("\n  URLs to spawn in new pages:");
        for url in &result.spawn_pages {
            println!("    - {}", url);
        }

        // Spawn concurrent pages with extraction and screenshots enabled
        // This runs all pages in parallel using tokio::spawn internally
        println!("\nSpawning concurrent pages with extraction...\n");

        // Setup a page setup callback for event tracking propagation
        // In a real spider integration, this would setup ChromeEventTracker
        let page_setup = std::sync::Arc::new(Box::new(|_page: &chromiumoxide::Page| {
            Box::pin(async move {
                // Example: Setup event tracking on this spawned page
                // In spider, you would call setup_chrome_events(page, tracker) here
                // to propagate the network map from the main page
                log::debug!("Setting up event tracking on spawned page");
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        }) as spider_agent::PageSetupFn);

        // Configure what we want from each page
        let options = SpawnPageOptions::new()
            .with_extraction(
                "Extract the page title, main heading, and a brief description of the page content",
            )
            .with_screenshot(true)
            .with_max_rounds(1)
            .with_page_setup(page_setup)
            .with_track_bytes(true);

        let spawn_results =
            run_spawn_pages_with_options(&browser, result.spawn_pages, &config, options).await;

        // Process results - use the convenient accessor methods
        let mut successes = 0;
        let mut failures = 0;
        let mut total_tokens = 0u32;
        let mut total_bytes = 0.0f64;
        let mut total_requests = 0usize;

        for spawn_result in &spawn_results {
            if spawn_result.is_ok() {
                println!("Page '{}' completed:", spawn_result.url);

                // Get label
                if let Some(label) = spawn_result.label() {
                    println!("  Label: {}", label);
                }

                // Get extracted data
                if let Some(extracted) = spawn_result.extracted() {
                    println!(
                        "  Extracted: {}",
                        serde_json::to_string(extracted)
                            .unwrap_or_default()
                            .chars()
                            .take(300)
                            .collect::<String>()
                    );
                }

                // Check for screenshot
                if let Some(screenshot) = spawn_result.screenshot() {
                    println!("  Screenshot: {} bytes (base64)", screenshot.len());
                }

                // Show bytes transferred from network events
                if let Some(bytes) = spawn_result.bytes_transferred {
                    println!("  Bytes transferred: {:.0}", bytes);
                    total_bytes += bytes;
                }
                if let Some(ref response_map) = spawn_result.response_map {
                    println!("  Network requests tracked: {}", response_map.len());
                    total_requests += response_map.len();
                }

                // Accumulate token usage
                if let Some(usage) = spawn_result.usage() {
                    total_tokens += usage.total_tokens;
                }

                // Check for recursive spawn_pages (pages that want to open more pages)
                if let Some(more_pages) = spawn_result.spawn_pages() {
                    if !more_pages.is_empty() {
                        println!("  Additional pages to spawn: {:?}", more_pages);
                    }
                }

                successes += 1;
            } else {
                println!(
                    "Page '{}' error: {}",
                    spawn_result.url,
                    spawn_result.error().unwrap_or("unknown")
                );
                failures += 1;
            }
            println!();
        }

        println!("\n=== Summary ===");
        println!("Concurrent pages completed: {}", successes);
        println!("Failed: {}", failures);
        if total_tokens > 0 {
            println!("Spawned pages total tokens: {}", total_tokens);
        }
        if total_bytes > 0.0 {
            println!(
                "Total bytes transferred: {:.0} ({:.2} KB)",
                total_bytes,
                total_bytes / 1024.0
            );
            println!("Total network requests: {}", total_requests);
        }
    } else {
        println!("\n  No URLs to spawn in new pages.");
        println!("  (The model didn't use OpenPage action)");
    }

    // Print usage
    println!("\n=== Token Usage ===");
    println!("  Prompt tokens: {}", result.usage.prompt_tokens);
    println!("  Completion tokens: {}", result.usage.completion_tokens);
    println!("  Total tokens: {}", result.usage.total_tokens);

    // Clean up
    drop(browser);
    handle.abort();

    Ok(())
}

/// Get API configuration from environment variables.
fn get_api_config() -> Result<(String, String, String), Box<dyn std::error::Error>> {
    // Try OpenRouter first
    if let Ok(key) = std::env::var("OPEN_ROUTER") {
        return Ok((
            "https://openrouter.ai/api/v1/chat/completions".to_string(),
            key,
            std::env::var("MODEL_NAME")
                .unwrap_or_else(|_| "anthropic/claude-3.5-sonnet".to_string()),
        ));
    }

    // Then try OpenAI
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        return Ok((
            "https://api.openai.com/v1/chat/completions".to_string(),
            key,
            std::env::var("MODEL_NAME").unwrap_or_else(|_| "gpt-4o".to_string()),
        ));
    }

    // Then try local Ollama
    if let Ok(url) = std::env::var("OLLAMA_URL") {
        return Ok((
            url,
            String::new(),
            std::env::var("MODEL_NAME").unwrap_or_else(|_| "qwen2.5-vl".to_string()),
        ));
    }

    Err("Please set OPEN_ROUTER, OPENAI_API_KEY, or OLLAMA_URL environment variable".into())
}
