use spider_agent::{Agent, SpiderBrowserToolConfig};
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

#[tokio::test]
async fn live_spider_browser_tool_registration() -> Result<(), Box<dyn std::error::Error>> {
    if !run_live_tests() {
        eprintln!("Skipping Spider Browser live test (RUN_LIVE_TESTS not enabled).");
        return Ok(());
    }

    let api_key = env::var("SPIDER_CLOUD_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .expect("RUN_LIVE_TESTS is enabled, but SPIDER_CLOUD_API_KEY is missing");

    let cfg = SpiderBrowserToolConfig::new(&api_key).with_stealth(true);
    let expected_url = cfg.connection_url();
    assert!(expected_url.contains("token="));
    assert!(expected_url.contains("stealth=true"));

    let agent = Agent::builder().with_spider_browser_config(cfg).build()?;

    assert!(agent.has_custom_tool("spider_browser_navigate"));
    assert!(agent.has_custom_tool("spider_browser_html"));
    assert!(agent.has_custom_tool("spider_browser_screenshot"));
    assert!(agent.has_custom_tool("spider_browser_evaluate"));
    assert!(agent.has_custom_tool("spider_browser_click"));
    assert!(agent.has_custom_tool("spider_browser_fill"));
    assert!(agent.has_custom_tool("spider_browser_wait"));

    eprintln!("Spider Browser tools registered successfully");
    eprintln!("Connection URL: {}", expected_url);

    Ok(())
}

#[tokio::test]
async fn live_spider_browser_navigate() -> Result<(), Box<dyn std::error::Error>> {
    if !run_live_tests() {
        eprintln!("Skipping Spider Browser navigate test (RUN_LIVE_TESTS not enabled).");
        return Ok(());
    }

    let api_key = env::var("SPIDER_CLOUD_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .expect("RUN_LIVE_TESTS is enabled, but SPIDER_CLOUD_API_KEY is missing");

    let agent = Agent::builder().with_spider_browser(&api_key).build()?;

    let body = serde_json::json!({ "url": "https://example.com" }).to_string();

    let result = agent
        .execute_custom_tool("spider_browser_navigate", None, None, Some(&body))
        .await;

    match result {
        Ok(r) => {
            eprintln!(
                "Navigate result: status={} success={} body_len={}",
                r.status,
                r.success,
                r.body.len()
            );
        }
        Err(e) => {
            eprintln!(
                "Navigate error (may be expected for WSS-only endpoint): {}",
                e
            );
        }
    }

    let usage = agent.usage();
    eprintln!(
        "Total custom tool calls: {}",
        usage.total_custom_tool_calls()
    );

    Ok(())
}

#[tokio::test]
async fn live_spider_browser_combined_with_cloud() -> Result<(), Box<dyn std::error::Error>> {
    if !run_live_tests() {
        eprintln!("Skipping combined test (RUN_LIVE_TESTS not enabled).");
        return Ok(());
    }

    let api_key = env::var("SPIDER_CLOUD_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .expect("RUN_LIVE_TESTS is enabled, but SPIDER_CLOUD_API_KEY is missing");

    let agent = Agent::builder()
        .with_spider_cloud(&api_key)
        .with_spider_browser(&api_key)
        .build()?;

    let tools = agent.list_custom_tools();
    let cloud_count = tools
        .iter()
        .filter(|t| t.starts_with("spider_cloud_"))
        .count();
    let browser_count = tools
        .iter()
        .filter(|t| t.starts_with("spider_browser_"))
        .count();

    eprintln!(
        "Combined agent: {} cloud tools + {} browser tools = {} total",
        cloud_count,
        browser_count,
        tools.len()
    );

    assert!(cloud_count >= 6, "expected at least 6 cloud tools");
    assert_eq!(browser_count, 7, "expected 7 browser tools");

    Ok(())
}
