//! End-to-end Spider Cloud pipeline from a single prompt.
//!
//! This example is designed for production-like workflows:
//! - takes one natural-language prompt
//! - plans a multi-step route sequence
//! - executes routes in order
//! - runs `unblocker` fallback when scrape fails
//! - optionally runs AI structured extraction when enabled
//! - prints a structured JSON execution report
//!
//! Run:
//! ```bash
//! SPIDER_CLOUD_API_KEY=your-key cargo run -p spider_agent --example spider_cloud_end_to_end \
//!   -- "Find top travel books on https://books.toscrape.com and return structured product fields"
//! ```
//!
//! Optional env vars:
//! - `SPIDER_CLOUD_API_URL` (default: `https://api.spider.cloud`)
//! - `SPIDER_CLOUD_TOOL_PREFIX` (default: `spider_cloud`)
//! - `SPIDER_CLOUD_RETURN_FORMAT` (default: `markdown`, supports `raw|bytes|markdown|commonmark|text`)
//! - `SPIDER_CLOUD_ENABLE_AI_ROUTES=1` (required for `/ai/*` routes)
//! - `SPIDER_CLOUD_INCLUDE_TRANSFORM=1` (only if transform is explicitly needed)
//! - `SPIDER_CLOUD_FORCE_UNBLOCKER=1` (always include unblocker step)
//! - `SPIDER_FLOW_PROMPT` (fallback prompt if CLI args are not provided)

use serde::Serialize;
use spider_agent::{Agent, SpiderCloudToolConfig};

#[derive(Debug, Clone)]
struct Step {
    suffix: &'static str,
    description: &'static str,
    body: serde_json::Value,
}

#[derive(Debug, Clone)]
struct PlanOptions {
    return_format: String,
    include_transform: bool,
    force_unblocker: bool,
    enable_ai_routes: bool,
}

#[derive(Debug, Clone, Serialize)]
struct StepReport {
    tool: String,
    description: String,
    success: bool,
    http_status: Option<u16>,
    status_field: Option<u64>,
    duration_elapsed_ms: Option<u64>,
    total_cost: Option<String>,
    preview: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RunReport {
    prompt: String,
    seed_url: String,
    query: String,
    return_format: String,
    ai_routes_enabled: bool,
    steps_planned: usize,
    steps: Vec<StepReport>,
    usage_total_custom_tool_calls: u64,
    usage_by_tool: std::collections::BTreeMap<String, u64>,
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

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

fn extract_first_url(prompt: &str) -> Option<String> {
    prompt
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| ",.;:!?()[]{}<>\"'".contains(c)))
        .find(|t| t.starts_with("https://") || t.starts_with("http://"))
        .map(|s| s.to_string())
}

fn query_from_prompt(prompt: &str, seed_url: &str) -> String {
    if let Some(idx) = prompt.find(seed_url) {
        let mut left = prompt[..idx].trim().to_string();
        for prep in [" on", " from", " at", " in", " for"] {
            if left.to_ascii_lowercase().ends_with(prep) {
                let new_len = left.len().saturating_sub(prep.len());
                left.truncate(new_len);
                left = left.trim_end().to_string();
                break;
            }
        }
        if !left.is_empty() {
            return left;
        }
    }

    let tokens: Vec<&str> = prompt
        .split_whitespace()
        .filter(|t| !t.contains(seed_url))
        .collect();
    let q = tokens.join(" ").trim().to_string();
    if !q.is_empty() {
        return q;
    }

    format!(
        "site:{} relevant pages",
        seed_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
    )
}

fn should_use_unblocker(prompt_lc: &str, force: bool) -> bool {
    force
        || contains_any(
            prompt_lc,
            &[
                "unblock",
                "bypass",
                "anti-bot",
                "antibot",
                "cloudflare",
                "challenge",
                "blocked",
                "bot protection",
            ],
        )
}

fn should_use_ai_extract(prompt_lc: &str, ai_enabled: bool) -> bool {
    ai_enabled
        && contains_any(
            prompt_lc,
            &[
                "structured",
                "schema",
                "json output",
                "typed output",
                "extract fields",
                "ai extract",
                "ai scrape",
            ],
        )
}

