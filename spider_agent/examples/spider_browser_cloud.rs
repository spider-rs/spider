//! Spider Browser Cloud agent integration.
//!
//! Demonstrates using Spider Browser Cloud (`wss://browser.spider.cloud/v1/browser`)
//! as a remote CDP browser from the spider_agent.
//!
//! ```bash
//! SPIDER_CLOUD_API_KEY=your-key cargo run -p spider_agent --example spider_browser_cloud
//! ```
//!
//! Optional env vars:
//! - `SPIDER_BROWSER_STEALTH=1`
//! - `SPIDER_BROWSER_COUNTRY=us`

use spider_agent::{Agent, SpiderBrowserToolConfig};

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let api_key = std::env::var("SPIDER_CLOUD_API_KEY").expect("SPIDER_CLOUD_API_KEY must be set");

    let stealth = env_flag("SPIDER_BROWSER_STEALTH");
    let country = std::env::var("SPIDER_BROWSER_COUNTRY").ok();

    let mut cfg = SpiderBrowserToolConfig::new(&api_key).with_stealth(stealth);
    if let Some(ref c) = country {
        cfg = cfg.with_country(c);
    }

    println!("=== Spider Browser Cloud ===");
    println!("WSS URL: {}", cfg.connection_url());
    println!("Stealth: {}", stealth);
    if let Some(ref c) = country {
        println!("Country: {}", c);
    }

    let agent = Agent::builder().with_spider_browser_config(cfg).build()?;

    // List registered browser tools.
    let tools = agent.list_custom_tools();
    println!("\nRegistered tools ({}):", tools.len());
    for tool in &tools {
        println!("  - {}", tool);
    }

    // Demonstrate tool execution — navigate to a page.
    println!("\nExecuting spider_browser_navigate...");
    let body = serde_json::json!({ "url": "https://example.com" });
    let result = agent
        .execute_custom_tool(
            "spider_browser_navigate",
            None,
            None,
            Some(&body.to_string()),
        )
        .await;

    match result {
        Ok(r) => {
            println!("Status: {} (success={})", r.status, r.success);
            let preview = if r.body.len() > 200 {
                format!("{}...", &r.body[..200])
            } else {
                r.body.clone()
            };
            println!("Response: {}", preview);
        }
        Err(e) => {
            println!("Error: {}", e);
            println!("(This is expected if the WSS endpoint requires a live connection)");
        }
    }

    // Show how to combine with Spider Cloud API tools.
    println!("\n--- Combined with Spider Cloud API ---");
    let combined = Agent::builder()
        .with_spider_cloud(&api_key)
        .with_spider_browser(&api_key)
        .build()?;

    let all_tools = combined.list_custom_tools();
    println!("Total tools: {}", all_tools.len());
    let cloud_tools: Vec<_> = all_tools
        .iter()
        .filter(|t| t.starts_with("spider_cloud_"))
        .collect();
    let browser_tools: Vec<_> = all_tools
        .iter()
        .filter(|t| t.starts_with("spider_browser_"))
        .collect();
    println!("  Spider Cloud API tools: {}", cloud_tools.len());
    println!("  Spider Browser tools:   {}", browser_tools.len());

    Ok(())
}
