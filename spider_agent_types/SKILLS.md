# spider_agent_types — Skills & Capabilities

Pure data types and constants for spider_agent automation with **zero heavy runtime dependencies** (no tokio, reqwest, dashmap, or chromey). Use this crate when you need automation types without the full agent runtime.

## Dependencies

`serde` + `serde_json` + `aho-corasick` + `llm_models_spider`

---

## Actions (`actions`)

| Export | Kind | Description |
|--------|------|-------------|
| `ActionType` | enum | 30+ action variants: Navigate, Click, ClickPoint, ClickDrag, Fill, KeyPress, Scroll, Wait, Screenshot, Extract, and more |
| `ActionResult` | struct | Result of action execution — success flag, duration, usage stats, screenshots, before/after URLs |
| `ActionRecord` | struct | Step-by-step record with retry counts for tracing automation history |

## Chain Execution (`chain`)

| Export | Kind | Description |
|--------|------|-------------|
| `ChainStep` | struct | Single step in an action chain with conditions, retries, timeouts |
| `ChainCondition` | enum | Conditional execution: UrlContains, ElementExists, PreviousSucceeded, All, Any, Not |
| `ChainContext` | struct | Runtime context for evaluating chain conditions against page state |
| `ChainResult` | struct | Overall chain execution result with step-level detail |
| `ChainStepResult` | struct | Individual step result (executed/skipped, action, error, extracted data) |
| `ChainBuilder` | struct | Fluent builder for creating action chains |

## Concurrent Chains (`concurrent_chain`)

| Export | Kind | Description |
|--------|------|-------------|
| `DependentStep` | struct | Step with dependency declarations (`depends_on`, `blocks`) |
| `DependencyGraph` | struct | DAG for managing execution order with `ready_steps()`, `complete()`, `max_parallelism()` |
| `ConcurrentChainConfig` | struct | Config: max parallelism, stop-on-failure, step timeouts |
| `ConcurrentChainResult` | struct | Aggregate result with per-step detail |
| `StepResult` | struct | Individual step success/failure with output |

## Confidence Tracking (`confidence`)

| Export | Kind | Description |
|--------|------|-------------|
| `ConfidentStep` | struct | Action step with confidence score and ranked alternatives |
| `Alternative` | struct | Alternative action with confidence and description |
| `Verification` | struct | Post-action verification (URL check, element exists, JS condition) |
| `VerificationType` | enum | UrlContains, ElementExists, TextContains, JsCondition |
| `ConfidenceRetryStrategy` | struct | Retry policy based on confidence thresholds with adaptive decay |
| `ConfidenceTracker` | struct | Session-wide confidence statistics tracker |
| `ConfidenceSummary` | struct | Snapshot of confidence metrics (avg, min, max, low-confidence ratio) |

## Configuration (`config`)

| Export | Kind | Description |
|--------|------|-------------|
| `RemoteMultimodalConfig` | struct | Per-task config: model, API key, vision, HTML diff, tool calling, planning, self-healing |
| `AutomationConfig` | struct | Task-level settings: goal, recovery strategy, retry policy, cleaning profile |
| `ModelProfile` | struct | Model metadata: name, provider, context window, pricing |
| `ModelCapabilities` | struct | Feature flags: vision, video, PDF, reasoning support |
| `ModelPolicy` | struct | Model selection by cost tier (small/medium/large) |
| `ModelEndpoint` | struct | Model endpoint override for dual-model routing |
| `ModelPricing` | struct | Input/output token pricing |
| `ModelInfoEntry` | struct | Static model registry entry |
| `ModelRanks` | struct | Arena ranking data |
| `HtmlCleaningProfile` | enum | Raw, Minimal, Default, Slim, Aggressive |
| `CleaningIntent` | enum | Intent-based cleaning (extraction, observation, etc.) |
| `CaptureProfile` | struct | Screenshot capture configuration |
| `ClipViewport` | struct | Viewport clipping region |
| `CostTier` | enum | Low, Medium, High |
| `RecoveryStrategy` | enum | Retry, Alternative, Skip, Abort |
| `RetryPolicy` | struct | Max attempts + backoff configuration |
| `ReasoningEffort` | enum | Reasoning effort levels |
| `VisionRouteMode` | enum | Vision vs text model routing strategy |
| `MODEL_INFO` | const | Static registry of known models with capabilities and pricing |
| `model_profile()` | fn | Look up a model's profile by name |
| `supports_vision()` | fn | Check if model supports vision |
| `supports_video()` | fn | Check if model supports video |
| `supports_pdf()` | fn | Check if model supports PDF |
| `arena_rank()` | fn | Get model's arena ranking |
| `reasoning_payload()` | fn | Build reasoning payload for model |
| `merged_config()` | fn | Merge two configs with override semantics |
| `is_url_allowed()` | fn | Check URL against allowlist/blocklist |

