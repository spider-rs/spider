#[cfg(any(feature = "openai", feature = "search_serper"))]
use std::env;

#[cfg(any(feature = "openai", feature = "search_serper"))]
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

#[cfg(feature = "openai")]
#[tokio::test]
async fn live_openai_prompt_smoke() -> Result<(), Box<dyn std::error::Error>> {
    if !run_live_tests() {
        eprintln!("Skipping live OpenAI smoke test (RUN_LIVE_TESTS not enabled).");
        return Ok(());
    }

    let key = env::var("OPENAI_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .expect("RUN_LIVE_TESTS is enabled, but OPENAI_API_KEY is missing");
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let agent = spider_agent::Agent::builder()
        .with_openai(key, model)
        .build()?;

    let output = agent
        .prompt(vec![spider_agent::Message::user("Reply with exactly: ok")])
        .await?;

    assert!(!output.trim().is_empty());
    Ok(())
}

#[cfg(feature = "search_serper")]
#[tokio::test]
async fn live_serper_search_smoke() -> Result<(), Box<dyn std::error::Error>> {
    if !run_live_tests() {
        eprintln!("Skipping live Serper smoke test (RUN_LIVE_TESTS not enabled).");
        return Ok(());
    }

    let key = env::var("SERPER_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .expect("RUN_LIVE_TESTS is enabled, but SERPER_API_KEY is missing");

    let agent = spider_agent::Agent::builder()
        .with_search_serper(key)
        .build()?;
    let results = agent.search("Spider Cloud web crawler").await?;
    assert!(!results.results.is_empty());

    Ok(())
}
