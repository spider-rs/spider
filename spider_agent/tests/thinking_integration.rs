//! Integration tests for thinking/extended thinking support.
//!
//! Unit tests validate response parsing for both OpenAI and Anthropic formats.
//! Live tests (gated behind RUN_LIVE_TESTS=1) hit real endpoints.

use serde_json::json;
use spider_agent_types::{
    extract_assistant_content, extract_thinking_content, extract_usage, is_anthropic_endpoint,
    parse_tool_calls,
};

// ── Unit: Anthropic Messages API response parsing ──

#[test]
fn anthropic_response_with_thinking() {
    let resp = json!({
        "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
        "type": "message",
        "role": "assistant",
        "content": [
            {
                "type": "thinking",
                "thinking": "The user wants me to extract data from an HTML page. Let me parse the content carefully."
            },
            {
                "type": "text",
                "text": "{\"label\": \"extract_products\", \"done\": true, \"steps\": [], \"extracted\": {\"products\": [{\"name\": \"Widget\", \"price\": 9.99}]}}"
            }
        ],
        "model": "claude-sonnet-4-20250514",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1523,
            "output_tokens": 256,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0
        }
    });

    // Text content extracted (skipping thinking blocks)
    let content = extract_assistant_content(&resp).unwrap();
    assert!(content.contains("extract_products"));
    assert!(content.contains("Widget"));

    // Thinking content extracted
    let thinking = extract_thinking_content(&resp).unwrap();
    assert!(thinking.contains("parse the content carefully"));

    // Usage mapped correctly
    let usage = extract_usage(&resp);
    assert_eq!(usage.prompt_tokens, 1523);
    assert_eq!(usage.completion_tokens, 256);
    assert_eq!(usage.api_calls, 1);
}

#[test]
fn anthropic_response_without_thinking() {
    let resp = json!({
        "id": "msg_02",
        "type": "message",
        "role": "assistant",
        "content": [
            {
                "type": "text",
                "text": "{\"label\": \"done\", \"done\": true, \"steps\": []}"
            }
        ],
        "usage": {
            "input_tokens": 500,
            "output_tokens": 30
        }
    });

    let content = extract_assistant_content(&resp).unwrap();
    assert!(content.contains("done"));
    assert!(extract_thinking_content(&resp).is_none());
}

#[test]
fn anthropic_tool_use_response() {
    let resp = json!({
        "id": "msg_03",
        "type": "message",
        "role": "assistant",
        "content": [
            {
                "type": "thinking",
                "thinking": "I need to click the submit button."
            },
            {
                "type": "text",
                "text": "I'll click the submit button for you."
            },
            {
                "type": "tool_use",
                "id": "toolu_abc123",
                "name": "Click",
                "input": {
                    "selector": "button[type=submit]"
                }
            }
        ],
        "usage": {
            "input_tokens": 800,
            "output_tokens": 150
        }
    });

    // Text extracted
    let content = extract_assistant_content(&resp).unwrap();
    assert!(content.contains("submit button"));

    // Thinking extracted
    let thinking = extract_thinking_content(&resp).unwrap();
    assert!(thinking.contains("click the submit button"));

    // Tool calls parsed
    let calls = parse_tool_calls(&resp);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, "Click");
    assert_eq!(calls[0].id, "toolu_abc123");

    let args: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
    assert_eq!(args["selector"], "button[type=submit]");
}

// ── Unit: OpenAI reasoning_content response parsing ──

#[test]
fn openai_reasoning_content_response() {
    let resp = json!({
        "choices": [{
            "message": {
                "content": "{\"label\": \"analyze\", \"done\": true, \"steps\": []}",
                "reasoning_content": "Let me think about this step by step.\n\nFirst, I need to look at the HTML structure..."
            }
        }],
        "usage": {
            "prompt_tokens": 1000,
            "completion_tokens": 200,
            "total_tokens": 1200
        }
    });

    let content = extract_assistant_content(&resp).unwrap();
    assert!(content.contains("analyze"));

    let thinking = extract_thinking_content(&resp).unwrap();
    assert!(thinking.contains("step by step"));

    let usage = extract_usage(&resp);
    assert_eq!(usage.prompt_tokens, 1000);
    assert_eq!(usage.completion_tokens, 200);
}

// ── Unit: Standard OpenAI response (no thinking) ──

#[test]
fn openai_standard_response_no_regression() {
    let resp = json!({
        "choices": [{
            "message": {
                "content": "{\"label\": \"crawl\", \"done\": false, \"steps\": [{\"Click\": \"#next\"}]}"
            }
        }],
        "usage": {
            "prompt_tokens": 500,
            "completion_tokens": 50,
            "total_tokens": 550
        }
    });

    let content = extract_assistant_content(&resp).unwrap();
    assert!(content.contains("crawl"));

    // No thinking
    assert!(extract_thinking_content(&resp).is_none());

    let usage = extract_usage(&resp);
    assert_eq!(usage.prompt_tokens, 500);
    assert_eq!(usage.completion_tokens, 50);

    // No tool calls
    assert!(parse_tool_calls(&resp).is_empty());
}

// ── Unit: endpoint detection ──

