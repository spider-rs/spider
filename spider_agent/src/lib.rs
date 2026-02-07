//! # Spider Agent
//!
//! A concurrent-safe multimodal agent for web automation and research.
//!
//! ## Features
//!
//! - **Concurrent-safe**: Designed to be wrapped in `Arc` for multi-task access
//! - **Feature-gated**: Only include dependencies you need
//! - **Multiple LLM providers**: OpenAI, OpenAI-compatible APIs
//! - **Multiple search providers**: Serper, Brave, Bing, Tavily
//! - **Browser automation**: Chrome support via chromiumoxide
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use spider_agent::{Agent, AgentConfig};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let agent = Arc::new(Agent::builder()
//!         .with_openai("sk-...", "gpt-4o-mini")
//!         .with_search_serper("serper-key")
//!         .build()?);
//!
//!     // Search
//!     let results = agent.search("rust web frameworks").await?;
//!     println!("Found {} results", results.len());
//!
//!     // Extract from first result
//!     let html = agent.fetch(&results.results[0].url).await?.html;
//!     let data = agent.extract(&html, "Extract framework name and features").await?;
//!     println!("{}", serde_json::to_string_pretty(&data)?);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Concurrent Execution
//!
//! ```rust,ignore
//! use spider_agent::Agent;
//! use std::sync::Arc;
//!
//! let agent = Arc::new(Agent::builder()
//!     .with_openai("sk-...", "gpt-4o")
//!     .with_search_serper("serper-key")
//!     .with_max_concurrent_llm_calls(10)
//!     .build()?);
//!
//! // Execute multiple searches concurrently
//! let queries = vec!["rust async", "rust web frameworks", "rust databases"];
//! let mut handles = Vec::new();
//!
//! for query in queries {
//!     let agent = agent.clone();
//!     let query = query.to_string();
//!     handles.push(tokio::spawn(async move {
//!         agent.search(&query).await
//!     }));
//! }
//!
//! // Collect results
//! for handle in handles {
//!     let result = handle.await??;
//!     println!("Found {} results", result.results.len());
//! }
//! ```
//!
//! ## Feature Flags
//!
//! - `openai` - OpenAI/OpenAI-compatible LLM provider
//! - `chrome` - Browser automation via chromiumoxide
//! - `search` - Base search functionality
//! - `search_serper` - Serper.dev search provider
//! - `search_brave` - Brave Search provider
//! - `search_bing` - Bing Search provider
//! - `search_tavily` - Tavily AI Search provider
//! - `full` - All features

#![warn(missing_docs)]

mod agent;
pub mod automation;
mod config;
mod error;
mod llm;
mod memory;
pub mod tools;

#[cfg(feature = "search")]
pub mod search;

#[cfg(feature = "chrome")]
pub mod browser;

#[cfg(feature = "webdriver")]
pub mod webdriver;

#[cfg(feature = "fs")]
pub mod temp;

// Re-exports
pub use agent::{Agent, AgentBuilder, FetchResult, PageExtraction};
pub use config::{
    AgentConfig, HtmlCleaningMode, LimitType, ResearchOptions, RetryConfig, SearchOptions,
    TimeRange, UsageLimits, UsageSnapshot, UsageStats,
};
pub use error::{AgentError, AgentResult, SearchError};
pub use llm::{
    CompletionOptions, CompletionResponse, LLMProvider, Message, MessageContent, TokenUsage,
};
pub use memory::AgentMemory;
pub use tools::{
    AuthConfig, CustomTool, CustomToolRegistry, CustomToolResult, HttpMethod,
    SpiderCloudToolConfig,
};

// Automation re-exports - core types
pub use automation::{
    ActResult, ActionRecord, ActionResult, ActionType, AutomationConfig, AutomationResult,
    AutomationUsage, CaptureProfile, ChainBuilder, ChainCondition, ChainContext, ChainResult,
    ChainStep, ChainStepResult, CleaningIntent, ClipViewport, ContentAnalysis, CostTier,
    ExtractionSchema, FormField, FormInfo, HtmlCleaningProfile, InteractiveElement, ModelEndpoint,
    ModelPolicy, NavigationOption, PageObservation, PromptUrlGate, ReasoningEffort,
    RecoveryStrategy, RetryPolicy, SelectorCache, SelectorCacheEntry, StructuredOutputConfig,
    VisionRouteMode,
};

// Automation re-exports - engine and configuration
pub use automation::{RemoteMultimodalConfig, RemoteMultimodalConfigs, RemoteMultimodalEngine};

// Automation re-exports - engine error types
pub use automation::{EngineError, EngineResult};

// Automation re-exports - helper functions
pub use automation::{
    best_effort_parse_json_object, extract_assistant_content, extract_last_code_block,
    extract_last_json_array, extract_last_json_boundaries, extract_last_json_object, extract_usage,
    fnv1a64, truncate_utf8_tail,
};

// Automation re-exports - HTML cleaning
pub use automation::{
    clean_html, clean_html_base, clean_html_full, clean_html_raw, clean_html_slim,
    clean_html_with_profile, clean_html_with_profile_and_intent, smart_clean_html,
};

// Automation re-exports - map result types
pub use automation::{categories, DiscoveredUrl, MapResult};

// Automation re-exports - memory operations
pub use automation::{AutomationMemory, MemoryOperation};

// Automation re-exports - system prompts
pub use automation::{
    ACT_SYSTEM_PROMPT, CONFIGURATION_SYSTEM_PROMPT, DEFAULT_SYSTEM_PROMPT, EXTRACT_SYSTEM_PROMPT,
    MAP_SYSTEM_PROMPT, OBSERVE_SYSTEM_PROMPT,
};

// Automation re-exports - concurrent chain types
pub use automation::{
    ConcurrentChainConfig, ConcurrentChainResult, DependencyGraph, DependentStep, StepResult,
};

// Automation re-exports - confidence types
pub use automation::{
    Alternative, ConfidenceRetryStrategy, ConfidenceSummary, ConfidenceTracker, ConfidentStep,
    Verification, VerificationType,
};

// Automation re-exports - tool calling types
pub use automation::{
    ActionToolSchemas, FunctionCall, FunctionDefinition, ToolCall, ToolCallingMode, ToolDefinition,
};

// Automation re-exports - HTML diff types
pub use automation::{
    ChangeType, DiffStats, ElementChange, HtmlDiffMode, HtmlDiffResult, PageStateDiff,
};

// Automation re-exports - planning types
pub use automation::{
    Checkpoint, CheckpointResult, CheckpointType, ExecutionPlan, PageState, PlanExecutionState,
    PlannedStep, PlanningModeConfig, ReplanContext,
};

// Automation re-exports - self-healing types
pub use automation::{
    HealedSelectorCache, HealingDiagnosis, HealingRequest, HealingResult, HealingStats,
    SelectorIssueType, SelfHealingConfig,
};

// Automation re-exports - synthesis types
pub use automation::{
    MultiPageContext, PageContext, PageContribution, SynthesisConfig, SynthesisResult,
};

// Automation re-exports - schema generation types
pub use automation::{
    build_schema_generation_prompt, generate_schema, infer_schema, infer_schema_from_examples,
    refine_schema, GeneratedSchema, SchemaCache, SchemaGenerationRequest,
};

// Automation re-exports - self-healing helper functions
pub use automation::extract_html_context;

// Automation re-exports - tool calling helper functions
pub use automation::{parse_tool_calls, tool_calls_to_steps};

// Automation re-exports - concurrent chain execution
pub use automation::execute_graph;

// Performance re-exports
pub use automation::cache::{CacheStats, CacheValue, SmartCache};
pub use automation::executor::{BatchExecutor, ChainExecutor, PrefetchManager};
pub use automation::router::{ModelRouter, RoutingDecision, TaskAnalysis, TaskCategory};

#[cfg(feature = "search")]
pub use agent::ResearchResult;

#[cfg(feature = "search")]
pub use search::{SearchProvider, SearchResult, SearchResults};

#[cfg(feature = "openai")]
pub use llm::OpenAIProvider;

#[cfg(feature = "search_serper")]
pub use search::SerperProvider;

#[cfg(feature = "search_brave")]
pub use search::BraveProvider;

#[cfg(feature = "search_bing")]
pub use search::BingProvider;

#[cfg(feature = "search_tavily")]
pub use search::TavilyProvider;

#[cfg(feature = "memvid")]
pub use automation::{
    ExperienceMemory, ExperienceMemoryConfig, ExperienceOutcome, ExperienceRecord,
    MemoryStats as ExperienceMemoryStats, RecalledExperience,
};

#[cfg(feature = "chrome")]
pub use browser::BrowserContext;

#[cfg(feature = "chrome")]
pub use automation::{
    run_remote_multimodal_with_page, run_spawn_pages_concurrent, run_spawn_pages_with_factory,
    run_spawn_pages_with_options, PageFactory, PageSetupFn, SpawnPageOptions, SpawnedPageResult,
};

#[cfg(feature = "webdriver")]
pub use webdriver::WebDriverContext;

#[cfg(feature = "fs")]
pub use temp::{TempFile, TempStorage};