## Content Analysis (`content`)

| Export | Kind | Description |
|--------|------|-------------|
| `ContentAnalysis` | struct | Full HTML content analysis: text ratio, visual elements, dynamic content detection, cleanable bytes estimate. Uses aho-corasick for fast pattern matching. Methods: `analyze()`, `quick_needs_screenshot()`, `has_visual_elements_quick()`, `recommended_cleaning()` |

## Helpers (`helpers`)

| Export | Kind | Description |
|--------|------|-------------|
| `extract_assistant_content()` | fn | Extract text from OpenAI-format LLM response JSON |
| `extract_usage()` | fn | Extract token usage from response JSON |
| `extract_last_code_block()` | fn | Extract last ````json` or ```` code block from markdown |
| `extract_last_json_object()` | fn | Extract last balanced `{...}` from text |
| `extract_last_json_array()` | fn | Extract last balanced `[...]` from text |
| `extract_last_json_boundaries()` | fn | Extract last balanced delimiter pair with positions |
| `truncate_utf8_tail()` | fn | Safe UTF-8 truncation by byte limit |
| `fnv1a64()` | fn | FNV-1a 64-bit hash function |

## HTML Diff (`html_diff`)

| Export | Kind | Description |
|--------|------|-------------|
| `PageStateDiff` | struct | Tracks HTML changes between automation rounds for 50-70% token savings |
| `HtmlDiffMode` | enum | Disabled, Enabled, Auto |
| `HtmlDiffResult` | struct | Diff output: changed/added/removed elements, condensed HTML, savings ratio |
| `ElementChange` | struct | Single element change with path and content |
| `ChangeType` | enum | ContentChanged, AttributeChanged, Appeared, Disappeared, Moved |
| `DiffStats` | struct | Cumulative diff performance statistics |

## Map Results (`map_result`)

| Export | Kind | Description |
|--------|------|-------------|
| `MapResult` | struct | Result of page discovery/mapping with AI-generated metadata |
| `DiscoveredUrl` | struct | URL with text, description, relevance score, category |
| `categories` | mod | Constants: NAVIGATION, CONTENT, EXTERNAL, RESOURCE, ACTION |

## Memory Operations (`memory_ops`)

| Export | Kind | Description |
|--------|------|-------------|
| `MemoryOperation` | enum | Set, Delete, Clear — LLM-driven session state operations |
| `AutomationMemory` | struct | In-memory KV store with extraction history, visited URLs, action log, level attempt tracking |

## Page Observation (`observation`)

| Export | Kind | Description |
|--------|------|-------------|
| `PageObservation` | struct | Full page state: URL, title, interactive elements, forms, navigation options |
| `InteractiveElement` | struct | Clickable/input element with selector, type, text, bounds |
| `FormInfo` | struct | Form descriptor with fields and action URL |
| `FormField` | struct | Field with type, label, required flag, options |
| `NavigationOption` | struct | Page navigation link/button |
| `ActResult` | struct | Single action result for observation |

## Planning (`planning`)

| Export | Kind | Description |
|--------|------|-------------|
| `PlanningModeConfig` | struct | Config: max steps, replan on failure, auto-execute threshold |
| `PlannedStep` | struct | Step with ID, action, dependencies, confidence, critical flag |
| `ExecutionPlan` | struct | Multi-step plan from LLM with dependency ordering |
| `PlanExecutionState` | struct | Live tracking of plan progress |
| `Checkpoint` | struct | Verification point (URL, element, text, JS condition) |
| `CheckpointResult` | struct | Pass/fail result of checkpoint verification |
| `CheckpointType` | enum | URL, Element, Text, JsCondition |
| `ReplanContext` | struct | Context passed to LLM when replanning after failure |
| `PageState` | struct | Page state snapshot for planning decisions |

## System Prompts (`prompts`)

| Export | Kind | Description |
|--------|------|-------------|
| `DEFAULT_SYSTEM_PROMPT` | const | Lean action-binding prompt for web automation |
| `CHROME_AI_SYSTEM_PROMPT` | const | Compact prompt optimized for Gemini Nano (~1500 chars) |
| `OBSERVE_SYSTEM_PROMPT` | const | Page observation mode prompt |
| `EXTRACT_SYSTEM_PROMPT` | const | Data extraction mode prompt |
| `MAP_SYSTEM_PROMPT` | const | Page discovery/mapping mode prompt |
| `ACT_SYSTEM_PROMPT` | const | Action execution mode prompt |
| `CONFIGURATION_SYSTEM_PROMPT` | const | Configuration mode prompt |
| `EXTRACTION_ONLY_SYSTEM_PROMPT` | const | Extraction-only mode prompt |

## Schema Generation (`schema_gen`)

| Export | Kind | Description |
|--------|------|-------------|
| `SchemaGenerationRequest` | struct | Request to generate schema from example data |
| `GeneratedSchema` | struct | Result with JSON schema, field descriptions, confidence |
| `SchemaCache` | struct | LRU cache of generated schemas with usage tracking |
| `generate_schema()` | fn | Generate schema from request |
| `infer_schema()` | fn | Infer schema from a single JSON value |
| `infer_schema_from_examples()` | fn | Infer schema from multiple examples (merges types) |
| `refine_schema()` | fn | Refine existing schema with new examples |
| `build_schema_generation_prompt()` | fn | Build LLM prompt for schema generation |

## Selector Cache (`selector_cache`)

| Export | Kind | Description |
|--------|------|-------------|
| `SelectorCache` | struct | LRU cache of CSS selectors with success/failure tracking and reliability scores |
| `SelectorCacheEntry` | struct | Single entry with selector, hit/miss counts, domain, reliability score |

## Self-Healing (`self_healing`)

| Export | Kind | Description |
|--------|------|-------------|
| `SelfHealingConfig` | struct | Config: max attempts, min confidence, cache healed selectors |
| `HealingRequest` | struct | Request to heal a failed selector with HTML context |
| `HealingResult` | struct | Healed selector with confidence and diagnosis |
| `HealingDiagnosis` | struct | Diagnostic info about why selector failed |
| `HealingStats` | struct | Cumulative healing statistics |
| `HealedSelectorCache` | struct | Cache of previously healed selectors |
| `SelectorIssueType` | enum | Type of selector failure (stale, ambiguous, dynamic, etc.) |
| `extract_html_context()` | fn | Extract surrounding HTML for healing context |

## Multi-Page Synthesis (`synthesis`)

| Export | Kind | Description |
|--------|------|-------------|
| `SynthesisConfig` | struct | Config: max tokens per page, pre-summarize, min relevance threshold |
| `PageContext` | struct | Single page context with URL, title, extracted data, relevance |
| `MultiPageContext` | struct | Combined context from multiple pages for synthesis |
| `PageContribution` | struct | How much each page contributed to the synthesis |
| `SynthesisResult` | struct | Final synthesis output with contributions and confidence |

## Tool Calling (`tool_calling`)

| Export | Kind | Description |
|--------|------|-------------|
| `ToolCallingMode` | enum | JsonObject, ToolCalling, Auto — action formatting strategy |
| `ToolDefinition` | struct | OpenAI-compatible tool definition |
| `FunctionDefinition` | struct | Function name, description, parameters schema |
| `ToolCall` | struct | Parsed tool call from LLM response |
| `FunctionCall` | struct | Function name + arguments |
| `ActionToolSchemas` | struct | Pre-built JSON schemas for all action types |
| `parse_tool_calls()` | fn | Parse tool calls from LLM response JSON |
| `tool_calls_to_steps()` | fn | Convert tool calls to automation steps |

## Top-Level Types

| Export | Kind | Description |
|--------|------|-------------|
| `PromptUrlGate` | struct | URL-based config overrides with exact and prefix matching |
| `AutomationUsage` | struct | Token + API call tracking (LLM, search, fetch, browser, custom tools) |
| `ExtractionSchema` | struct | JSON Schema definition for structured data extraction |
| `StructuredOutputConfig` | struct | Structured output mode with strict schema enforcement |
| `AutomationResult` | struct | Overall automation result: success, steps, extracted data, usage, screenshots, spawn pages |