#[test]
fn endpoint_detection() {
    assert!(is_anthropic_endpoint(
        "https://api.anthropic.com/v1/messages"
    ));
    assert!(!is_anthropic_endpoint(
        "https://openrouter.ai/api/v1/chat/completions"
    ));
    assert!(!is_anthropic_endpoint(
        "https://api.openai.com/v1/chat/completions"
    ));
    assert!(!is_anthropic_endpoint("http://localhost:8080/v1/chat"));
}

// ── Unit: config thinking budget auto-resolution ──

#[test]
fn thinking_budget_auto_resolves_from_reasoning_effort() {
    use spider_agent_types::{
        effective_thinking_budget, effective_thinking_payload, ReasoningEffort,
        RemoteMultimodalConfig,
    };

    // No reasoning configured → no thinking
    let cfg = RemoteMultimodalConfig::default();
    assert!(effective_thinking_budget(&cfg).is_none());
    assert!(effective_thinking_payload(&cfg).is_none());

    // reasoning_effort → auto-translated to thinking budget
    let cfg = RemoteMultimodalConfig::default().with_reasoning_effort(Some(ReasoningEffort::High));
    assert_eq!(effective_thinking_budget(&cfg), Some(16384));

    let pl = effective_thinking_payload(&cfg).unwrap();
    assert_eq!(pl["type"], "enabled");
    assert_eq!(pl["budget_tokens"], 16384);

    // Explicit thinking_budget takes priority
    let cfg = RemoteMultimodalConfig::default()
        .with_reasoning_effort(Some(ReasoningEffort::High))
        .with_thinking_budget(Some(50000));
    assert_eq!(effective_thinking_budget(&cfg), Some(50000));
}

// ── Unit: Anthropic response with multiple thinking blocks ──

#[test]
fn anthropic_multiple_thinking_blocks() {
    let resp = json!({
        "content": [
            {"type": "thinking", "thinking": "First, let me understand the problem."},
            {"type": "thinking", "thinking": "Now I'll formulate my approach."},
            {"type": "text", "text": "{\"label\": \"solution\"}"}
        ]
    });

    let thinking = extract_thinking_content(&resp).unwrap();
    assert!(thinking.contains("understand the problem"));
    assert!(thinking.contains("formulate my approach"));
    // Blocks separated by newline
    assert!(thinking.contains('\n'));
}

// ── Unit: edge cases ──

#[test]
fn empty_content_array_returns_none() {
    let resp = json!({
        "content": []
    });
    assert!(extract_assistant_content(&resp).is_none());
    assert!(extract_thinking_content(&resp).is_none());
}

#[test]
fn anthropic_thinking_only_no_text() {
    let resp = json!({
        "content": [
            {"type": "thinking", "thinking": "Just thinking..."}
        ]
    });
    // No text content
    assert!(extract_assistant_content(&resp).is_none());
    // But thinking is captured
    assert!(extract_thinking_content(&resp).is_some());
}

// ── Live integration test (gated) ──

fn run_live_tests() -> bool {
    matches!(
        std::env::var("RUN_LIVE_TESTS")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Live test: extract data through OpenRouter with a thinking-capable model.
/// Validates the full pipeline: request building → API call → response parsing.
#[tokio::test]
async fn live_thinking_extraction_openrouter() {
    if !run_live_tests() {
        eprintln!("Skipping live thinking test (RUN_LIVE_TESTS not enabled).");
        return;
    }

    dotenvy::dotenv().ok();

    let api_key = std::env::var("OPEN_ROUTER")
        .ok()
        .filter(|v| !v.trim().is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => {
            eprintln!("Skipping: OPEN_ROUTER not set.");
            return;
        }
    };

    use spider_agent_types::{ReasoningEffort, RemoteMultimodalConfig};

    let cfg = RemoteMultimodalConfig::default()
        .with_reasoning_effort(Some(ReasoningEffort::Low))
        .with_extraction(true)
        .with_extraction_prompt("Extract the page title and main heading");

    let engine = spider_agent::automation::RemoteMultimodalEngine::new(
        "https://openrouter.ai/api/v1/chat/completions",
        "anthropic/claude-sonnet-4",
        None,
    )
    .with_api_key(Some(&api_key))
    .with_config(cfg);

    let html = r#"<html><head><title>Test Page</title></head><body><h1>Hello World</h1><p>Some content.</p></body></html>"#;

    let result = engine
        .extract_from_html(html, "https://example.com", Some("Test Page"))
        .await;

    match result {
        Ok(res) => {
            assert!(res.success, "Extraction should succeed");
            assert!(res.extracted.is_some(), "Should have extracted data");
            let extracted = res.extracted.unwrap();
            let s = serde_json::to_string(&extracted).unwrap_or_default();
            // Should contain something about "Test Page" or "Hello World"
            let lower = s.to_lowercase();
            assert!(
                lower.contains("test") || lower.contains("hello"),
                "Extracted data should reference page content: {s}"
            );
            println!("Live test passed. Extracted: {s}");
            println!(
                "Usage: prompt={} completion={} calls={}",
                res.usage.prompt_tokens, res.usage.completion_tokens, res.usage.api_calls
            );
            if let Some(reasoning) = &res.reasoning {
                println!(
                    "Reasoning captured: {}...",
                    &reasoning[..reasoning.len().min(100)]
                );
            }
        }
        Err(e) => {
            // Rate limits or transient errors shouldn't fail the test suite
            eprintln!("Live test error (non-fatal): {e}");
        }
    }
}
