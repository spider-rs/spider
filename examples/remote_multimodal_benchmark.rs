//! Remote multimodal model benchmark example.
//!
//! This example benchmarks multiple LLM models via OpenRouter for extraction tasks,
//! comparing speed, token usage, and cost across top-tier and budget models.
//!
//! Run with:
//! ```bash
//! OPEN_ROUTER=your-api-key cargo run --example remote_multimodal_benchmark --features "spider/sync spider/chrome"
//! ```

// ==================================================================================================================================
// BENCHMARK RESULTS
// ==================================================================================================================================

// Model                  Tier        Time(s)     Prompt       Comp      Total    Cost($)  Quality   Status
// ----------------------------------------------------------------------------------------------------------------------------------
// GPT-4o                 top-tier      15.04       2681        294       2975   0.009643     100%        ✓
// Claude 3.5 Sonnet      top-tier      12.86       2439        284       2723   0.011577     100%        ✓
// Gemini 2.0 Flash       top-tier       6.21       3606        427       4033   0.000531     100%        ✓
// Qwen2-VL 72B           top-tier      11.16       2578        312       2890   0.001156     100%        ✓
// Gemini 2.0 Flash Lite  budget         5.95       3606        419       4025   0.000396     100%        ✓
// Gemini 1.5 Flash       budget         2.53          0          0          0   0.000000       0%        ✗
//     └─ Error: Page fetched (9308B) but no AI data
// Qwen2-VL 7B            budget        22.28       2907        315       3222   0.000322     100%        ✓
// Pixtral 12B            budget         7.08       3838        511       4349   0.000435     100%        ✓
// ----------------------------------------------------------------------------------------------------------------------------------

// SUMMARY (successful runs only):
//   🏆 Fastest: Gemini 2.0 Flash Lite (5.95s)
//   💰 Cheapest: Qwen2-VL 7B ($0.000322)
//   ⚡ Highest throughput: Pixtral 12B (72.2 tokens/s)
//   🎯 Best value: Qwen2-VL 7B (100% quality, $0.000322)
//   🌟 Highest quality: Pixtral 12B (100%)

// TIER COMPARISON:
//   Top-tier avg: 11.31s, $0.005727, 100% quality
//   Budget avg:   11.77s, $0.000384, 100% quality
//   Budget is 14.9x cheaper on average

// ========================================================================================================================

extern crate spider;

use spider::features::automation::RemoteMultimodalConfigs;
use spider::tokio;
use spider::website::Website;
use std::time::{Duration, Instant};

/// Model configuration for benchmarking
struct ModelConfig {
    /// Display name for the model
    name: &'static str,
    /// OpenRouter model identifier
    model_id: &'static str,
    /// Tier (top-tier or budget)
    tier: &'static str,
    /// Approximate cost per 1M input tokens (USD)
    input_cost_per_m: f64,
    /// Approximate cost per 1M output tokens (USD)
    output_cost_per_m: f64,
}

/// Benchmark result for a single model
struct BenchmarkResult {
    model_name: String,
    tier: String,
    duration: Duration,
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
    estimated_cost: f64,
    success: bool,
    error: Option<String>,
    /// Quality score (0-100) based on expected fields present
    quality_score: u8,
    /// Fields found in the extraction
    fields_found: Vec<String>,
}

/// Expected fields for the book extraction test
const EXPECTED_FIELDS: &[&str] = &["title", "price", "availability", "description", "upc"];

/// Validate the extraction output and return (quality_score, fields_found)
fn validate_extraction(content_str: &str) -> (u8, Vec<String>) {
    let mut fields_found = Vec::new();
    let content_lower = content_str.to_lowercase();

    for field in EXPECTED_FIELDS {
        if content_lower.contains(field) {
            fields_found.push(field.to_string());
        }
    }

    // Check for specific expected values
    let has_title = content_lower.contains("light in the attic");
    let has_price = content_lower.contains("51.77");
    let has_upc = content_lower.contains("a897fe39b1053632");

    let mut bonus = 0u8;
    if has_title {
        bonus += 10;
    }
    if has_price {
        bonus += 10;
    }
    if has_upc {
        bonus += 10;
    }

    let base_score = (fields_found.len() as f64 / EXPECTED_FIELDS.len() as f64 * 70.0) as u8;
    let quality_score = (base_score + bonus).min(100);

    (quality_score, fields_found)
}