fn build_steps(prompt: &str, seed_url: &str, query: &str, opts: &PlanOptions) -> Vec<Step> {
    let prompt_lc = prompt.to_ascii_lowercase();
    let use_unblocker = should_use_unblocker(&prompt_lc, opts.force_unblocker);
    let use_ai_extract = should_use_ai_extract(&prompt_lc, opts.enable_ai_routes);

    let mut steps = vec![
        Step {
            suffix: "search",
            description: "discover candidate URLs",
            body: serde_json::json!({
                "search": query,
                "num": 5,
                "fetch_page_content": false
            }),
        },
        Step {
            suffix: "links",
            description: "extract first-layer internal links",
            body: serde_json::json!({
                "url": seed_url,
                "limit": 2
            }),
        },
        Step {
            suffix: "crawl",
            description: "crawl nearby pages to widen coverage",
            body: serde_json::json!({
                "url": seed_url,
                "limit": 2,
                "depth": 1,
                "return_format": opts.return_format
            }),
        },
        Step {
            suffix: "scrape",
            description: "extract page content using selected return_format",
            body: serde_json::json!({
                "url": seed_url,
                "return_format": opts.return_format,
                "metadata": true
            }),
        },
    ];

    if use_unblocker {
        steps.insert(
            3,
            Step {
                suffix: "unblocker",
                description: "attempt anti-bot resistant retrieval",
                body: serde_json::json!({
                    "url": seed_url,
                    "return_format": opts.return_format,
                    "metadata": true
                }),
            },
        );
    }

    if opts.include_transform {
        steps.push(Step {
            suffix: "transform",
            description: "optional post-processing transform",
            body: serde_json::json!({
                "url": seed_url,
                "return_format": opts.return_format,
                "metadata": true
            }),
        });
    }

    if use_ai_extract {
        steps.push(Step {
            suffix: "ai_scrape",
            description: "AI structured extraction",
            body: serde_json::json!({
                "url": seed_url,
                "prompt": "Extract a concise structured summary of the page with title, key_entities, and key_facts.",
                "cleaning_intent": "extraction",
                "metadata": true,
                "extraction_schema": {
                    "name": "page_structured_summary",
                    "description": "Structured summary extracted from target page",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string" },
                            "key_entities": {
                                "type": "array",
                                "items": { "type": "string" }
                            },
                            "key_facts": {
                                "type": "array",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["title"]
                    }
                }
            }),
        });
    }

    steps
}

fn summarize_json_payload(payload: &serde_json::Value) -> (Option<u64>, Option<u64>, Option<String>) {
    let first = if let Some(arr) = payload.as_array() {
        arr.first().unwrap_or(payload)
    } else {
        payload
    };

    let status = first.get("status").and_then(|v| v.as_u64());
    let duration = first.get("duration_elapsed_ms").and_then(|v| v.as_u64());
    let total_cost = first
        .get("costs")
        .and_then(|v| v.get("total_cost_formatted").or_else(|| v.get("total_cost")))
        .map(|v| v.to_string());

    (status, duration, total_cost)
}

