# Spider Agent Types

Pure data types and constants for `spider_agent` automation with zero heavy runtime dependencies.

## Overview

This crate extracts all type definitions, system prompts, and helper utilities from `spider_agent` into a lightweight, dependency-minimal package. Use it when you need automation types without the full agent runtime (no `tokio`, `reqwest`, `dashmap`, or `chromey`).

## Installation

```toml
[dependencies]
spider_agent_types = "0.1"
```

## What's Included

- **Action Types** — `ActionType` (30+ variants), `ActionResult`, `ActionRecord`
- **Chain Execution** — `ChainStep`, `ChainCondition`, `ChainBuilder` for sequential action chains
- **Concurrent Chains** — `DependencyGraph`, `DependentStep` for parallel execution with dependency ordering
- **Confidence Tracking** — `ConfidenceTracker`, `ConfidenceRetryStrategy` for smarter retry decisions
- **Configuration** — `RemoteMultimodalConfig`, `AutomationConfig`, `ModelProfile`, `ModelCapabilities`
- **Content Analysis** — `ContentAnalysis` with aho-corasick for fast HTML structure detection
- **HTML Diff** — `PageStateDiff`, `HtmlDiffResult` for 50-70% token reduction between rounds
- **Memory Operations** — `AutomationMemory`, `MemoryOperation` for session state
- **Page Observation** — `PageObservation`, `InteractiveElement`, `FormInfo`
- **Planning** — `PlanningModeConfig`, `ExecutionPlan`, `PlannedStep` with checkpoints
- **System Prompts** — `DEFAULT_SYSTEM_PROMPT`, `CHROME_AI_SYSTEM_PROMPT`, and 6 more
- **Schema Generation** — `generate_schema()`, `infer_schema()`, `SchemaCache`
- **Selector Cache** — `SelectorCache` with LRU eviction and reliability scoring
- **Self-Healing** — `SelfHealingConfig`, `HealingRequest`, `HealedSelectorCache`
- **Synthesis** — `SynthesisConfig`, `MultiPageContext` for multi-page analysis
- **Tool Calling** — `ToolCallingMode`, `ToolDefinition`, `parse_tool_calls()` for OpenAI-compatible function calling
- **Helpers** — JSON extraction, LLM response parsing, FNV hashing, UTF-8 truncation

## Quick Start

```rust
use spider_agent_types::{
    ActionType, AutomationConfig, RemoteMultimodalConfig,
    ToolCallingMode, HtmlDiffMode, ContentAnalysis,
};

// Create automation config
let config = AutomationConfig::new("Extract product data");

// Analyze HTML content
let analysis = ContentAnalysis::analyze("<html>...</html>");
println!("Needs screenshot: {}", analysis.needs_screenshot);
println!("Text ratio: {:.1}%", analysis.text_ratio * 100.0);

// Parse LLM responses
use spider_agent_types::{extract_last_json_object, extract_assistant_content};
let json = extract_last_json_object(r#"Here is the result: {"name": "test"}"#);
```

## Model Registry

Built-in model profiles for 40+ models with capabilities and pricing:

```rust
use spider_agent_types::{model_profile, supports_vision, MODEL_INFO};

if let Some(profile) = model_profile("gpt-4o") {
    println!("Context: {} tokens", profile.context_window);
    println!("Vision: {}", supports_vision("gpt-4o"));
}
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `serde` + `serde_json` | Serialization |
| `aho-corasick` | Fast multi-pattern matching for content analysis |
| `llm_models_spider` | Model capabilities detection |

## License

MIT
