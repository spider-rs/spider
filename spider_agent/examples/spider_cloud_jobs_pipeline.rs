//! Real-world Spider Cloud example: job market intelligence pipeline.
//!
//! This flow demonstrates automated hiring-intel collection:
//! - `search` discover current postings/pages
//! - `crawl` gather nearby pages for context
//! - `unblocker` handle harder anti-bot pages
//! - `scrape` extract listing text/content
//! - optional `transform` normalize output for alerting/storage
//! - optional `ai_scrape` for typed extraction
//!
//! Run:
//! ```bash
//! SPIDER_CLOUD_API_KEY=your-key cargo run -p spider_agent --example spider_cloud_jobs_pipeline \
//!   -- "rust engineer remote" "https://remoteok.com/remote-rust-jobs"
//! ```
//!
//! Optional env vars:
//! - `SPIDER_CLOUD_API_URL` (default: `https://api.spider.cloud`)
//! - `SPIDER_CLOUD_TOOL_PREFIX` (default: `spider_cloud`)
//! - `SPIDER_CLOUD_ENABLE_AI_ROUTES=1` (required for `/ai/*` routes)
//! - `SPIDER_CLOUD_RETURN_FORMAT` (default: `markdown`, supports `raw|bytes|markdown|commonmark|text`)
//! - `SPIDER_CLOUD_INCLUDE_TRANSFORM=1` to explicitly run `/transform`

use spider_agent::{Agent, SpiderCloudToolConfig};

#[derive(Debug, Clone)]
struct Step {
    suffix: &'static str,
    description: &'static str,
    body: serde_json::Value,
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn tool_name(prefix: &str, suffix: &str) -> String {
    let p = prefix.trim().trim_end_matches('_');
    if p.is_empty() {
        suffix.to_string()
    } else {
        format!("{}_{}", p, suffix)
    }
}

fn summarize_usage(value: &serde_json::Value) -> String {
    let first = if let Some(arr) = value.as_array() {
        arr.first()
    } else {
        Some(value)
    };
    let Some(first) = first else {
        return "empty-response".to_string();
    };

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
    let total_cost = first
        .get("costs")
        .and_then(|v| v.get("total_cost_formatted").or_else(|| v.get("total_cost")))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string());

    format!(
        "status={}, duration_ms={}, total_cost={}",
        status, duration_ms, total_cost
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let api_key = std::env::var("SPIDER_CLOUD_API_KEY")
        .expect("SPIDER_CLOUD_API_KEY environment variable must be set");
    let api_url =
        std::env::var("SPIDER_CLOUD_API_URL").unwrap_or_else(|_| "https://api.spider.cloud".into());
    let tool_prefix =
        std::env::var("SPIDER_CLOUD_TOOL_PREFIX").unwrap_or_else(|_| "spider_cloud".into());
    let enable_ai_routes = env_flag("SPIDER_CLOUD_ENABLE_AI_ROUTES");
    let include_transform = env_flag("SPIDER_CLOUD_INCLUDE_TRANSFORM");
    let return_format =
        std::env::var("SPIDER_CLOUD_RETURN_FORMAT").unwrap_or_else(|_| "markdown".into());

    let args: Vec<String> = std::env::args().skip(1).collect();
    let search_query = args
        .first()
        .cloned()
        .unwrap_or_else(|| "rust engineer remote".to_string());
    let seed_url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://remoteok.com/remote-rust-jobs".to_string());

    let cloud_cfg = SpiderCloudToolConfig::new(api_key)
        .with_api_url(api_url.clone())
        .with_tool_name_prefix(tool_prefix.clone())
        .with_enable_ai_routes(enable_ai_routes);
    let agent = Agent::builder()
        .with_spider_cloud_config(cloud_cfg)
        .build()?;

    println!("=== Job Market Intelligence Pipeline ===");
    println!("Search query: {}", search_query);
    println!("Seed URL: {}", seed_url);
    println!("API URL: {}", api_url);
    println!("AI routes enabled: {}", enable_ai_routes);
    println!("Return format: {}", return_format);
    println!("Include transform: {}", include_transform);
    println!();

    let mut steps = vec![
        Step {
            suffix: "search",
            description: "discover relevant job pages",
            body: serde_json::json!({
                "search": search_query,
                "num": 5,
                "fetch_page_content": false
            }),
        },
        Step {
            suffix: "crawl",
            description: "crawl adjacent pages for related openings",
            body: serde_json::json!({
                "url": seed_url,
                "limit": 2,
                "depth": 1,
                "return_format": return_format
            }),
        },
        Step {
            suffix: "unblocker",
            description: "attempt anti-bot resistant retrieval",
            body: serde_json::json!({
                "url": seed_url,
                "return_format": return_format,
                "metadata": true
            }),
        },
        Step {
            suffix: "scrape",
            description: "extract normalized listing text",
            body: serde_json::json!({
                "url": seed_url,
                "return_format": return_format,
                "metadata": true
            }),
        },
    ];

    if include_transform {
        steps.push(Step {
            suffix: "transform",
            description: "produce transform-ready output",
            body: serde_json::json!({
                "url": seed_url,
                "return_format": return_format,
                "metadata": true
            }),
        });
    }

    if enable_ai_routes {
        steps.push(Step {
            suffix: "ai_scrape",
            description: "AI extraction for hiring signal fields",
            body: serde_json::json!({
                "url": seed_url,
                "prompt": "Extract up to 5 job postings with title, company, location, and url.",
                "cleaning_intent": "extraction",
                "metadata": true,
                "extraction_schema": {
                    "name": "job_postings",
                    "description": "Structured job listing extraction",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "jobs": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "title": { "type": "string" },
                                        "company": { "type": "string" },
                                        "location": { "type": "string" },
                                        "url": { "type": "string" }
                                    },
                                    "required": ["title", "company"]
                                }
                            }
                        },
                        "required": ["jobs"]
                    }
                }
            }),
        });
    }

    for step in steps {
        let name = tool_name(&tool_prefix, step.suffix);
        let body = step.body.to_string();
        println!("Running {} ({})", name, step.description);

        match agent.execute_custom_tool(&name, None, None, Some(&body)).await {
            Ok(result) => {
                if !result.success {
                    println!(
                        "  failed: HTTP {} body={}",
                        result.status,
                        result.body.chars().take(240).collect::<String>()
                    );
                    continue;
                }

                let parsed = serde_json::from_str::<serde_json::Value>(&result.body)
                    .unwrap_or(serde_json::Value::Null);
                println!("  ok: HTTP {} | {}", result.status, summarize_usage(&parsed));
                println!(
                    "  preview: {}",
                    result.body.chars().take(180).collect::<String>()
                );
            }
            Err(err) => println!("  error: {}", err),
        }
    }

    let usage = agent.usage();
    println!("\n=== Usage Snapshot ===");
    println!("Total custom tool calls: {}", usage.total_custom_tool_calls());
    for (tool, count) in &usage.custom_tool_calls {
        println!("- {}: {}", tool, count);
    }

    Ok(())
}
