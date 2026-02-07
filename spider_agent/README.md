# Spider Agent

A concurrent-safe multimodal agent for web automation and research.

## Features

- **Concurrent-safe**: Designed to be wrapped in `Arc` for multi-task access
- **Feature-gated**: Only include dependencies you need
- **Multiple LLM providers**: OpenAI, OpenAI-compatible APIs
- **Multiple search providers**: Serper, Brave, Bing, Tavily
- **HTML extraction**: Clean and extract structured data from web pages
- **Research synthesis**: Combine search + extraction + LLM synthesis

### Advanced Automation Features

- **Tool Calling Schema**: OpenAI-compatible function calling for reliable action parsing
- **HTML Diff Mode**: 50-70% token reduction by sending only page changes after first round
- **Planning Mode**: Multi-step planning reduces LLM round-trips
- **Parallel Synthesis**: Analyze N pages in a single LLM call
- **Confidence Tracking**: Smarter retry decisions based on LLM confidence scores
- **Self-Healing Selectors**: Auto-repair failed selectors with LLM diagnosis
- **Schema Generation**: Auto-generate JSON schemas from example outputs
- **Concurrent Chains**: Execute independent actions in parallel with dependency graphs

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
spider_agent = { version = "0.1", features = ["openai", "search_serper"] }
```

## Quick Start

```rust
use spider_agent::{Agent, AgentConfig};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Arc::new(Agent::builder()
        .with_openai("sk-...", "gpt-4o-mini")
        .with_search_serper("serper-key")
        .build()?);

    // Search
    let results = agent.search("rust web frameworks").await?;
    println!("Found {} results", results.len());

    // Extract from first result
    let html = agent.fetch(&results.results[0].url).await?.html;
    let data = agent.extract(&html, "Extract framework name and features").await?;
    println!("{}", serde_json::to_string_pretty(&data)?);

    Ok(())
}
```

## Concurrent Execution

```rust
use spider_agent::Agent;
use std::sync::Arc;

let agent = Arc::new(Agent::builder()
    .with_openai("sk-...", "gpt-4o")
    .with_search_serper("serper-key")
    .with_max_concurrent_llm_calls(10)
    .build()?);

// Execute multiple searches concurrently
let queries = vec!["rust async", "rust web frameworks", "rust databases"];
let mut handles = Vec::new();

for query in queries {
    let agent = agent.clone();
    let query = query.to_string();
    handles.push(tokio::spawn(async move {
        agent.search(&query).await
    }));
}

// Collect results
for handle in handles {
    let result = handle.await??;
    println!("Found {} results", result.results.len());
}
```

## Research with Synthesis

```rust
use spider_agent::{Agent, ResearchOptions};

let agent = Agent::builder()
    .with_openai("sk-...", "gpt-4o")
    .with_search_serper("serper-key")
    .build()?;

let research = agent.research(
    "How do Tokio and async-std compare?",
    ResearchOptions::new()
        .with_max_pages(5)
        .with_synthesize(true),
).await?;

println!("Summary: {}", research.summary.unwrap());
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `openai` | OpenAI/OpenAI-compatible LLM provider |
| `chrome` | Browser automation via chromiumoxide |
| `search` | Base search functionality |
| `search_serper` | Serper.dev search provider |
| `search_brave` | Brave Search provider |
| `search_bing` | Bing Search provider |
| `search_tavily` | Tavily AI Search provider |
| `full` | All features |

## Examples

```bash
# Basic search
SERPER_API_KEY=xxx cargo run --example basic_search --features search_serper

# Extract data
OPENAI_API_KEY=xxx cargo run --example extract --features openai

# Research
OPENAI_API_KEY=xxx SERPER_API_KEY=xxx cargo run --example research --features "openai search_serper"

# Concurrent execution
OPENAI_API_KEY=xxx SERPER_API_KEY=xxx cargo run --example concurrent --features "openai search_serper"
```

## Verification

From the repository root:

