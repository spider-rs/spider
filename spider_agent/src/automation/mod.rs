//! Automation module for spider_agent.
//!
//! Provides sophisticated automation capabilities including:
//! - Action chains with conditional execution
//! - Self-healing selector cache
//! - Page observation and understanding
//! - Recovery strategies for resilient automation
//! - Content analysis for smart decisions
//! - Remote multimodal engine configuration
//! - HTML cleaning utilities
//! - Memory operations for session state
//!
//! ## Crate Structure
//!
//! Core types and constants are defined in [`spider_agent_types`] and re-exported here
//! for backward compatibility. HTML cleaning utilities come from [`spider_agent_html`].
//! Runtime components (engine, browser, executor, router) remain in this crate.

// =========================================================================
// Local modules (runtime components that stay in spider_agent)
// =========================================================================

#[cfg(feature = "chrome")]
mod browser;
pub mod cache;
mod concurrent_chain;
mod config;
mod engine;
mod engine_error;
pub mod executor;
mod helpers;
#[cfg(feature = "memvid")]
pub mod long_term_memory;
pub mod router;
#[cfg(feature = "skills")]
pub mod skills;

// =========================================================================
// Re-exports from spider_agent_types (all pure data types)
// =========================================================================

pub use spider_agent_types::{
    // Config types
    arena_rank,
    // Schema generation types
    build_schema_generation_prompt,
    // Map result types
    categories,
    // Helpers
    extract_assistant_content,
    // Self-healing types
    extract_html_context,
    extract_last_code_block,
    extract_last_json_array,
    extract_last_json_boundaries,
    extract_last_json_object,
    extract_usage,
    fnv1a64,
    generate_schema,
    infer_schema,
    infer_schema_from_examples,
    is_url_allowed,
    merged_config,
    model_profile,
    // Tool calling types
    parse_tool_calls,
    reasoning_payload,
    refine_schema,
    supports_pdf,
    supports_video,
    supports_vision,
    tool_calls_to_steps,
    truncate_utf8_tail,
    // Observation types
    ActResult,
    // Actions
    ActionRecord,
    ActionResult,
    ActionToolSchemas,
    ActionType,
    // Confidence types
    Alternative,
    AutomationConfig,
    // Memory operations
    AutomationMemory,
    // Top-level types
    AutomationResult,
    AutomationUsage,
    CaptureProfile,
    // Chain types
    ChainBuilder,
    ChainCondition,
    ChainContext,
    ChainResult,
    ChainStep,
    ChainStepResult,
    // HTML diff types
    ChangeType,
    // Planning types
    Checkpoint,
    CheckpointResult,
    CheckpointType,
    CleaningIntent,
    ClipViewport,
    // Concurrent chain types (data only)
    ConcurrentChainConfig,
    ConcurrentChainResult,
    ConfidenceRetryStrategy,
    ConfidenceSummary,
    ConfidenceTracker,
    ConfidentStep,
    // Content analysis
    ContentAnalysis,
    CostTier,
    DependencyGraph,
    DependentStep,
    DiffStats,
    DiscoveredUrl,
    ElementChange,
    ExecutionPlan,
    ExtractionSchema,
    FormField,
    FormInfo,
    FunctionCall,
    FunctionDefinition,
    GeneratedSchema,
    HealedSelectorCache,
    HealingDiagnosis,
    HealingRequest,
    HealingResult,
    HealingStats,
    HtmlCleaningProfile,
    HtmlDiffMode,
    HtmlDiffResult,
    InteractiveElement,
    MapResult,
    MemoryOperation,
    ModelCapabilities,
    ModelEndpoint,
    ModelInfoEntry,
    ModelPolicy,
    ModelPricing,
    ModelProfile,
    ModelRanks,
    // Synthesis types
    MultiPageContext,
    NavigationOption,
    PageContext,
    PageContribution,
    PageObservation,
    PageState,
    PageStateDiff,
    PlanExecutionState,
    PlannedStep,
    PlanningModeConfig,
    PromptUrlGate,
    ReasoningEffort,
    RecoveryStrategy,
    RemoteMultimodalConfig,
    ReplanContext,
    RetryPolicy,
    SchemaCache,
    SchemaGenerationRequest,
    // Selector cache
    SelectorCache,
    SelectorCacheEntry,
    SelectorIssueType,
    SelfHealingConfig,
    StepResult,
    StructuredOutputConfig,
    SynthesisConfig,
    SynthesisResult,
    ToolCall,
    ToolCallingMode,
    ToolDefinition,
    Verification,
    VerificationType,
    VisionRouteMode,
    // Prompts
    ACT_SYSTEM_PROMPT,
    CHROME_AI_SYSTEM_PROMPT,
    CONFIGURATION_SYSTEM_PROMPT,
    DEFAULT_SYSTEM_PROMPT,
    EXTRACTION_ONLY_SYSTEM_PROMPT,
    EXTRACT_SYSTEM_PROMPT,
    MAP_SYSTEM_PROMPT,
    MODEL_INFO,
    OBSERVE_SYSTEM_PROMPT,
};

// =========================================================================
// Re-exports from spider_agent_html (HTML cleaning utilities)
// =========================================================================

pub use spider_agent_html::{
    clean_html, clean_html_base, clean_html_full, clean_html_raw, clean_html_slim,
    clean_html_with_profile, clean_html_with_profile_and_intent, smart_clean_html,
};

// =========================================================================
// Re-exports from local modules (runtime components)
// =========================================================================

// Re-export RemoteMultimodalConfigs from local config (has heavy deps)
pub use config::RemoteMultimodalConfigs;

// Re-export engine
pub use engine::RemoteMultimodalEngine;

// Re-export error types
pub use engine_error::{EngineError, EngineResult};

// Re-export helpers that depend on EngineError
pub use helpers::best_effort_parse_json_object;

// Re-export concurrent chain execution (needs tokio)
pub use concurrent_chain::execute_graph;

// Re-export long-term memory types (memvid feature)
#[cfg(feature = "memvid")]
pub use long_term_memory::{
    ExperienceMemory, ExperienceMemoryConfig, ExperienceOutcome, ExperienceRecord, MemoryStats,
    RecalledExperience,
};

// Re-export browser functions (chrome feature)
#[cfg(feature = "chrome")]
pub use browser::{
    run_remote_multimodal_with_page, run_spawn_pages_concurrent, run_spawn_pages_with_factory,
    run_spawn_pages_with_options, PageFactory, PageSetupFn, SpawnPageOptions, SpawnedPageResult,
};
