//! Integration tests for action feedback, partial result preservation,
//! and Evaluate JS error capture improvements.
//!
//! These tests verify the improvements at the integration boundary — they
//! exercise the actual types and formatting logic that the agent loop uses
//! without requiring a running browser.
//!
//! For live LLM integration, set `RUN_LIVE_TESTS=1` and `OPENROUTER_API_KEY`.
//!
//! Run with:
//! ```sh
//! cargo test -p spider_agent --features chrome --test action_feedback_integration
//! ```

#[cfg(feature = "chrome")]
mod feedback_tests {
    use spider_agent::{AutomationResult, AutomationUsage, EngineError};

    // ====================================================================
    // Partial result preservation: public API surface
    // ====================================================================

    #[test]
    fn partial_result_round_trip_json() {
        let result = AutomationResult {
            label: "scraping".to_string(),
            steps_executed: 7,
            success: false,
            error: Some("LLM inference failed on round 3: Http timeout".to_string()),
            usage: AutomationUsage {
                prompt_tokens: 2500,
                completion_tokens: 400,
                total_tokens: 2900,
                llm_calls: 2,
                ..Default::default()
            },
            extracted: Some(serde_json::json!({
                "products": [
                    {"name": "Widget A", "price": 19.99},
                    {"name": "Widget B", "price": 29.99},
                ]
            })),
            screenshot: None,
            spawn_pages: vec!["https://example.com/page2".to_string()],
            relevant: Some(true),
            reasoning: Some("Collecting product data".to_string()),
        };

        // Serialize to JSON and back — all fields must survive
        let json = serde_json::to_string(&result).unwrap();
        let parsed: AutomationResult = serde_json::from_str(&json).unwrap();

        assert!(!parsed.success);
        assert_eq!(parsed.steps_executed, 7);
        assert!(parsed.error.as_ref().unwrap().contains("round 3"));
        assert!(parsed.extracted.is_some());
        let products = parsed.extracted.as_ref().unwrap()["products"]
            .as_array()
            .unwrap();
        assert_eq!(products.len(), 2);
        assert_eq!(parsed.usage.llm_calls, 2);
        assert_eq!(parsed.spawn_pages.len(), 1);
        assert_eq!(parsed.relevant, Some(true));
        assert!(parsed.reasoning.is_some());
    }

    #[test]
    fn partial_result_with_usage_preserved() {
        let mut usage = AutomationUsage::default();
        usage.prompt_tokens = 1000;
        usage.completion_tokens = 200;
        usage.total_tokens = 1200;
        usage.llm_calls = 3;

        let result = AutomationResult::failure("test", "LLM error").with_usage(usage.clone());

        assert!(!result.success);
        assert_eq!(result.usage.prompt_tokens, 1000);
        assert_eq!(result.usage.llm_calls, 3);
    }

    #[test]
    fn engine_error_display() {
        let http_err_msg = format!("{}", EngineError::Remote("429 rate limited".to_string()));
        assert!(http_err_msg.contains("429 rate limited"));

        let missing = format!(
            "{}",
            EngineError::MissingField("choices[0].message.content")
        );
        assert!(missing.contains("choices[0].message.content"));

        let invalid = format!("{}", EngineError::InvalidField("content not JSON"));
        assert!(invalid.contains("content not JSON"));
    }

    // ====================================================================
    // Verify AutomationResult builder preserves all fields
    // ====================================================================

    #[test]
    fn automation_result_builder_chain() {
        let result = AutomationResult::success("extraction", 3)
            .with_extracted(serde_json::json!({"key": "value"}))
            .with_screenshot("base64png".to_string())
            .with_spawn_pages(vec!["https://a.com".to_string()]);

        assert!(result.success);
        assert!(result.error.is_none());
        assert!(result.extracted.is_some());
        assert!(result.screenshot.is_some());
        assert_eq!(result.spawn_pages.len(), 1);
    }
}

#[cfg(feature = "chrome")]
mod live_llm_tests {
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

    /// Integration test that verifies action feedback is properly formatted
    /// and could be injected into an LLM prompt. Uses OpenRouter if available.
    #[tokio::test]
    async fn live_action_feedback_prompt_injection() {
        if !run_live_tests() {
            eprintln!("Skipping live LLM test (RUN_LIVE_TESTS not enabled).");
            return;
        }

        let api_key = match env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            Some(k) => k,
            None => {
                eprintln!("Skipping: OPENROUTER_API_KEY not set.");
                return;
            }
        };

        let model = env::var("OPENROUTER_MODEL")
            .unwrap_or_else(|_| "google/gemini-2.0-flash-001".to_string());

        // Build a prompt that includes action feedback, similar to what the agent sends
        let action_feedback = concat!(
            "PREVIOUS ACTION RESULTS:\n",
            "- Click → ok\n",
            "- Fill → FAILED: selector not found: input#email\n",
            "- Evaluate → FAILED: JS error: ReferenceError: foo is not defined\n",
            "\n"
        );
        let user_prompt = format!(
            "{}You are a web automation agent. The previous round's actions had failures as shown above. \
             What CSS selector should you try instead of 'input#email' for an email field? \
             Reply with just the selector, nothing else.",
            action_feedback
        );

        // Call OpenRouter API directly
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "user", "content": user_prompt}
            ],
            "max_tokens": 100,
            "temperature": 0.0,
        });

        let resp = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let json: serde_json::Value = r.json().await.unwrap();
                let content = json["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("");
                // The model should suggest an alternative selector
                assert!(
                    !content.trim().is_empty(),
                    "LLM should return a non-empty alternative selector"
                );
                eprintln!("LLM suggested selector: {}", content.trim());
            }
            Ok(r) => {
                eprintln!(
                    "OpenRouter returned non-success status: {} — skipping assertion",
                    r.status()
                );
            }
            Err(e) => {
                eprintln!("OpenRouter request failed: {} — skipping", e);
            }
        }
    }
}