```bash
cargo check --workspace
cargo test -p spider_agent
cargo test -p spider_agent --features "openai search_serper"
RUN_LIVE_TESTS=1 cargo test -p spider_agent --features "openai search_serper" --test live_env_smoke -- --nocapture
```

## API Reference

### Agent

The main struct for all agent operations:

- `search(query)` - Search the web
- `search_with_options(query, options)` - Search with custom options
- `fetch(url)` - Fetch a URL
- `extract(html, prompt)` - Extract data from HTML using LLM
- `extract_structured(html, schema)` - Extract data matching a JSON schema
- `research(topic, options)` - Research a topic with synthesis
- `prompt(messages)` - Send a prompt to the LLM
- `memory_get/set/clear()` - Session memory operations

### AgentBuilder

Configure and build agents:

```rust
Agent::builder()
    .with_config(config)
    .with_system_prompt("You are a helpful assistant")
    .with_max_concurrent_llm_calls(10)
    .with_openai(api_key, model)
    .with_spider_cloud("spider-cloud-api-key")
    .with_search_serper(api_key)
    .build()
```

### Spider Cloud Tool Inheritance

You can register Spider Cloud routes as custom tools directly from the builder.

```rust
use spider_agent::Agent;

let agent = Agent::builder()
    .with_spider_cloud("spider-cloud-api-key")
    .build()?;

// Available tools:
// - spider_cloud_crawl
// - spider_cloud_scrape
// - spider_cloud_search
// - spider_cloud_links
// - spider_cloud_transform
// - spider_cloud_unblocker
```

For full control (custom API URL, toggles, AI subscription gating), use `SpiderCloudToolConfig`:

```rust
use spider_agent::{Agent, SpiderCloudToolConfig};

let spider_cloud = SpiderCloudToolConfig::new("spider-cloud-api-key")
    .with_api_url("https://api.spider.cloud")
    .with_tool_name_prefix("spider_cloud")
    .with_enable_ai_routes(true); // Only enable if your plan includes /ai/* routes

let agent = Agent::builder()
    .with_spider_cloud_config(spider_cloud)
    .build()?;
```

AI routes are disabled by default because they require a paid subscription:
https://spider.cloud/ai/pricing

Prompt-driven route orchestration example:

```bash
SPIDER_CLOUD_API_KEY=your-key cargo run -p spider_agent --example spider_cloud_prompt_flows \
  -- "run all flows for https://books.toscrape.com/ including search scrape crawl links transform unblocker"
```

To include AI routes (`/ai/crawl`, `/ai/scrape`, `/ai/search`, `/ai/browser`, `/ai/links`), enable both:
- config/env gate: `SPIDER_CLOUD_ENABLE_AI_ROUTES=1`
- prompt intent: include text like `include ai routes`

End-to-end release example (single-prompt pipeline + report):

```bash
SPIDER_CLOUD_API_KEY=your-key cargo run -p spider_agent --example spider_cloud_end_to_end \
  -- "Find top travel books on https://books.toscrape.com and return structured product fields"
```

Additional real-world examples:

```bash
# E-commerce competitor intelligence
SPIDER_CLOUD_API_KEY=your-key cargo run -p spider_agent --example spider_cloud_ecommerce_competitor \
  -- "https://books.toscrape.com/" "travel books"

# Job market intelligence pipeline
SPIDER_CLOUD_API_KEY=your-key cargo run -p spider_agent --example spider_cloud_jobs_pipeline \
  -- "rust engineer remote" "https://remoteok.com/remote-rust-jobs"
```

Notes:
- For markdown/text/raw/commonmark/bytes, use route-level `return_format`; transform is optional.
- For binary assets (PDF/images/files), `return_format: "bytes"` preserves fidelity.
- Run transform only when you explicitly need post-processing (`SPIDER_CLOUD_INCLUDE_TRANSFORM=1` in the examples).

You can also point this at any compatible endpoint (not only `api.spider.cloud`) and
use your own naming convention:

