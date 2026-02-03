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
pub use config::{AgentConfig, HtmlCleaningMode, ResearchOptions, RetryConfig, SearchOptions, TimeRange, UsageSnapshot, UsageStats};
pub use error::{AgentError, AgentResult, SearchError};
pub use llm::{CompletionOptions, CompletionResponse, LLMProvider, Message, MessageContent, TokenUsage};
pub use memory::AgentMemory;

// Automation re-exports
pub use automation::{
    ActionRecord, ActionResult, ActionType, AutomationConfig, AutomationResult, AutomationUsage,
    CaptureProfile, ChainBuilder, ChainCondition, ChainContext, ChainResult, ChainStep,
    ChainStepResult, CleaningIntent, ContentAnalysis, CostTier, ExtractionSchema, FormField,
    FormInfo, HtmlCleaningProfile, InteractiveElement, ModelPolicy, NavigationOption,
    PageObservation, RecoveryStrategy, RetryPolicy, SelectorCache, SelectorCacheEntry,
    StructuredOutputConfig,
};

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

#[cfg(feature = "chrome")]
pub use browser::BrowserContext;

#[cfg(feature = "webdriver")]
pub use webdriver::WebDriverContext;

#[cfg(feature = "fs")]
pub use temp::{TempStorage, TempFile};