/// Get the list of models to benchmark (verified OpenRouter multimodal models)
fn get_models() -> Vec<ModelConfig> {
    vec![
        // Top-tier models (highest quality)
        ModelConfig {
            name: "GPT-4o",
            model_id: "openai/gpt-4o",
            tier: "top-tier",
            input_cost_per_m: 2.50,
            output_cost_per_m: 10.00,
        },
        ModelConfig {
            name: "Claude 3.5 Sonnet",
            model_id: "anthropic/claude-3.5-sonnet",
            tier: "top-tier",
            input_cost_per_m: 3.00,
            output_cost_per_m: 15.00,
        },
        ModelConfig {
            name: "Gemini 2.0 Flash",
            model_id: "google/gemini-2.0-flash-001",
            tier: "top-tier",
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.40,
        },
        ModelConfig {
            name: "Qwen2-VL 72B",
            model_id: "qwen/qwen-2-vl-72b-instruct",
            tier: "top-tier",
            input_cost_per_m: 0.40,
            output_cost_per_m: 0.40,
        },
        // Budget models (fast and cheap)
        ModelConfig {
            name: "Gemini 2.0 Flash Lite",
            model_id: "google/gemini-2.0-flash-lite-001",
            tier: "budget",
            input_cost_per_m: 0.075,
            output_cost_per_m: 0.30,
        },
        ModelConfig {
            name: "Qwen2-VL 7B",
            model_id: "qwen/qwen-2-vl-7b-instruct",
            tier: "budget",
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.10,
        },
        ModelConfig {
            name: "Pixtral 12B",
            model_id: "mistralai/pixtral-12b",
            tier: "budget",
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.10,
        },
    ]
}

async fn benchmark_model(api_key: &str, url: &str, model: &ModelConfig) -> BenchmarkResult {
    println!("\n[{}] Testing {}...", model.tier, model.name);

    let mut mm_config = RemoteMultimodalConfigs::new(
        "https://openrouter.ai/api/v1/chat/completions",
        model.model_id,
    );

    mm_config.api_key = Some(api_key.to_string());
    mm_config.system_prompt = Some(
        "Extract the following data from the webpage content and return as valid JSON: \
         Extract book details including title, price, availability, description, and any other relevant information."
            .to_string(),
    );

    mm_config.cfg.extra_ai_data = true;
    mm_config.cfg.include_html = true;
    mm_config.cfg.include_title = true;
    mm_config.cfg.include_url = true;
    mm_config.cfg.max_rounds = 1;
    mm_config.cfg.request_json_object = true;

    let mut website: Website = Website::new(url)
        .with_limit(1)
        .with_remote_multimodal(Some(mm_config))
        .build()
        .unwrap();

    // Use subscribe + crawl pattern (like the working example)
    let mut rx = website.subscribe(16).unwrap();

    // Collect results from the subscription
    let results_handle = tokio::spawn(async move {
        let mut prompt_tokens = 0u32;
        let mut completion_tokens = 0u32;
        let mut total_tokens = 0u32;
        let mut success = false;
        let mut error: Option<String> = None;
        let mut content_str: Option<String> = None;

        while let Ok(page) = rx.recv().await {
            if let Some(ref ai_data) = page.extra_remote_multimodal_data {
                for result in ai_data {
                    if let Some(ref usage) = result.usage {
                        prompt_tokens = usage.prompt_tokens;
                        completion_tokens = usage.completion_tokens;
                        total_tokens = usage.total_tokens;
                    }
                    if let Some(ref err) = result.error {
                        error = Some(err.clone());
                    } else if !result.content_output.is_null() {
                        success = true;
                        content_str = Some(result.content_output.to_string());
                    }
                }
            } else {
                let html_len = page.get_html().len();
                if html_len > 0 && error.is_none() {
                    error = Some(format!("Page fetched ({}B) but no AI data", html_len));
                }
            }
        }

        (
            prompt_tokens,
            completion_tokens,
            total_tokens,
            success,
            error,
            content_str,
        )
    });

    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    // Close the channel
    website.unsubscribe();

    // Get results from the spawned task
    let (prompt_tokens, completion_tokens, total_tokens, success, error, content_output) =
        results_handle
            .await
            .unwrap_or((0, 0, 0, false, Some("Task failed".to_string()), None));

    // Validate extraction quality
    let (quality_score, fields_found) = if let Some(ref content) = content_output {
        validate_extraction(content)
    } else {
        (0, vec![])
    };

    // Calculate estimated cost
    let estimated_cost = (prompt_tokens as f64 * model.input_cost_per_m / 1_000_000.0)
        + (completion_tokens as f64 * model.output_cost_per_m / 1_000_000.0);

    let status_icon = if success && quality_score >= 70 {
        "✓"
    } else if success {
        "~"
    } else {
        "✗"
    };
    println!(
        "  {} Completed in {:.2}s | Tokens: {} | Quality: {}% | Est. cost: ${:.6}",
        status_icon,
        duration.as_secs_f64(),
        total_tokens,
        quality_score,
        estimated_cost
    );

    BenchmarkResult {
        model_name: model.name.to_string(),
        tier: model.tier.to_string(),
        duration,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        estimated_cost,
        success: success && quality_score >= 70,
        error,
        quality_score,
        fields_found,
    }
}

