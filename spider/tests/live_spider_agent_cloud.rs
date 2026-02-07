#![cfg(feature = "agent")]

use spider::agent::Agent;
use std::env;

fn run_live_tests() -> bool {
    matches!(
        env::var("RUN_LIVE_TESTS")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn summarize_response_usage(value: &serde_json::Value) -> String {
    let first = if let Some(arr) = value.as_array() {
        arr.first()
    } else {
        Some(value)
    };

    let Some(first) = first else {
        return "empty-response".to_string();
    };

    let total_cost = first
        .get("costs")
        .and_then(|v| v.get("total_cost_formatted").or_else(|| v.get("total_cost")))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string());

    let status = first
        .get("status")
        .and_then(|v| v.as_u64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string());

    let duration_ms = first
        .get("duration_elapsed_ms")
        .and_then(|v| v.as_u64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string());

    format!(
        "status={}, duration_ms={}, total_cost={}",
        status, duration_ms, total_cost
    )
}

#[tokio::test]
async fn live_spider_agent_cloud_search_and_scrape() -> Result<(), Box<dyn std::error::Error>> {
    if !run_live_tests() {
        eprintln!("Skipping spider::agent live test (RUN_LIVE_TESTS not enabled).");
        return Ok(());
    }

    let api_key = env::var("SPIDER_CLOUD_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .expect("RUN_LIVE_TESTS is enabled, but SPIDER_CLOUD_API_KEY is missing");

    let agent = Agent::builder().with_spider_cloud(api_key).build()?;

    let search_body = serde_json::json!({
        "search": "books to scrape",
        "num": 3,
        "fetch_page_content": false
    })
    .to_string();

    let search_result = agent
        .execute_custom_tool("spider_cloud_search", None, None, Some(&search_body))
        .await?;
    assert!(
        search_result.success,
        "spider_cloud_search failed with HTTP {} body={}",
        search_result.status,
        search_result.body.chars().take(300).collect::<String>()
    );
    let search_json: serde_json::Value = serde_json::from_str(&search_result.body)?;
    eprintln!(
        "spider::agent search usage: {}",
        summarize_response_usage(&search_json)
    );

    let scrape_body = serde_json::json!({
        "url": "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html",
        "return_format": "markdown",
        "limit": 1
    })
    .to_string();

    let scrape_result = agent
        .execute_custom_tool("spider_cloud_scrape", None, None, Some(&scrape_body))
        .await?;
    assert!(
        scrape_result.success,
        "spider_cloud_scrape failed with HTTP {} body={}",
        scrape_result.status,
        scrape_result.body.chars().take(300).collect::<String>()
    );
    let scrape_json: serde_json::Value = serde_json::from_str(&scrape_result.body)?;
    eprintln!(
        "spider::agent scrape usage: {}",
        summarize_response_usage(&scrape_json)
    );

    let usage = agent.usage();
    eprintln!(
        "spider::agent usage snapshot: search_tool_calls={}, scrape_tool_calls={}, total_custom_tool_calls={}",
        usage.get_custom_tool_calls("spider_cloud_search"),
        usage.get_custom_tool_calls("spider_cloud_scrape"),
        usage.total_custom_tool_calls(),
    );
    assert_eq!(usage.get_custom_tool_calls("spider_cloud_search"), 1);
    assert_eq!(usage.get_custom_tool_calls("spider_cloud_scrape"), 1);

    Ok(())
}
