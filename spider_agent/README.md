# Spider Agent

A concurrent-safe multimodal agent for web automation and research.

## Features

- **Concurrent-safe**: Designed to be wrapped in `Arc` for multi-task access
- **Feature-gated**: Only include dependencies you need
- **Multiple LLM providers**: OpenAI, OpenAI-compatible APIs
- **Multiple search providers**: Serper, Brave, Bing, Tavily
- **HTML extraction**: Clean and extract structured data from web pages
- **Research synthesis**: Combine search + extraction + LLM synthesis

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
    .with_search_serper(api_key)
    .build()
```

## License

MIT
