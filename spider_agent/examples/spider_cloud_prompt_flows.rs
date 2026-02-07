//! Spider Cloud prompt-driven tool flow example.
//!
//! A single natural-language prompt triggers one or more Spider Cloud tool
//! calls: search, scrape, crawl, links, transform, unblocker, and optional
//! AI routes.
//!
//! Run with:
//! ```bash
//! SPIDER_CLOUD_API_KEY=your-key cargo run --example spider_cloud_prompt_flows -p spider_agent
//! ```
//!
//! Example prompt:
//! ```text
//! run all flows for https://books.toscrape.com/ including search scrape crawl links transform unblocker
//! ```
//!
//! Optional env vars:
//! - `SPIDER_CLOUD_API_URL` (default: `https://api.spider.cloud`)
//! - `SPIDER_CLOUD_TOOL_PREFIX` (default: `spider_cloud`)
//! - `SPIDER_CLOUD_ENABLE_AI_ROUTES=1` (AI plan required)
//! - `SPIDER_FLOW_PROMPT` (fallback prompt if no CLI args passed)
//!
//! AI prompt example:
//! ```text
//! run all flows for https://books.toscrape.com/ and include ai routes
//! ```

use spider_agent::{Agent, SpiderCloudToolConfig};

#[derive(Debug, Clone)]
struct ToolFlow {
    suffix: &'static str,
    body: serde_json::Value,
    description: &'static str,
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

fn extract_first_url(prompt: &str) -> Option<String> {
    prompt
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| ",.;:!?()[]{}<>\"'".contains(c)))
        .find(|t| t.starts_with("https://") || t.starts_with("http://"))
        .map(|s| s.to_string())
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