fn preview_from_body(body: &str, max: usize) -> String {
    body.chars().take(max).collect::<String>()
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
    let return_format =
        std::env::var("SPIDER_CLOUD_RETURN_FORMAT").unwrap_or_else(|_| "markdown".into());
    let enable_ai_routes = env_flag("SPIDER_CLOUD_ENABLE_AI_ROUTES");
    let include_transform = env_flag("SPIDER_CLOUD_INCLUDE_TRANSFORM");
    let force_unblocker = env_flag("SPIDER_CLOUD_FORCE_UNBLOCKER");

    let prompt = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            std::env::var("SPIDER_FLOW_PROMPT").unwrap_or_else(|_| {
                "Find top travel books on https://books.toscrape.com and return structured product fields"
                    .to_string()
            })
        } else {
            args.join(" ")
        }
    };

    let seed_url = extract_first_url(&prompt).unwrap_or_else(|| "https://books.toscrape.com/".into());
    let query = query_from_prompt(&prompt, &seed_url);

    let cloud_cfg = SpiderCloudToolConfig::new(api_key)
        .with_api_url(api_url)
        .with_tool_name_prefix(tool_prefix.clone())
        .with_enable_ai_routes(enable_ai_routes);
    let agent = Agent::builder()
        .with_spider_cloud_config(cloud_cfg)
        .build()?;

    let plan_options = PlanOptions {
        return_format: return_format.clone(),
        include_transform,
        force_unblocker,
        enable_ai_routes,
    };
    let steps = build_steps(&prompt, &seed_url, &query, &plan_options);

    println!("=== Spider Cloud End-to-End ===");
    println!("Prompt: {}", prompt);
    println!("Seed URL: {}", seed_url);
    println!("Query: {}", query);
    println!("Return format: {}", return_format);
    println!("AI routes enabled: {}", enable_ai_routes);
    println!("Transform enabled: {}", include_transform);
    println!("Force unblocker: {}", force_unblocker);
    println!("Planned steps: {}", steps.len());

    let mut reports = Vec::with_capacity(steps.len());

    for step in steps {
        let name = tool_name(&tool_prefix, step.suffix);
        let body = step.body.to_string();
        println!("Running {} ({})", name, step.description);

        match agent.execute_custom_tool(&name, None, None, Some(&body)).await {
            Ok(result) => {
                let parsed =
                    serde_json::from_str::<serde_json::Value>(&result.body).unwrap_or(serde_json::Value::Null);
                let (status_field, duration_elapsed_ms, total_cost) = summarize_json_payload(&parsed);

                let preview = preview_from_body(&result.body, 240);
                println!("  -> success={} http={}", result.success, result.status);

                reports.push(StepReport {
                    tool: name.clone(),
                    description: step.description.to_string(),
                    success: result.success,
                    http_status: Some(result.status),
                    status_field,
                    duration_elapsed_ms,
                    total_cost,
                    preview: preview.clone(),
                    error: if result.success {
                        None
                    } else {
                        Some(format!("HTTP {} failure", result.status))
                    },
                });

                if !result.success && step.suffix == "scrape" && !force_unblocker {
                    let fallback_name = tool_name(&tool_prefix, "unblocker");
                    let fallback_body = serde_json::json!({
                        "url": seed_url,
                        "return_format": return_format,
                        "metadata": true
                    })
                    .to_string();

                    println!("  scrape failed, trying fallback {}", fallback_name);
                    match agent
                        .execute_custom_tool(&fallback_name, None, None, Some(&fallback_body))
                        .await
                    {
                        Ok(fallback) => {
                            let parsed = serde_json::from_str::<serde_json::Value>(&fallback.body)
                                .unwrap_or(serde_json::Value::Null);
                            let (status_field, duration_elapsed_ms, total_cost) =
                                summarize_json_payload(&parsed);
                            reports.push(StepReport {
                                tool: fallback_name.clone(),
                                description: "fallback unblocker after scrape failure".to_string(),
                                success: fallback.success,
                                http_status: Some(fallback.status),
                                status_field,
                                duration_elapsed_ms,
                                total_cost,
                                preview: preview_from_body(&fallback.body, 240),
                                error: if fallback.success {
                                    None
                                } else {
                                    Some(format!("HTTP {} fallback failure", fallback.status))
                                },
                            });
                        }
                        Err(err) => {
                            reports.push(StepReport {
                                tool: fallback_name,
                                description: "fallback unblocker after scrape failure".to_string(),
                                success: false,
                                http_status: None,
                                status_field: None,
                                duration_elapsed_ms: None,
                                total_cost: None,
                                preview: String::new(),
                                error: Some(err.to_string()),
                            });
                        }
                    }
                }
            }
            Err(err) => {
                reports.push(StepReport {
                    tool: name,
                    description: step.description.to_string(),
                    success: false,
                    http_status: None,
                    status_field: None,
                    duration_elapsed_ms: None,
                    total_cost: None,
                    preview: String::new(),
                    error: Some(err.to_string()),
                });
            }
        }
    }

    let usage = agent.usage();
    let mut usage_by_tool = std::collections::BTreeMap::new();
    for (tool, count) in &usage.custom_tool_calls {
        usage_by_tool.insert(tool.clone(), *count);
    }

    let report = RunReport {
        prompt,
        seed_url,
        query,
        return_format,
        ai_routes_enabled: enable_ai_routes,
        steps_planned: reports.len(),
        steps: reports,
        usage_total_custom_tool_calls: usage.total_custom_tool_calls(),
        usage_by_tool,
    };

    println!("\n=== End-to-End Report ===");
    println!("{}", serde_json::to_string_pretty(&report)?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(steps: &[Step]) -> Vec<&'static str> {
        steps.iter().map(|s| s.suffix).collect()
    }

    #[test]
    fn planner_includes_unblocker_on_blocked_intent() {
        let opts = PlanOptions {
            return_format: "markdown".to_string(),
            include_transform: false,
            force_unblocker: false,
            enable_ai_routes: false,
        };
        let steps = build_steps(
            "Collect blocked pages from https://example.com behind anti-bot challenge",
            "https://example.com",
            "collect blocked pages",
            &opts,
        );
        assert!(names(&steps).contains(&"unblocker"));
    }

    #[test]
    fn planner_ai_scrape_only_when_ai_enabled_and_requested() {
        let opts = PlanOptions {
            return_format: "markdown".to_string(),
            include_transform: false,
            force_unblocker: false,
            enable_ai_routes: true,
        };
        let steps = build_steps(
            "Use structured json output for https://example.com and extract fields",
            "https://example.com",
            "extract fields",
            &opts,
        );
        assert!(names(&steps).contains(&"ai_scrape"));

        let disabled_opts = PlanOptions {
            enable_ai_routes: false,
            ..opts
        };
        let disabled_steps = build_steps(
            "Use structured json output for https://example.com and extract fields",
            "https://example.com",
            "extract fields",
            &disabled_opts,
        );
        assert!(!names(&disabled_steps).contains(&"ai_scrape"));
    }

    #[test]
    fn planner_transform_is_opt_in() {
        let base = PlanOptions {
            return_format: "bytes".to_string(),
            include_transform: false,
            force_unblocker: false,
            enable_ai_routes: false,
        };
        let without = build_steps(
            "Extract docs for https://example.com",
            "https://example.com",
            "extract docs",
            &base,
        );
        assert!(!names(&without).contains(&"transform"));

        let with = build_steps(
            "Extract docs for https://example.com",
            "https://example.com",
            "extract docs",
            &PlanOptions {
                include_transform: true,
                ..base
            },
        );
        assert!(names(&with).contains(&"transform"));
    }

    #[test]
    fn query_builder_prefers_clean_left_side_of_url() {
        let q = query_from_prompt(
            "Find top travel books on https://books.toscrape.com and return structured fields",
            "https://books.toscrape.com",
        );
        assert_eq!(q, "Find top travel books");
    }
}