fn print_results_table(results: &[BenchmarkResult]) {
    println!("\n{}", "=".repeat(130));
    println!("BENCHMARK RESULTS");
    println!("{}", "=".repeat(130));

    println!(
        "\n{:<22} {:<10} {:>8} {:>10} {:>10} {:>10} {:>10} {:>8} {:>8}",
        "Model", "Tier", "Time(s)", "Prompt", "Comp", "Total", "Cost($)", "Quality", "Status"
    );
    println!("{}", "-".repeat(130));

    for result in results {
        let status = if result.success { "✓" } else { "✗" };
        let quality_str = format!("{}%", result.quality_score);
        println!(
            "{:<22} {:<10} {:>8.2} {:>10} {:>10} {:>10} {:>10.6} {:>8} {:>8}",
            result.model_name,
            result.tier,
            result.duration.as_secs_f64(),
            result.prompt_tokens,
            result.completion_tokens,
            result.total_tokens,
            result.estimated_cost,
            quality_str,
            status
        );
        if let Some(ref err) = result.error {
            println!("    └─ Error: {}", err);
        }
        if result.quality_score < 100 && !result.fields_found.is_empty() {
            println!("    └─ Fields found: {}", result.fields_found.join(", "));
        }
    }

    println!("{}", "-".repeat(130));

    // Summary statistics
    let successful: Vec<_> = results.iter().filter(|r| r.success).collect();
    if !successful.is_empty() {
        println!("\nSUMMARY (successful runs only):");

        // Fastest model
        if let Some(fastest) = successful.iter().min_by_key(|r| r.duration) {
            println!(
                "  🏆 Fastest: {} ({:.2}s)",
                fastest.model_name,
                fastest.duration.as_secs_f64()
            );
        }

        // Cheapest model
        if let Some(cheapest) = successful
            .iter()
            .filter(|r| r.estimated_cost > 0.0)
            .min_by(|a, b| a.estimated_cost.partial_cmp(&b.estimated_cost).unwrap())
        {
            println!(
                "  💰 Cheapest: {} (${:.6})",
                cheapest.model_name, cheapest.estimated_cost
            );
        }

        // Most efficient (tokens per second)
        if let Some(efficient) = successful
            .iter()
            .filter(|r| r.duration.as_secs_f64() > 0.0)
            .max_by(|a, b| {
                let a_tps = a.completion_tokens as f64 / a.duration.as_secs_f64();
                let b_tps = b.completion_tokens as f64 / b.duration.as_secs_f64();
                a_tps.partial_cmp(&b_tps).unwrap()
            })
        {
            let tps = efficient.completion_tokens as f64 / efficient.duration.as_secs_f64();
            println!(
                "  ⚡ Highest throughput: {} ({:.1} tokens/s)",
                efficient.model_name, tps
            );
        }

        // Best value (quality per dollar)
        if let Some(best_value) =
            successful
                .iter()
                .filter(|r| r.estimated_cost > 0.0)
                .max_by(|a, b| {
                    let a_val = a.quality_score as f64 / a.estimated_cost;
                    let b_val = b.quality_score as f64 / b.estimated_cost;
                    a_val.partial_cmp(&b_val).unwrap()
                })
        {
            println!(
                "  🎯 Best value: {} ({}% quality, ${:.6})",
                best_value.model_name, best_value.quality_score, best_value.estimated_cost
            );
        }

        // Highest quality
        if let Some(best_quality) = successful.iter().max_by_key(|r| r.quality_score) {
            println!(
                "  🌟 Highest quality: {} ({}%)",
                best_quality.model_name, best_quality.quality_score
            );
        }
    }

    // Top-tier vs Budget comparison
    let top_tier: Vec<_> = successful.iter().filter(|r| r.tier == "top-tier").collect();
    let budget: Vec<_> = successful.iter().filter(|r| r.tier == "budget").collect();

    if !top_tier.is_empty() && !budget.is_empty() {
        let avg_top_time: f64 = top_tier
            .iter()
            .map(|r| r.duration.as_secs_f64())
            .sum::<f64>()
            / top_tier.len() as f64;
        let avg_budget_time: f64 =
            budget.iter().map(|r| r.duration.as_secs_f64()).sum::<f64>() / budget.len() as f64;
        let avg_top_cost: f64 =
            top_tier.iter().map(|r| r.estimated_cost).sum::<f64>() / top_tier.len() as f64;
        let avg_budget_cost: f64 =
            budget.iter().map(|r| r.estimated_cost).sum::<f64>() / budget.len() as f64;
        let avg_top_quality: f64 =
            top_tier.iter().map(|r| r.quality_score as f64).sum::<f64>() / top_tier.len() as f64;
        let avg_budget_quality: f64 =
            budget.iter().map(|r| r.quality_score as f64).sum::<f64>() / budget.len() as f64;

        println!("\nTIER COMPARISON:");
        println!(
            "  Top-tier avg: {:.2}s, ${:.6}, {:.0}% quality",
            avg_top_time, avg_top_cost, avg_top_quality
        );
        println!(
            "  Budget avg:   {:.2}s, ${:.6}, {:.0}% quality",
            avg_budget_time, avg_budget_cost, avg_budget_quality
        );
        if avg_budget_cost > 0.0 {
            println!(
                "  Budget is {:.1}x cheaper on average",
                avg_top_cost / avg_budget_cost
            );
        }
    }

    println!("\n{}", "=".repeat(120));
}

#[tokio::main]
async fn main() {
    let api_key =
        std::env::var("OPEN_ROUTER").expect("OPEN_ROUTER environment variable must be set");

    let url = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";
    let models = get_models();

    println!("Remote Multimodal Model Benchmark");
    println!("==================================");
    println!("URL: {}", url);
    println!("Models to test: {}", models.len());
    println!("\nStarting benchmark...");

    let mut results = Vec::new();

    for model in &models {
        let result = benchmark_model(&api_key, url, model).await;
        results.push(result);

        // Small delay between requests to avoid rate limiting
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    print_results_table(&results);
}
