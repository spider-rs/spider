//! Not A Robot challenge test using Claude via OpenRouter.
//!
//! This example tests the multimodal automation capabilities against
//! neal.fun/not-a-robot - a fun interactive challenge where you prove
//! you're not a robot by completing increasingly complex tasks.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example not_a_robot --features "spider/sync spider/chrome spider/chrome_headed spider/agent_chrome"
//! ```

extern crate spider;

use spider::features::automation::RemoteMultimodalConfigs;
use spider::tokio;
use spider::website::Website;
use std::fs;
use std::path::Path;
use std::time::Duration;

#[tokio::main]
async fn main() {
    // Enable debug logging to see model responses and actions
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    // Get API key from environment variable
    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    // Create output directory for screenshots
    let output_dir = Path::new("not_a_robot_results");
    if !output_dir.exists() {
        fs::create_dir_all(output_dir).expect("Failed to create output directory");
    }

    // Target URL - the "I'm Not A Robot" challenge
    let url = "https://neal.fun/not-a-robot/";

    // Configure remote multimodal with OpenRouter - using Claude for better vision
    let mut mm_config = RemoteMultimodalConfigs::new(
        "https://openrouter.ai/api/v1/chat/completions",
        "anthropic/claude-opus-4.5", // Best vision accuracy for image grids
    );

    // Set the API key
    mm_config.api_key = Some(api_key);

    // Minimal task - system prompt handles challenge types
    mm_config.user_message_extra = Some(
        "Complete ALL levels of this challenge.".to_string(),
    );

    // Configure for interactive challenge completion
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 30; // Allow many rounds for multiple challenge levels
    mm_config.cfg.request_json_object = true;
    mm_config.cfg.post_plan_wait_ms = 1000; // Brief wait between rounds
    mm_config.cfg.screenshot = true; // Capture screenshots
    mm_config.cfg.best_effort_json_extract = true;

    // Create a viewport with higher device scale factor for better screenshot quality
    let mut viewport = spider::configuration::Viewport::new(1280, 960);
    viewport.set_scale_factor(Some(3.0)); // 3x resolution for better text detail

    // Create website with Chrome in headed mode
    let mut website: Website = Website::new(url)
        .with_viewport(Some(viewport))
        .with_limit(1)
        .with_chrome_intercept(
            spider::features::chrome_common::RequestInterceptConfiguration::new(true),
        )
        .with_wait_for_idle_network(Some(spider::configuration::WaitForIdleNetwork::new(Some(
            Duration::from_secs(5),
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

                    // Parse the extracted data to track level progression
                    // content_output is serde_json::Value when serde feature is enabled
                    let parsed = &result.content_output;
                    if !parsed.is_null() {
                        if let Some(extracted) = parsed.get("extracted") {
                            let current_level = extracted
                                .get("current_level")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32;

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

                            // Detect level progression
                            if current_level > highest_level_seen {
                                if highest_level_seen > 0 {
                                    println!(
                                        "  ✓ LEVEL {} COMPLETED! Advanced to Level {}",
                                        highest_level_seen, current_level
                                    );
                                    levels_completed.push(highest_level_seen);
                                }
                                highest_level_seen = current_level;
                            }

                            if level_completed {
                                println!("  ✓ Level marked as completed");
                            }

                            if challenge_complete {
                                println!("  🎉 CHALLENGE COMPLETE!");
                            }
                        }

                        // Show the steps taken
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
            if let Some(ref ai_data) = page.extra_remote_multimodal_data {
                // Get the last result which should have the final screenshot
                if let Some(last_result) = ai_data.last() {
                    if let Some(ref _usage) = last_result.usage {
                        // The screenshot is stored in the page's automation result
                        // We'll save it based on the highest level reached
                        let screenshot_path = output_dir_clone
                            .join(format!("final_level_{}.png", highest_level_seen));
                        println!(
                            "\nScreenshot would be saved to: {}",
                            screenshot_path.display()
                        );
                    }
                }
            }

            // Print summary
            println!("\n=== Level Progression Summary ===");
            println!("Highest level reached: {}", highest_level_seen);
            println!("Levels completed: {:?}", levels_completed);

            // Check for usage statistics
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

        // Return the levels completed for verification
        levels_completed
    });

    // Start the automation
    println!("==========================================");
    println!("   'I'm Not A Robot' Challenge Test");
    println!("==========================================");
    println!("URL: {}", url);
    println!("Model: anthropic/claude-sonnet-4");
    println!("Max rounds: 30");
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
                println!("  ✓ Level {} completed", level);
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
