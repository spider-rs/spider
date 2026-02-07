//! Not A Robot challenge test using Claude via OpenRouter.
//!
//! This example tests the multimodal automation capabilities against
//! neal.fun/not-a-robot - a fun interactive challenge where you prove
//! you're not a robot by completing increasingly complex tasks.
//!
//! Run with:
//! ```bash
//! cargo run --example not_a_robot --features "spider/sync spider/chrome spider/chrome_headed spider/agent_chrome spider/agent_skills"
//! ```
//!
//! The API key is read from `.env` file (OPEN_ROUTER=...) or the environment.

extern crate spider;

use spider::features::automation::skills;
use spider::features::automation::RemoteMultimodalConfigs;
use spider::tokio;
use spider::website::Website;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// Load environment variables from a `.env` file if present.
fn load_dotenv() {
    let paths = [".env", "../.env"];
    for path in &paths {
        if let Ok(contents) = fs::read_to_string(path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();
                    // Only set if not already present in environment
                    if std::env::var(key).is_err() {
                        std::env::set_var(key, value);
                    }
                }
            }
            break;
        }
    }
}

#[tokio::main]
async fn main() {
    // Load .env file first, then init logging
    load_dotenv();

    // Enable debug logging to see model responses and actions
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    // Get API key from environment variable (loaded from .env or shell)
    let api_key = std::env::var("OPEN_ROUTER")
        .expect("OPEN_ROUTER environment variable must be set (set in .env or shell)");

    // Create output directory for screenshots
    let output_dir = Path::new("not_a_robot_results");
    if !output_dir.exists() {
        fs::create_dir_all(output_dir).expect("Failed to create output directory");
    }

    // Target URL - the "I'm Not A Robot" challenge
    let url = "https://neal.fun/not-a-robot/";

    // Configure remote multimodal with OpenRouter
    // Default to Claude Opus 4.6 (frontier model, excellent vision + reasoning).
    // Override with MODEL env var for other models.
    let model = std::env::var("MODEL").unwrap_or_else(|_| "anthropic/claude-opus-4.6".to_string());
    let mut mm_config =
        RemoteMultimodalConfigs::new("https://openrouter.ai/api/v1/chat/completions", &model);

    // Set the API key
    mm_config.api_key = Some(api_key);

    // Minimal task - the default system prompt handles challenge types
    mm_config.user_message_extra = Some(
        "Complete ALL levels of this challenge. Track your progress using memory_ops and extracted fields. Report current_level number and level_name in extracted.".to_string(),
    );

    // Configure for interactive challenge completion
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 200; // Allow many rounds for multiple challenge levels
    mm_config.cfg.request_json_object = true;
    mm_config.cfg.post_plan_wait_ms = 1500; // Wait between rounds for UI transitions
    mm_config.cfg.screenshot = true;
    mm_config.cfg.best_effort_json_extract = true;
    mm_config.cfg.max_tokens = 4096; // Enough for detailed responses with coordinates
    mm_config.cfg.temperature = 0.1;

    // Load built-in web challenge skills (image grids, tic-tac-toe, word search, etc.)
    // Skills are matched per-round against page state and injected into the system prompt.
    mm_config.skill_registry = Some(skills::builtin_web_challenges());

    // Create a viewport with 2x device scale factor - larger viewport for better grid/text visibility
    let mut viewport = spider::configuration::Viewport::new(1440, 1080);
    viewport.set_scale_factor(Some(2.0));

    // Create website with Chrome in headed mode
    let mut website: Website = Website::new(url)
        .with_viewport(Some(viewport))
        .with_limit(1)
        .with_request_timeout(None) // Disable request timeout - automation has its own max_rounds
        .with_chrome_intercept(
            spider::features::chrome_common::RequestInterceptConfiguration::new(true),
        )
        .with_wait_for_idle_network(Some(spider::configuration::WaitForIdleNetwork::new(Some(
            Duration::from_secs(30),
        ))))
        .with_remote_multimodal(Some(mm_config))
        .build()
        .unwrap();

    // Subscribe to receive pages
    let mut rx = website.subscribe(16).unwrap();

    // Clone output_dir path for the spawned task
    let output_dir_clone = output_dir.to_path_buf();

    // Spawn task to handle received pages and save screenshots
    let join_handle = tokio::spawn(async move {
        let mut levels_completed: Vec<u32> = Vec::new();
        let mut highest_level_seen = 0u32;

        while let Ok(page) = rx.recv().await {
            println!("\n==================================================");
            println!("=== Challenge Progress ===");
            println!("URL: {}", page.get_url());

            // Check for remote multimodal automation results
            if let Some(ref ai_data) = page.extra_remote_multimodal_data {
                println!("\n=== AI Automation Rounds ===");

                for (i, result) in ai_data.iter().enumerate() {
                    println!("\nRound {}:", i + 1);
                    println!("  Label: {}", result.input);

                    let parsed = &result.content_output;
                    if !parsed.is_null() {
                        if let Some(extracted) = parsed.get("extracted") {
                            let current_level = extracted
                                .get("current_level")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as u32;

                            let level_name = extracted
                                .get("level_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown");

                            let level_completed = extracted
                                .get("level_completed")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            let challenge_complete = extracted
                                .get("challenge_complete")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            println!("  Current Level: {} - {}", current_level, level_name);

                            if current_level > highest_level_seen {
                                if highest_level_seen > 0 {
                                    println!(
                                        "  LEVEL {} COMPLETED! Advanced to Level {}",
                                        highest_level_seen, current_level
                                    );
                                    levels_completed.push(highest_level_seen);
                                }
                                highest_level_seen = current_level;
                            }

                            if level_completed {
                                println!("  Level marked as completed");
                            }

                            if challenge_complete {
                                println!("  CHALLENGE COMPLETE!");
                            }
                        }

                        if let Some(steps) = parsed.get("steps") {
                            println!("  Steps: {}", steps);
                        }
                    }

                    if let Some(ref usage) = result.usage {
                        println!(
                            "  Tokens: {} prompt, {} completion",
                            usage.prompt_tokens, usage.completion_tokens
                        );
                    }

                    if let Some(ref err) = result.error {
                        println!("  Error: {}", err);
                    }
                }
            }

            // Save final screenshot
            if let Some(ref screenshot_b64) = page.screenshot_bytes {
                let screenshot_path = output_dir_clone.join("final_screenshot.png");
                if fs::write(&screenshot_path, screenshot_b64).is_ok() {
                    println!("\nFinal screenshot saved to: {}", screenshot_path.display());
                }
            }

            // Print summary
            println!("\n=== Level Progression Summary ===");
            println!("Highest level reached: {}", highest_level_seen);
            println!("Levels completed: {:?}", levels_completed);

            if let Some(ref usage) = page.remote_multimodal_usage {
                println!("\n=== Total Usage ===");
                let mut total_prompt = 0u32;
                let mut total_completion = 0u32;
                let mut total_calls = 0u32;
                for u in usage.iter() {
                    total_prompt += u.prompt_tokens;
                    total_completion += u.completion_tokens;
                    total_calls += u.llm_calls;
                }
                println!("LLM Calls: {}", total_calls);
                println!("Prompt Tokens: {}", total_prompt);
                println!("Completion Tokens: {}", total_completion);
                println!("Total Tokens: {}", total_prompt + total_completion);
            }
        }

        levels_completed
    });

    // Start the automation
    println!("==========================================");
    println!("   'I'm Not A Robot' Challenge Test");
    println!("==========================================");
    println!("URL: {}", url);
    println!("Model: {}", model);
    println!("Max rounds: 50");
    println!("Output dir: {}", output_dir.display());
    println!("==========================================\n");

    let start = tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    // Unsubscribe to close the channel
    website.unsubscribe();

    // Wait for the spawned task to complete and get results
    match join_handle.await {
        Ok(levels_completed) => {
            println!("\n==========================================");
            println!("   CHALLENGE RESULTS");
            println!("==========================================");
            println!("Total time: {:?}", duration);
            println!("Levels completed: {}", levels_completed.len());
            for level in &levels_completed {
                println!("  Level {} completed", level);
            }
            if levels_completed.is_empty() {
                println!("  No levels were completed");
            }
            println!("==========================================");
        }
        Err(e) => {
            println!("\nError in handler task: {}", e);
        }
    }
}
