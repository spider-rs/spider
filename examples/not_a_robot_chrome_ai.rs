//! Not A Robot challenge using Chrome's built-in AI (Gemini Nano).
//!
//! This example runs the "I'm Not A Robot" challenge using Chrome's
//! built-in LanguageModel API (Gemini Nano) for inference — no external
//! API key required.
//!
//! ## Prerequisites
//!
//! **Chrome Canary or Dev channel is required.** The LanguageModel API
//! (Prompt API) is not available in Chrome stable — it requires the
//! "Optimization Guide On Device Model" component which is only present
//! in Canary/Dev.
//!
//! ### 1. Install Chrome Canary
//!
//! Download from <https://www.google.com/chrome/canary/>
//!
//! ### 2. Enable flags in Chrome Canary
//!
//! 1. Open `chrome://flags/#optimization-guide-on-device-model` → **Enabled BypassPerfRequirement**
//! 2. Open `chrome://flags/#prompt-api-for-gemini-nano` → **Enabled**
//! 3. Restart Chrome Canary
//!
//! ### 3. Verify the model is downloaded
//!
//! 1. Open `chrome://components` → find "Optimization Guide On Device Model"
//!    - If version is `0.0.0.0`, click "Check for update" and wait
//!    - The model is ~1-2 GB and may take several minutes to download
//! 2. Open `chrome://on-device-internals` → "Model Status" tab
//!    - Foundational model state should be **Ready**
//! 3. Test in DevTools console: `await LanguageModel.availability()` → should return `"available"`
//!
//! **Hardware requirements:** 22 GB free storage, 4 GB+ VRAM or 16 GB RAM with 4+ CPU cores.
//!
//! ## Run
//!
//! Option A — Connect to Chrome Canary with remote debugging (recommended):
//! ```bash
//! # 1. Launch Chrome Canary with AI flags:
//! /Applications/Google\ Chrome\ Canary.app/Contents/MacOS/Google\ Chrome\ Canary \
//!   --remote-debugging-port=9222 \
//!   --enable-features=OptimizationGuideOnDeviceModel:BypassPerfRequirement/true,PromptAPIForGeminiNano,PromptAPIForGeminiNanoMultimodalInput \
//!   --no-first-run --user-data-dir=$HOME/.chrome-ai-profile &
//!
//! # 2. Get the WebSocket URL:
//! curl -s http://localhost:9222/json/version | jq -r .webSocketDebuggerUrl
//!
//! # 3. Run the example:
//! CHROME_URL=ws://localhost:9222/devtools/browser/<ID> \
//!   cargo run --example not_a_robot_chrome_ai --features "spider/sync spider/chrome spider/chrome_headed spider/agent_chrome spider/agent_skills"
//! ```
//!
//! Option B — Let spider launch Chrome (adds AI flags automatically when `use_chrome_ai` is set):
//! ```bash
//! cargo run --example not_a_robot_chrome_ai --features "spider/sync spider/chrome spider/chrome_headed spider/agent_chrome spider/agent_skills"
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

    // Create output directory for screenshots
    let output_dir = Path::new("not_a_robot_chrome_ai_results");
    if !output_dir.exists() {
        fs::create_dir_all(output_dir).expect("Failed to create output directory");
    }

    // Target URL
    let url = "https://neal.fun/not-a-robot/";

    // Configure with Chrome AI — no API URL or key needed.
    // We pass empty strings since Chrome AI bypasses the HTTP path entirely.
    let mut mm_config = RemoteMultimodalConfigs::new("", "chrome-ai");
    mm_config.use_chrome_ai = true;

    // Task instructions — kept concise for the smaller model
    mm_config.user_message_extra = Some(
        "Complete ALL levels. Track progress in memory_ops. Report current_level and level_name in extracted. Max 5 attempts per level; change strategy after 5 fails."
            .to_string(),
    );

    // Chrome AI context budget — Gemini Nano has limited context
    mm_config.chrome_ai_max_user_chars = 5000;

    // Configure for interactive challenge completion
    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.html_max_bytes = 4000; // Chrome AI has less context — don't waste on huge HTML
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 200;
    mm_config.cfg.request_json_object = true;
    mm_config.cfg.post_plan_wait_ms = 1500;
    mm_config.cfg.screenshot = true;
    mm_config.cfg.best_effort_json_extract = true;
    mm_config.cfg.max_tokens = 4096;
    mm_config.cfg.temperature = 0.1;

    // Create a viewport with 2x scale factor
    let mut viewport = spider::configuration::Viewport::new(1440, 1080);
    viewport.set_scale_factor(Some(2.0));

    // Connect to an existing Chrome via CHROME_URL env var, or launch a new one.
    // When using Chrome AI, connecting to a pre-launched Chrome with the AI flags
    // is the most reliable approach.
    let chrome_url = std::env::var("CHROME_URL").ok();
    if let Some(ref u) = chrome_url {
        println!("Connecting to Chrome at: {}", u);
    }

    let mut website: Website = Website::new(url)
        .with_chrome_connection(chrome_url)
        .with_viewport(Some(viewport))
        .with_limit(1)
        .with_request_timeout(None)
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

    let output_dir_clone = output_dir.to_path_buf();

    let join_handle = tokio::spawn(async move {
        let mut levels_completed: Vec<u32> = Vec::new();
        let mut highest_level_seen = 0u32;

        while let Ok(page) = rx.recv().await {
            println!("\n==================================================");
            println!("=== Challenge Progress (Chrome AI) ===");
            println!("URL: {}", page.get_url());

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
                        }
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

            println!("\n=== Level Progression Summary ===");
            println!("Highest level reached: {}", highest_level_seen);
            println!("Levels completed: {:?}", levels_completed);
        }

        levels_completed
    });

    // Start the automation
    println!("==========================================");
    println!("  'Not A Robot' Challenge (Chrome AI)");
    println!("==========================================");
    println!("URL: {}", url);
    println!("Model: Chrome built-in LanguageModel (Gemini Nano)");
    println!("Output dir: {}", output_dir.display());
    println!("==========================================\n");

    let start = tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    website.unsubscribe();

    match join_handle.await {
        Ok(levels_completed) => {
            println!("\n==========================================");
            println!("   CHALLENGE RESULTS (Chrome AI)");
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
