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
    // Actions
    ActionRecord,
    ActionResult,
    ActionType,
    // Chain types
    ChainBuilder,
    ChainCondition,
    ChainContext,
    ChainResult,
    ChainStep,
    ChainStepResult,
    // Concurrent chain types (data only)
    ConcurrentChainConfig,
    ConcurrentChainResult,
    DependencyGraph,
    DependentStep,
    StepResult,
    // Confidence types
    Alternative,
    ConfidenceRetryStrategy,
    ConfidenceSummary,
    ConfidenceTracker,
    ConfidentStep,
    Verification,
    VerificationType,
    // Config types
    arena_rank,
    is_url_allowed,
    merged_config,
    model_profile,
    reasoning_payload,
    supports_pdf,
    supports_video,
    supports_vision,
    AutomationConfig,
    CaptureProfile,
    CleaningIntent,
    ClipViewport,
    CostTier,
    HtmlCleaningProfile,
    ModelCapabilities,
    ModelEndpoint,
    ModelInfoEntry,
    ModelPolicy,
    ModelPricing,
    ModelProfile,
    ModelRanks,
    ReasoningEffort,
    RecoveryStrategy,
    RemoteMultimodalConfig,
    RetryPolicy,
    VisionRouteMode,
    MODEL_INFO,
    // Content analysis
    ContentAnalysis,
    // Helpers
    extract_assistant_content,
    extract_last_code_block,
    extract_last_json_array,
    extract_last_json_boundaries,
    extract_last_json_object,
    extract_usage,
    fnv1a64,
    truncate_utf8_tail,
    // HTML diff types
    ChangeType,
    DiffStats,
    ElementChange,
    HtmlDiffMode,
    HtmlDiffResult,
    PageStateDiff,
    // Map result types
    categories,
    DiscoveredUrl,
    MapResult,
    // Memory operations
    AutomationMemory,
    MemoryOperation,
    // Observation types
    ActResult,
    FormField,
    FormInfo,
    InteractiveElement,
    NavigationOption,
    PageObservation,
    // Planning types
    Checkpoint,
    CheckpointResult,
    CheckpointType,
    ExecutionPlan,
    PageState,
    PlanExecutionState,
    PlannedStep,
    PlanningModeConfig,
    ReplanContext,
    // Prompts
    ACT_SYSTEM_PROMPT,
    CHROME_AI_SYSTEM_PROMPT,
    CONFIGURATION_SYSTEM_PROMPT,
    DEFAULT_SYSTEM_PROMPT,
    EXTRACTION_ONLY_SYSTEM_PROMPT,
    EXTRACT_SYSTEM_PROMPT,
    MAP_SYSTEM_PROMPT,
    OBSERVE_SYSTEM_PROMPT,
    // Schema generation types
    build_schema_generation_prompt,
    generate_schema,
    infer_schema,
    infer_schema_from_examples,
    refine_schema,
    GeneratedSchema,
    SchemaCache,
    SchemaGenerationRequest,
    // Selector cache
    SelectorCache,
    SelectorCacheEntry,
    // Self-healing types
    extract_html_context,
    HealedSelectorCache,
    HealingDiagnosis,
    HealingRequest,
    HealingResult,
    HealingStats,
    SelectorIssueType,
    SelfHealingConfig,
    // Synthesis types
    MultiPageContext,
    PageContext,
    PageContribution,
    SynthesisConfig,
    SynthesisResult,
    // Tool calling types
    parse_tool_calls,
    tool_calls_to_steps,
    ActionToolSchemas,
    FunctionCall,
    FunctionDefinition,
    ToolCall,
    ToolCallingMode,
    ToolDefinition,
    // Top-level types
    AutomationResult,
    AutomationUsage,
    ExtractionSchema,
    PromptUrlGate,
    StructuredOutputConfig,
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