```rust
let spider_cloud = SpiderCloudToolConfig::new("provider-key")
    .with_api_url("https://my-gateway.example.com/v1")
    .with_tool_name_prefix("web_api"); // tools become web_api_search, web_api_scrape, etc.
```

## Advanced Configuration

### RemoteMultimodalConfig

Configure automation features. Use preset configurations for optimal performance:

```rust
use spider_agent::RemoteMultimodalConfig;

// Fast mode: All performance-positive features enabled
// - Tool calling (Auto), HTML diff (Auto), Confidence retries, Concurrent execution
let config = RemoteMultimodalConfig::fast();

// Fast with planning: Adds multi-step planning and self-healing
// Best for complex multi-step automations
let config = RemoteMultimodalConfig::fast_with_planning();

// Manual configuration for fine-grained control:
use spider_agent::{
    ToolCallingMode, HtmlDiffMode, ReasoningEffort,
    PlanningModeConfig, SelfHealingConfig, ConfidenceRetryStrategy,
};

let config = RemoteMultimodalConfig::default()
    .with_tool_calling_mode(ToolCallingMode::Auto)
    .with_html_diff_mode(HtmlDiffMode::Auto)
    .with_reasoning_effort(Some(ReasoningEffort::Medium))
    .with_planning_mode(PlanningModeConfig::default())
    .with_self_healing(SelfHealingConfig::default())
    .with_confidence_strategy(ConfidenceRetryStrategy::default())
    .with_concurrent_execution(true);
```

`reasoning_effort` is optional and only sent when configured, so OpenAI-compatible providers that do not support reasoning controls remain unaffected.

### Concurrent Action Chains

Execute independent actions in parallel using dependency graphs:

```rust
use spider_agent::{DependentStep, DependencyGraph, ConcurrentChainConfig, execute_graph};

// Define steps with dependencies
let steps = vec![
    DependentStep::new("fetch_data", json!({"Navigate": "https://example.com"})),
    DependentStep::new("click_a", json!({"Click": "#btn-a"})).depends_on("fetch_data"),
    DependentStep::new("click_b", json!({"Click": "#btn-b"})).depends_on("fetch_data"),
    DependentStep::new("submit", json!({"Click": "#submit"}))
        .depends_on("click_a")
        .depends_on("click_b"),
];

// Create dependency graph
let mut graph = DependencyGraph::new(steps)?;

// Execute with parallel-safe actions running concurrently
let config = ConcurrentChainConfig::default();
let result = execute_graph(&mut graph, &config, |step| async move {
    // Your execution logic here
    StepResult::success()
}).await;
```

### Schema Generation

Auto-generate JSON schemas from examples:

```rust
use spider_agent::{generate_schema, SchemaGenerationRequest};

let request = SchemaGenerationRequest {
    examples: vec![
        json!({"name": "Product A", "price": 19.99}),
        json!({"name": "Product B", "price": 29.99}),
    ],
    description: Some("Product listing data".to_string()),
    strict: false,
    name: Some("products".to_string()),
};

let schema = generate_schema(&request);
// Use schema.to_extraction_schema() for structured extraction
```

### Performance Features

| Feature | Default | `fast()` | Impact |
|---------|---------|----------|--------|
| Tool Calling | `JsonObject` | `Auto` | ~30% reduction in parse errors |
| HTML Diff | `Disabled` | `Auto` | 50-70% token reduction |
| Planning Mode | `None` | `None` | Fewer LLM round-trips |
| Parallel Synthesis | `None` | `None` | N pages = 1 LLM call |
| Confidence | `None` | `Enabled` | Smarter retry decisions |
| Self-Healing | `None` | `None` | Higher success rate on failures |
| Concurrent Execution | `false` | `true` | Parallel action execution |

**Recommended**: Use `RemoteMultimodalConfig::fast()` for optimal performance.

All features are opt-in with zero overhead when disabled.

## License

MIT