fn wants_route(prompt: &str, run_all: bool, keywords: &[&str]) -> bool {
    run_all || contains_any(prompt, keywords)
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

fn build_flows(prompt: &str, enable_ai_routes: bool) -> Vec<ToolFlow> {
    let p = prompt.to_ascii_lowercase();
    let run_all = contains_any(&p, &["all flows", "all tools", "everything"]);

    let seed_url = extract_first_url(prompt).unwrap_or_else(|| "https://books.toscrape.com/".into());
    let detail_url = if seed_url.contains("books.toscrape.com") {
        "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html".to_string()
    } else {
        seed_url.clone()
    };

    let mut flows = Vec::new();

    if wants_route(&p, run_all, &["search"]) {
        flows.push(ToolFlow {
            suffix: "search",
            description: "Spider Cloud /search",
            body: serde_json::json!({
                "search": "books to scrape",
                "num": 3,
                "fetch_page_content": false
            }),
        });
    }
    if wants_route(&p, run_all, &["scrape"]) {
        flows.push(ToolFlow {
            suffix: "scrape",
            description: "Spider Cloud /scrape",
            body: serde_json::json!({
                "url": detail_url,
                "return_format": "markdown",
                "limit": 1
            }),
        });
    }
    if wants_route(&p, run_all, &["crawl"]) {
        flows.push(ToolFlow {
            suffix: "crawl",
            description: "Spider Cloud /crawl",
            body: serde_json::json!({
                "url": seed_url,
                "limit": 1,
                "return_format": "raw"
            }),
        });
    }
    if wants_route(&p, run_all, &["links"]) {
        flows.push(ToolFlow {
            suffix: "links",
            description: "Spider Cloud /links",
            body: serde_json::json!({
                "url": seed_url,
                "limit": 1
            }),
        });
    }
    if wants_route(&p, run_all, &["transform"]) {
        flows.push(ToolFlow {
            suffix: "transform",
            description: "Spider Cloud /transform",
            body: serde_json::json!({
                "url": detail_url,
                "return_format": "markdown",
                "limit": 1
            }),
        });
    }
    if wants_route(
        &p,
        run_all,
        &["unblocker", "unblock", "bypass", "anti-bot", "antibot"],
    ) {
        flows.push(ToolFlow {
            suffix: "unblocker",
            description: "Spider Cloud /unblocker",
            body: serde_json::json!({
                "url": detail_url,
                "return_format": "raw"
            }),
        });
    }

    let wants_ai_routes = enable_ai_routes
        && wants_route(
            &p,
            run_all,
            &[
                " ai",
                "ai ",
                "/ai/",
                "/ai",
                "ai routes",
                "ai route",
                "ai tools",
            ],
        );

    if wants_ai_routes {
        flows.push(ToolFlow {
            suffix: "ai_search",
            description: "Spider Cloud /ai/search (AI subscription)",
            body: serde_json::json!({
                "url": seed_url,
                "prompt": "find key pages about books",
                "limit": 2
            }),
        });
        flows.push(ToolFlow {
            suffix: "ai_scrape",
            description: "Spider Cloud /ai/scrape (AI subscription)",
            body: serde_json::json!({
                "url": detail_url,
                "prompt": "extract title, price, availability",
                "cleaning_intent": "extraction"
            }),
        });
        flows.push(ToolFlow {
            suffix: "ai_crawl",
            description: "Spider Cloud /ai/crawl (AI subscription)",
            body: serde_json::json!({
                "url": seed_url,
                "prompt": "crawl product pages and extract titles",
                "limit": 2
            }),
        });
        flows.push(ToolFlow {
            suffix: "ai_links",
            description: "Spider Cloud /ai/links (AI subscription)",
            body: serde_json::json!({
                "url": seed_url,
                "prompt": "find product detail links",
                "limit": 3
            }),
        });
        flows.push(ToolFlow {
            suffix: "ai_browser",
            description: "Spider Cloud /ai/browser (AI subscription)",
            body: serde_json::json!({
                "url": detail_url,
                "prompt": "open the page and extract title, price, and availability",
                "cleaning_intent": "general"
            }),
        });
    }

    if flows.is_empty() {
        flows.push(ToolFlow {
            suffix: "search",
            description: "Spider Cloud /search (default)",
            body: serde_json::json!({
                "search": "books to scrape",
                "num": 3,
                "fetch_page_content": false
            }),
        });
        flows.push(ToolFlow {
            suffix: "scrape",
            description: "Spider Cloud /scrape (default)",
            body: serde_json::json!({
                "url": detail_url,
                "return_format": "markdown",
                "limit": 1
            }),
        });
    }

    flows
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

    let prompt = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            std::env::var("SPIDER_FLOW_PROMPT").unwrap_or_else(|_| {
                "run all flows for https://books.toscrape.com/ including search scrape crawl links transform unblocker"
                    .to_string()
            })
        } else {
            args.join(" ")
        }
    };

    let cloud_cfg = SpiderCloudToolConfig::new(api_key)
        .with_api_url(api_url.clone())
        .with_tool_name_prefix(tool_prefix.clone())
        .with_enable_ai_routes(enable_ai_routes);
    let agent = Agent::builder()
        .with_spider_cloud_config(cloud_cfg)
        .build()?;

    println!("=== Spider Cloud Prompt Flows ===");
    println!("Prompt: {}", prompt);
    println!("API URL: {}", api_url);
    println!("Tool prefix: {}", tool_prefix);
    println!("AI routes enabled: {}", enable_ai_routes);
    println!();

    let flows = build_flows(&prompt, enable_ai_routes);
    println!("Planned {} tool call(s):", flows.len());
    for flow in &flows {
        println!("- {} ({})", tool_name(&tool_prefix, flow.suffix), flow.description);
    }
    println!();

    for flow in flows {
        let name = tool_name(&tool_prefix, flow.suffix);
        let body = flow.body.to_string();
        println!("Running {} ...", name);

        match agent
            .execute_custom_tool(&name, None, None, Some(&body))
            .await
        {
            Ok(result) => {
                if result.success {
                    let json_value = serde_json::from_str::<serde_json::Value>(&result.body)
                        .unwrap_or(serde_json::Value::Null);
                    println!("  ok: HTTP {} | {}", result.status, summarize_usage(&json_value));
                } else {
                    println!(
                        "  failed: HTTP {} body={}",
                        result.status,
                        result.body.chars().take(200).collect::<String>()
                    );
                }
            }
            Err(err) => {
                println!("  error: {}", err);
            }
        }
    }

    let usage = agent.usage();
    println!("\n=== Agent Usage Snapshot ===");
    println!("Total custom tool calls: {}", usage.total_custom_tool_calls());
    for (tool, count) in &usage.custom_tool_calls {
        println!("- {}: {}", tool, count);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_suffix(flows: &[ToolFlow], suffix: &str) -> bool {
        flows.iter().any(|f| f.suffix == suffix)
    }

    #[test]
    fn base_routes_selected_for_all_flows_prompt() {
        let flows = build_flows("run all flows for https://example.com", false);
        assert!(has_suffix(&flows, "search"));
        assert!(has_suffix(&flows, "scrape"));
        assert!(has_suffix(&flows, "crawl"));
        assert!(has_suffix(&flows, "links"));
        assert!(has_suffix(&flows, "transform"));
        assert!(has_suffix(&flows, "unblocker"));
        assert!(!has_suffix(&flows, "ai_search"));
    }

    #[test]
    fn ai_routes_selected_when_enabled_and_requested() {
        let flows = build_flows("include ai routes for https://example.com", true);
        assert!(has_suffix(&flows, "ai_search"));
        assert!(has_suffix(&flows, "ai_scrape"));
        assert!(has_suffix(&flows, "ai_crawl"));
        assert!(has_suffix(&flows, "ai_links"));
        assert!(has_suffix(&flows, "ai_browser"));
    }

    #[test]
    fn defaults_to_search_and_scrape_when_prompt_has_no_route_hints() {
        let flows = build_flows("do something useful", false);
        assert_eq!(flows.len(), 2);
        assert!(has_suffix(&flows, "search"));
        assert!(has_suffix(&flows, "scrape"));
    }
}
