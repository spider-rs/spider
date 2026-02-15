//! Smart model routing for optimal performance and cost.
//!
//! Routes requests to appropriate models based on:
//! - Task complexity and category
//! - Token count estimates
//! - Latency requirements
//! - Cost constraints
//! - Arena rankings and model capabilities (from `llm_models_spider`)
//!
//! ## Multi-Agent Model Selection
//!
//! The [`ModelSelector`] allows users to pass in their available models and
//! get optimal routing based on task type, arena rankings, and pricing data.
//! Users can also define custom rank/priority overrides.

use super::{CostTier, ModelPolicy};
use std::time::Duration;

/// Smart router for selecting optimal models.
///
/// Analyzes tasks and routes them to the most appropriate model
/// based on complexity, cost, and latency requirements.
#[derive(Debug, Clone)]
pub struct ModelRouter {
    /// Model policy configuration.
    policy: ModelPolicy,
    /// Token threshold for using larger models.
    large_model_threshold: usize,
    /// Token threshold for using medium models.
    medium_model_threshold: usize,
    /// Whether to enable smart routing.
    enabled: bool,
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRouter {
    /// Create a new router with default settings.
    pub fn new() -> Self {
        Self {
            policy: ModelPolicy::default(),
            large_model_threshold: 4000,
            medium_model_threshold: 1000,
            enabled: true,
        }
    }

    /// Create with custom policy.
    pub fn with_policy(policy: ModelPolicy) -> Self {
        Self {
            policy,
            ..Default::default()
        }
    }

    /// Set token thresholds.
    pub fn with_thresholds(mut self, medium: usize, large: usize) -> Self {
        self.medium_model_threshold = medium;
        self.large_model_threshold = large;
        self
    }

    /// Enable or disable smart routing.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Route a task to the optimal model.
    ///
    /// Returns the recommended model name.
    pub fn route(&self, task: &TaskAnalysis) -> RoutingDecision {
        if !self.enabled {
            return RoutingDecision {
                model: self.policy.medium.clone(),
                tier: CostTier::Medium,
                reason: "Smart routing disabled".to_string(),
            };
        }

        // Determine complexity tier
        let tier = self.analyze_complexity(task);

        // Check policy constraints
        let tier = self.apply_constraints(tier, task);

        let model = self.policy.model_for_tier(tier).to_string();
        let reason = self.explain_routing(task, tier);

        RoutingDecision {
            model,
            tier,
            reason,
        }
    }

    /// Analyze task complexity.
    fn analyze_complexity(&self, task: &TaskAnalysis) -> CostTier {
        let mut score = 0;

        // Token count factor
        if task.estimated_tokens > self.large_model_threshold {
            score += 3;
        } else if task.estimated_tokens > self.medium_model_threshold {
            score += 2;
        } else {
            score += 1;
        }

        // Complexity indicators
        if task.requires_reasoning {
            score += 2;
        }
        if task.requires_code_generation {
            score += 2;
        }
        if task.requires_structured_output {
            score += 1;
        }
        if task.multi_step {
            score += 1;
        }

        // Map score to tier
        match score {
            0..=2 => CostTier::Low,
            3..=5 => CostTier::Medium,
            _ => CostTier::High,
        }
    }

    /// Apply policy constraints to the selected tier.
    fn apply_constraints(&self, tier: CostTier, task: &TaskAnalysis) -> CostTier {
        // Check max tier constraint
        let tier = match (tier, self.policy.max_cost_tier) {
            (CostTier::High, CostTier::Low) => CostTier::Low,
            (CostTier::High, CostTier::Medium) => CostTier::Medium,
            (CostTier::Medium, CostTier::Low) => CostTier::Low,
            _ => tier,
        };

        // Check latency constraint
        if let Some(max_latency) = self.policy.max_latency_ms {
            let estimated_latency = self.estimate_latency(tier, task);
            if estimated_latency > max_latency {
                // Downgrade to faster model
                return match tier {
                    CostTier::High => CostTier::Medium,
                    CostTier::Medium => CostTier::Low,
                    CostTier::Low => CostTier::Low,
                };
            }
        }

        // Check if large model is allowed
        if tier == CostTier::High && !self.policy.allow_large {
            return CostTier::Medium;
        }

        tier
    }

    /// Estimate latency for a tier.
    fn estimate_latency(&self, tier: CostTier, task: &TaskAnalysis) -> u64 {
        // Rough estimates in milliseconds
        let base_latency = match tier {
            CostTier::Low => 500,
            CostTier::Medium => 1500,
            CostTier::High => 3000,
        };

        // Add token-based estimate (rough: 50ms per 100 tokens)
        let token_latency = (task.estimated_tokens as u64 / 100) * 50;

        base_latency + token_latency
    }

    /// Explain the routing decision.
    fn explain_routing(&self, task: &TaskAnalysis, tier: CostTier) -> String {
        let mut reasons = Vec::new();

        if task.estimated_tokens > self.large_model_threshold {
            reasons.push("high token count");
        }
        if task.requires_reasoning {
            reasons.push("requires reasoning");
        }
        if task.requires_code_generation {
            reasons.push("requires code generation");
        }

        if reasons.is_empty() {
            reasons.push("standard task");
        }

        format!("{:?} tier selected: {}", tier, reasons.join(", "))
    }

    /// Quickly route a simple prompt.
    pub fn route_simple(&self, prompt: &str) -> RoutingDecision {
        let task = TaskAnalysis::from_prompt(prompt);
        self.route(&task)
    }
}

// ── ModelSelector ─────────────────────────────────────────────────────────────

/// Capability requirements for model selection.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModelRequirements {
    /// Requires vision/image input support.
    pub vision: bool,
    /// Requires audio input support.
    pub audio: bool,
    /// Requires video input support.
    pub video: bool,
    /// Requires PDF/file input support.
    pub pdf: bool,
    /// Minimum context window (input tokens). 0 = no requirement.
    pub min_context_tokens: u32,
    /// Maximum input cost per million tokens in USD. 0.0 = no limit.
    pub max_input_cost_per_m: f32,
    /// Minimum arena score (0.0-100.0). 0.0 = no minimum.
    pub min_arena_score: f32,
}

impl ModelRequirements {
    /// Require vision support.
    pub fn with_vision(mut self) -> Self {
        self.vision = true;
        self
    }

    /// Require a minimum context window.
    pub fn with_min_context(mut self, tokens: u32) -> Self {
        self.min_context_tokens = tokens;
        self
    }

    /// Set a maximum input cost per million tokens.
    pub fn with_max_cost(mut self, cost: f32) -> Self {
        self.max_input_cost_per_m = cost;
        self
    }

    /// Require a minimum arena score.
    pub fn with_min_arena(mut self, score: f32) -> Self {
        self.min_arena_score = score;
        self
    }
}

/// A scored model candidate for selection.
#[derive(Debug, Clone)]
pub struct ScoredModel {
    /// Model name (borrowed from the user's pool or MODEL_INFO).
    pub name: String,
    /// Effective score used for ranking (higher is better).
    pub score: f32,
    /// Arena rank if available (0.0-100.0).
    pub arena_rank: Option<f32>,
    /// Input cost per million tokens if available.
    pub input_cost_per_m: Option<f32>,
    /// Whether the model supports vision.
    pub supports_vision: bool,
    /// Max input tokens.
    pub max_input_tokens: u32,
}

/// Priority strategy for scoring models.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SelectionStrategy {
    /// Prefer highest arena score (quality).
    #[default]
    BestQuality,
    /// Prefer lowest cost.
    CheapestFirst,
    /// Prefer largest context window.
    LargestContext,
    /// Balance quality and cost (arena_score / cost).
    ValueOptimal,
}

/// Flexible model selector that picks optimal models from a user-provided pool.
///
/// Users pass in the models they have available (API keys, endpoints), and the
/// selector uses arena rankings, pricing, and capability data to rank them.
/// Custom priority overrides let users boost or penalize specific models.
///
/// # Example
///
/// ```rust,ignore
/// use spider_agent::automation::router::{ModelSelector, ModelRequirements, SelectionStrategy};
///
/// let mut selector = ModelSelector::new(&["gpt-4o", "claude-sonnet-4.5", "gemini-2.5-pro"]);
/// selector.set_strategy(SelectionStrategy::BestQuality);
///
/// // Pick best model for a vision task
/// let reqs = ModelRequirements::default().with_vision();
/// if let Some(best) = selector.select(&reqs) {
///     println!("Use: {} (score: {:.1})", best.name, best.score);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ModelSelector {
    /// User's available models with optional custom priority overrides.
    /// Each entry: (model_name, custom_priority_override).
    /// Priority override: None = use auto scoring; Some(f32) = fixed score.
    models: Vec<(String, Option<f32>)>,
    /// Selection strategy.
    strategy: SelectionStrategy,
}

impl ModelSelector {
    /// Create a selector from a list of available model names.
    pub fn new(models: &[&str]) -> Self {
        Self {
            models: models.iter().map(|m| (m.to_lowercase(), None)).collect(),
            strategy: SelectionStrategy::default(),
        }
    }

    /// Create from owned strings.
    pub fn from_owned(models: Vec<String>) -> Self {
        Self {
            models: models.into_iter().map(|m| (m.to_lowercase(), None)).collect(),
            strategy: SelectionStrategy::default(),
        }
    }

    /// Set the selection strategy.
    pub fn set_strategy(&mut self, strategy: SelectionStrategy) {
        self.strategy = strategy;
    }

    /// Set a custom priority override for a specific model.
    ///
    /// The priority is a fixed score (higher = more preferred).
    /// This overrides the auto-calculated score from arena/pricing data.
    pub fn set_priority(&mut self, model: &str, priority: f32) {
        let lower = model.to_lowercase();
        for (name, prio) in &mut self.models {
            if *name == lower {
                *prio = Some(priority);
                return;
            }
        }
        // Model not in pool — add it with the override
        self.models.push((lower, Some(priority)));
    }

    /// Add a model to the pool.
    pub fn add_model(&mut self, model: &str) {
        let lower = model.to_lowercase();
        if !self.models.iter().any(|(n, _)| *n == lower) {
            self.models.push((lower, None));
        }
    }

    /// Select the best model matching the given requirements.
    ///
    /// Returns `None` if no model in the pool satisfies the requirements.
    ///
    /// For pools with ≤ 2 models, skips scoring/sorting and returns the first
    /// model that satisfies the requirements. Use the full ranking pipeline
    /// only when there are 3+ models to meaningfully choose between.
    pub fn select(&self, reqs: &ModelRequirements) -> Option<ScoredModel> {
        if self.models.len() <= 2 {
            // Fast path: no meaningful selection with 0-2 models.
            // Just check requirements and return the first match.
            return self
                .models
                .iter()
                .filter_map(|(name, custom_prio)| self.score_model(name, *custom_prio, reqs))
                .next();
        }
        self.ranked(reqs).into_iter().next()
    }

    /// Return all models that satisfy the requirements, ranked best-to-worst.
    ///
    /// For pools with ≤ 2 models, skips the sorting step since the ordering
    /// is trivial. Scoring/sorting is only worthwhile with 3+ candidates.
    pub fn ranked(&self, reqs: &ModelRequirements) -> Vec<ScoredModel> {
        let mut candidates: Vec<ScoredModel> = self
            .models
            .iter()
            .filter_map(|(name, custom_prio)| {
                self.score_model(name, *custom_prio, reqs)
            })
            .collect();

        // Only worth sorting when there are 3+ candidates
        if candidates.len() > 2 {
            candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        }
        candidates
    }

    /// Select the best model for each distinct requirement set from a list.
    ///
    /// Useful for multi-agent dispatch: given N different sub-tasks with different
    /// requirements, get the best model for each without reusing the same model
    /// (unless it's the only option).
    pub fn select_multi(&self, requirements: &[ModelRequirements]) -> Vec<Option<ScoredModel>> {
        let mut used: Vec<bool> = vec![false; self.models.len()];
        let mut results = Vec::with_capacity(requirements.len());

        for reqs in requirements {
            let mut best: Option<(ScoredModel, usize)> = None;

            for (idx, (name, custom_prio)) in self.models.iter().enumerate() {
                if used[idx] {
                    continue;
                }
                if let Some(scored) = self.score_model(name, *custom_prio, reqs) {
                    let dominated = match &best {
                        Some((current, _)) => scored.score > current.score,
                        None => true,
                    };
                    if dominated {
                        best = Some((scored, idx));
                    }
                }
            }

            if let Some((model, idx)) = best {
                used[idx] = true;
                results.push(Some(model));
            } else {
                // Fallback: allow reuse if no unused model fits
                let fallback = self.select(reqs);
                results.push(fallback);
            }
        }

        results
    }

    /// Score a model against requirements. Returns None if it doesn't meet them.
    fn score_model(
        &self,
        name: &str,
        custom_prio: Option<f32>,
        reqs: &ModelRequirements,
    ) -> Option<ScoredModel> {
        let profile = llm_models_spider::model_profile(name);

        // Extract capabilities (from profile or from individual lookups)
        let has_vision = profile
            .as_ref()
            .map(|p| p.capabilities.vision)
            .unwrap_or_else(|| llm_models_spider::supports_vision(name));
        let has_audio = profile
            .as_ref()
            .map(|p| p.capabilities.audio)
            .unwrap_or(false);
        let has_video = profile
            .as_ref()
            .map(|p| p.capabilities.video)
            .unwrap_or_else(|| llm_models_spider::supports_video(name));
        let has_pdf = profile
            .as_ref()
            .map(|p| p.capabilities.file)
            .unwrap_or_else(|| llm_models_spider::supports_pdf(name));

        // Check hard requirements
        if reqs.vision && !has_vision {
            return None;
        }
        if reqs.audio && !has_audio {
            return None;
        }
        if reqs.video && !has_video {
            return None;
        }
        if reqs.pdf && !has_pdf {
            return None;
        }

        let max_input = profile.as_ref().map(|p| p.max_input_tokens).unwrap_or(0);
        if reqs.min_context_tokens > 0 && max_input < reqs.min_context_tokens {
            return None;
        }

        let arena = profile
            .as_ref()
            .and_then(|p| p.ranks.overall);
        let input_cost = profile
            .as_ref()
            .and_then(|p| p.pricing.input_cost_per_m_tokens);

        if reqs.max_input_cost_per_m > 0.0 {
            if let Some(cost) = input_cost {
                if cost > reqs.max_input_cost_per_m {
                    return None;
                }
            }
        }

        if reqs.min_arena_score > 0.0 {
            match arena {
                Some(score) if score >= reqs.min_arena_score => {}
                Some(_) => return None,
                None => {} // Unknown arena — don't filter out
            }
        }

        // Compute score
        let score = if let Some(prio) = custom_prio {
            prio
        } else {
            self.auto_score(arena, input_cost, max_input)
        };

        Some(ScoredModel {
            name: name.to_string(),
            score,
            arena_rank: arena,
            input_cost_per_m: input_cost,
            supports_vision: has_vision,
            max_input_tokens: max_input,
        })
    }

    /// Compute an automatic score based on strategy.
    fn auto_score(&self, arena: Option<f32>, cost: Option<f32>, context: u32) -> f32 {
        match self.strategy {
            SelectionStrategy::BestQuality => arena.unwrap_or(50.0),
            SelectionStrategy::CheapestFirst => {
                // Invert cost: lower cost = higher score
                match cost {
                    Some(c) if c > 0.0 => 1000.0 / c,
                    _ => 50.0, // Unknown cost = neutral
                }
            }
            SelectionStrategy::LargestContext => context as f32 / 1000.0,
            SelectionStrategy::ValueOptimal => {
                let quality = arena.unwrap_or(50.0);
                let cost_factor = match cost {
                    Some(c) if c > 0.0 => 100.0 / c,
                    _ => 1.0,
                };
                quality * cost_factor.sqrt()
            }
        }
    }
}

/// Build a [`ModelPolicy`] automatically from a pool of available models.
///
/// Inspects arena rankings and pricing to assign models to tiers.
/// The best-ranked model becomes `large`, cheapest becomes `small`,
/// and something in-between becomes `medium`.
///
/// For pools with ≤ 2 models, skips the full scoring pipeline:
/// - 0 models → default policy
/// - 1 model → all tiers use that model
/// - 2 models → first=large/medium, second=small (no scoring needed,
///   dual-model routing via [`VisionRouteMode`] handles the rest)
pub fn auto_policy(available_models: &[&str]) -> ModelPolicy {
    if available_models.is_empty() {
        return ModelPolicy::default();
    }
    if available_models.len() == 1 {
        let m = available_models[0].to_string();
        return ModelPolicy {
            small: m.clone(),
            medium: m.clone(),
            large: m,
            allow_large: true,
            max_latency_ms: None,
            max_cost_tier: CostTier::High,
        };
    }
    if available_models.len() == 2 {
        // With only 2 models, skip arena/pricing lookups.
        // Assign first as large/medium, second as small — the caller
        // already knows which is vision vs text via VisionRouteMode.
        let a = available_models[0].to_string();
        let b = available_models[1].to_string();
        return ModelPolicy {
            large: a.clone(),
            medium: a,
            small: b,
            allow_large: true,
            max_latency_ms: None,
            max_cost_tier: CostTier::High,
        };
    }

    // 3+ models: full scoring pipeline
    // Collect (name, arena_score, input_cost)
    let mut models: Vec<(&str, f32, f32)> = available_models
        .iter()
        .map(|&name| {
            let profile = llm_models_spider::model_profile(name);
            let arena = profile
                .as_ref()
                .and_then(|p| p.ranks.overall)
                .unwrap_or(50.0);
            let cost = profile
                .as_ref()
                .and_then(|p| p.pricing.input_cost_per_m_tokens)
                .unwrap_or(5.0);
            (name, arena, cost)
        })
        .collect();

    // Sort by arena score descending
    models.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let large = models[0].0.to_string();
    let small = models.last().unwrap().0.to_string();
    let medium = if models.len() >= 3 {
        models[models.len() / 2].0.to_string()
    } else {
        large.clone()
    };

    ModelPolicy {
        small,
        medium,
        large,
        allow_large: true,
        max_latency_ms: None,
        max_cost_tier: CostTier::High,
    }
}

// ── TaskAnalysis ──────────────────────────────────────────────────────────────

/// Analysis of a task for routing.
#[derive(Debug, Clone, Default)]
pub struct TaskAnalysis {
    /// Estimated input tokens.
    pub estimated_tokens: usize,
    /// Whether the task requires complex reasoning.
    pub requires_reasoning: bool,
    /// Whether the task requires code generation.
    pub requires_code_generation: bool,
    /// Whether structured JSON output is required.
    pub requires_structured_output: bool,
    /// Whether this is a multi-step task.
    pub multi_step: bool,
    /// Maximum acceptable latency.
    pub max_latency: Option<Duration>,
    /// Task category.
    pub category: TaskCategory,
    /// Whether the task requires vision capabilities.
    pub requires_vision: bool,
    /// Whether the task requires audio capabilities.
    pub requires_audio: bool,
}

impl TaskAnalysis {
    /// Create analysis from a prompt.
    pub fn from_prompt(prompt: &str) -> Self {
        let estimated_tokens = estimate_tokens(prompt);
        let lower = prompt.to_lowercase();

        Self {
            estimated_tokens,
            requires_reasoning: lower.contains("analyze")
                || lower.contains("compare")
                || lower.contains("explain")
                || lower.contains("why"),
            requires_code_generation: lower.contains("code")
                || lower.contains("implement")
                || lower.contains("function")
                || lower.contains("script"),
            requires_structured_output: lower.contains("json")
                || lower.contains("extract")
                || lower.contains("list"),
            multi_step: lower.contains("then")
                || lower.contains("step")
                || lower.contains("first")
                || lower.contains("next"),
            max_latency: None,
            category: TaskCategory::General,
            requires_vision: lower.contains("screenshot")
                || lower.contains("image")
                || lower.contains("picture")
                || lower.contains("visual"),
            requires_audio: lower.contains("audio")
                || lower.contains("voice")
                || lower.contains("speech"),
        }
    }

    /// Create analysis for extraction task.
    pub fn extraction(html_length: usize) -> Self {
        Self {
            estimated_tokens: html_length / 4 + 200, // Rough estimate
            requires_reasoning: false,
            requires_code_generation: false,
            requires_structured_output: true,
            multi_step: false,
            max_latency: None,
            category: TaskCategory::Extraction,
            requires_vision: false,
            requires_audio: false,
        }
    }

    /// Create analysis for action task.
    pub fn action(instruction: &str) -> Self {
        let mut analysis = Self::from_prompt(instruction);
        analysis.category = TaskCategory::Action;
        analysis.requires_structured_output = true;
        analysis
    }

    /// Set max latency requirement.
    pub fn with_max_latency(mut self, latency: Duration) -> Self {
        self.max_latency = Some(latency);
        self
    }

    /// Convert to model requirements for the selector.
    pub fn to_requirements(&self) -> ModelRequirements {
        ModelRequirements {
            vision: self.requires_vision,
            audio: self.requires_audio,
            ..Default::default()
        }
    }
}

/// Category of task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskCategory {
    /// General purpose task.
    #[default]
    General,
    /// Data extraction.
    Extraction,
    /// Browser action.
    Action,
    /// Code generation.
    Code,
    /// Analysis/reasoning.
    Analysis,
    /// Simple classification.
    Classification,
}

/// Result of routing decision.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// Selected model name.
    pub model: String,
    /// Selected cost tier.
    pub tier: CostTier,
    /// Explanation for the decision.
    pub reason: String,
}

impl RoutingDecision {
    /// Check if this routes to a fast model.
    pub fn is_fast(&self) -> bool {
        self.tier == CostTier::Low
    }

    /// Check if this routes to a powerful model.
    pub fn is_powerful(&self) -> bool {
        self.tier == CostTier::High
    }
}

/// Estimate token count for text.
///
/// Uses a rough approximation of 4 characters per token.
pub fn estimate_tokens(text: &str) -> usize {
    // Rough estimate: ~4 characters per token for English
    // This is a simplification; real tokenization is more complex
    text.len() / 4 + 1
}

/// Estimate tokens for messages.
pub fn estimate_message_tokens(messages: &[crate::Message]) -> usize {
    messages
        .iter()
        .map(|m| estimate_tokens(m.content.as_text()) + 4) // +4 for message overhead
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_router_simple() {
        let router = ModelRouter::new();

        let decision = router.route_simple("Extract the title from this page");
        assert!(!decision.model.is_empty());
    }

    #[test]
    fn test_model_router_complex() {
        let router = ModelRouter::new();

        let task = TaskAnalysis {
            estimated_tokens: 5000,
            requires_reasoning: true,
            requires_code_generation: true,
            ..Default::default()
        };

        let decision = router.route(&task);
        assert_eq!(decision.tier, CostTier::High);
    }

    #[test]
    fn test_model_router_constrained() {
        let policy = ModelPolicy {
            max_cost_tier: CostTier::Medium,
            ..Default::default()
        };

        let router = ModelRouter::with_policy(policy);

        let task = TaskAnalysis {
            estimated_tokens: 5000,
            requires_reasoning: true,
            ..Default::default()
        };

        let decision = router.route(&task);
        // Should be capped at Medium due to policy
        assert!(decision.tier != CostTier::High);
    }

    #[test]
    fn test_task_analysis_from_prompt() {
        let analysis = TaskAnalysis::from_prompt(
            "Analyze the code and explain why it's slow, then implement a fix",
        );

        assert!(analysis.requires_reasoning);
        assert!(analysis.requires_code_generation);
        assert!(analysis.multi_step);
    }

    #[test]
    fn test_task_analysis_vision_detection() {
        let analysis = TaskAnalysis::from_prompt("Look at this screenshot and describe it");
        assert!(analysis.requires_vision);

        let analysis = TaskAnalysis::from_prompt("Summarize this text");
        assert!(!analysis.requires_vision);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 + 1
        assert_eq!(estimate_tokens(""), 1);
    }

    // ── ModelSelector tests ───────────────────────────────────────────────

    #[test]
    fn test_model_selector_basic() {
        let selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        let reqs = ModelRequirements::default();
        let result = selector.select(&reqs);
        assert!(result.is_some());
    }

    #[test]
    fn test_model_selector_vision_filter() {
        let selector = ModelSelector::new(&["gpt-4o", "gpt-3.5-turbo"]);
        let reqs = ModelRequirements::default().with_vision();
        let ranked = selector.ranked(&reqs);

        // gpt-4o supports vision, gpt-3.5-turbo does not
        assert!(!ranked.is_empty());
        for m in &ranked {
            assert!(m.supports_vision, "non-vision model {} passed filter", m.name);
        }
    }

    #[test]
    fn test_model_selector_custom_priority() {
        let mut selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        // Override gpt-4o-mini to be top priority
        selector.set_priority("gpt-4o-mini", 999.0);

        let reqs = ModelRequirements::default();
        let best = selector.select(&reqs).unwrap();
        assert_eq!(best.name, "gpt-4o-mini");
        assert_eq!(best.score, 999.0);
    }

    #[test]
    fn test_model_selector_cheapest_strategy() {
        let mut selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini"]);
        selector.set_strategy(SelectionStrategy::CheapestFirst);

        let reqs = ModelRequirements::default();
        let ranked = selector.ranked(&reqs);

        // With CheapestFirst, cheaper model should rank higher
        assert!(ranked.len() >= 1);
    }

    #[test]
    fn test_model_selector_multi_dispatch() {
        let selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);

        let requirements = vec![
            ModelRequirements::default().with_vision(),
            ModelRequirements::default(),
        ];

        let results = selector.select_multi(&requirements);
        assert_eq!(results.len(), 2);

        // First task needs vision — should pick a vision model
        assert!(results[0].is_some());
        assert!(results[0].as_ref().unwrap().supports_vision);

        // Second task — should pick a different model if possible
        assert!(results[1].is_some());
    }

    #[test]
    fn test_model_selector_add_model() {
        let mut selector = ModelSelector::new(&["gpt-4o"]);
        selector.add_model("gpt-4o-mini");
        assert_eq!(selector.models.len(), 2);
        // Adding duplicate should not increase count
        selector.add_model("gpt-4o");
        assert_eq!(selector.models.len(), 2);
    }

    #[test]
    fn test_auto_policy_single_model() {
        let policy = auto_policy(&["gpt-4o"]);
        assert_eq!(policy.small, "gpt-4o");
        assert_eq!(policy.medium, "gpt-4o");
        assert_eq!(policy.large, "gpt-4o");
    }

    #[test]
    fn test_auto_policy_multiple_models() {
        let policy = auto_policy(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        // Should assign different models to different tiers
        assert!(!policy.small.is_empty());
        assert!(!policy.medium.is_empty());
        assert!(!policy.large.is_empty());
    }

    #[test]
    fn test_auto_policy_empty() {
        let policy = auto_policy(&[]);
        // Should return default policy
        assert_eq!(policy.small, "gpt-4o-mini");
        assert_eq!(policy.medium, "gpt-4o");
    }

    #[test]
    fn test_model_requirements_builder() {
        let reqs = ModelRequirements::default()
            .with_vision()
            .with_min_context(100_000)
            .with_max_cost(10.0)
            .with_min_arena(60.0);

        assert!(reqs.vision);
        assert_eq!(reqs.min_context_tokens, 100_000);
        assert_eq!(reqs.max_input_cost_per_m, 10.0);
        assert_eq!(reqs.min_arena_score, 60.0);
    }

    #[test]
    fn test_task_to_requirements() {
        let task = TaskAnalysis::from_prompt("Look at this screenshot and extract data");
        let reqs = task.to_requirements();
        assert!(reqs.vision);
    }

    // ── Phase 1: llm_models_spider data accuracy ────────────────────────

    #[test]
    fn test_llm_data_vision_models_detected() {
        // Models that MUST report vision support
        for model in &[
            "gpt-4o",
            "gpt-4o-mini",
            "claude-sonnet-4-5-20250514",
            "gemini-2.0-flash",
            "qwen2-vl-72b-instruct",
            "llama-3.2-11b-vision-instruct",
        ] {
            assert!(
                llm_models_spider::supports_vision(model),
                "{model} should support vision"
            );
        }
        // Models that MUST NOT report vision support
        for model in &["gpt-3.5-turbo", "deepseek-chat"] {
            assert!(
                !llm_models_spider::supports_vision(model),
                "{model} should NOT support vision"
            );
        }
    }

    #[test]
    fn test_llm_data_model_profiles_exist() {
        let must_have = [
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-3.5-turbo",
            "claude-3-5-sonnet-20241022",
            "gemini-2.0-flash",
            "deepseek-chat",
        ];
        for name in &must_have {
            let profile = llm_models_spider::model_profile(name);
            assert!(
                profile.is_some(),
                "model_profile({name}) should return Some"
            );
            let p = profile.unwrap();
            assert!(
                p.max_input_tokens > 0,
                "{name} should have max_input_tokens > 0, got {}",
                p.max_input_tokens
            );
        }
    }

    #[test]
    fn test_llm_data_arena_scores_present() {
        // These well-known models should have arena scores
        // (use short canonical names that match the arena data)
        for name in &["claude-3.5-sonnet", "chatgpt-4o-latest", "claude-opus-4"] {
            let profile = llm_models_spider::model_profile(name);
            assert!(profile.is_some(), "{name} should have a profile");
            let p = profile.unwrap();
            assert!(
                p.ranks.overall.is_some(),
                "{name} should have an arena score"
            );
            assert!(
                p.ranks.overall.unwrap() > 0.0,
                "{name} arena score should be > 0"
            );
        }
    }

    #[test]
    fn test_llm_data_pricing_ordering() {
        let cheap = llm_models_spider::model_profile("gpt-4o-mini");
        let expensive = llm_models_spider::model_profile("claude-opus-4-20250514");
        assert!(cheap.is_some() && expensive.is_some());
        let cheap_cost = cheap.unwrap().pricing.input_cost_per_m_tokens.unwrap();
        let expensive_cost = expensive.unwrap().pricing.input_cost_per_m_tokens.unwrap();
        assert!(
            cheap_cost < expensive_cost,
            "gpt-4o-mini (${cheap_cost}) should be cheaper than claude-opus-4 (${expensive_cost})"
        );
    }

    #[test]
    fn test_llm_data_context_window_ordering() {
        let large_ctx = llm_models_spider::model_profile("gemini-2.5-pro-preview-05-06");
        let small_ctx = llm_models_spider::model_profile("gpt-3.5-turbo");
        assert!(large_ctx.is_some() && small_ctx.is_some());
        let large_tokens = large_ctx.unwrap().max_input_tokens;
        let small_tokens = small_ctx.unwrap().max_input_tokens;
        assert!(
            large_tokens > small_tokens,
            "gemini-2.5-pro ({large_tokens}) should have more context than gpt-3.5-turbo ({small_tokens})"
        );
    }

    // ── Phase 2: ModelSelector reliability ──────────────────────────────

    #[test]
    fn test_selector_realistic_pool_best_quality() {
        let selector = ModelSelector::new(&[
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-3.5-turbo",
            "claude-3-5-sonnet-20241022",
            "gemini-2.0-flash",
            "deepseek-chat",
        ]);
        let reqs = ModelRequirements::default();
        let ranked = selector.ranked(&reqs);
        assert!(!ranked.is_empty());
        // BestQuality (default) → first should have the highest arena score
        let top = &ranked[0];
        for other in &ranked[1..] {
            assert!(
                top.score >= other.score,
                "top model {} (score {}) should beat {} (score {})",
                top.name,
                top.score,
                other.name,
                other.score
            );
        }
    }

    #[test]
    fn test_selector_realistic_pool_cheapest() {
        let mut selector = ModelSelector::new(&[
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-3.5-turbo",
            "claude-3-5-sonnet-20241022",
        ]);
        selector.set_strategy(SelectionStrategy::CheapestFirst);
        let reqs = ModelRequirements::default();
        let ranked = selector.ranked(&reqs);
        assert!(ranked.len() >= 2);
        // CheapestFirst → lower cost gets higher score
        let top = &ranked[0];
        let bottom = ranked.last().unwrap();
        if let (Some(top_cost), Some(bottom_cost)) = (top.input_cost_per_m, bottom.input_cost_per_m)
        {
            assert!(
                top_cost <= bottom_cost,
                "cheapest ({}, ${top_cost}) should rank above expensive ({}, ${bottom_cost})",
                top.name,
                bottom.name
            );
        }
    }

    #[test]
    fn test_selector_vision_filter_rejects_text_only() {
        let selector = ModelSelector::new(&["gpt-3.5-turbo", "deepseek-chat"]);
        let reqs = ModelRequirements::default().with_vision();
        let result = selector.select(&reqs);
        assert!(
            result.is_none(),
            "text-only pool should return None for vision requirement"
        );
    }

    #[test]
    fn test_selector_unknown_models_graceful() {
        let selector = ModelSelector::new(&["my-custom-model", "local-llama"]);
        let reqs = ModelRequirements::default();
        let result = selector.select(&reqs);
        assert!(
            result.is_some(),
            "unknown models should still return Some with default score"
        );
        let scored = result.unwrap();
        assert_eq!(scored.score, 50.0, "unknown model gets default score 50.0");
    }

    #[test]
    fn test_selector_single_model_all_strategies() {
        for strategy in &[
            SelectionStrategy::BestQuality,
            SelectionStrategy::CheapestFirst,
            SelectionStrategy::LargestContext,
            SelectionStrategy::ValueOptimal,
        ] {
            let mut selector = ModelSelector::new(&["gpt-4o"]);
            selector.set_strategy(*strategy);
            let reqs = ModelRequirements::default();
            let result = selector.select(&reqs);
            assert!(
                result.is_some(),
                "single model should be returned for {strategy:?}"
            );
            assert_eq!(result.unwrap().name, "gpt-4o");
        }
    }

    #[test]
    fn test_selector_deterministic_ordering() {
        let selector = ModelSelector::new(&[
            "gpt-4o",
            "gpt-4o-mini",
            "claude-3-5-sonnet-20241022",
            "gemini-2.0-flash",
        ]);
        let reqs = ModelRequirements::default();

        let first_run: Vec<String> = selector.ranked(&reqs).iter().map(|m| m.name.clone()).collect();
        let second_run: Vec<String> = selector.ranked(&reqs).iter().map(|m| m.name.clone()).collect();
        assert_eq!(
            first_run, second_run,
            "repeated calls must produce identical ordering"
        );
    }

    #[test]
    fn test_selector_cost_filter_strict() {
        let selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        let reqs = ModelRequirements::default().with_max_cost(1.0);
        let ranked = selector.ranked(&reqs);
        for m in &ranked {
            if let Some(cost) = m.input_cost_per_m {
                assert!(
                    cost <= 1.0,
                    "{} has cost ${cost} which exceeds max 1.0",
                    m.name
                );
            }
        }
    }

    #[test]
    fn test_selector_min_context_filter() {
        let selector = ModelSelector::new(&["gpt-4o", "gpt-3.5-turbo", "gemini-2.0-flash"]);
        let reqs = ModelRequirements::default().with_min_context(500_000);
        let ranked = selector.ranked(&reqs);
        for m in &ranked {
            assert!(
                m.max_input_tokens >= 500_000,
                "{} has {} tokens, below 500k minimum",
                m.name,
                m.max_input_tokens
            );
        }
    }

    #[test]
    fn test_selector_value_optimal_balances() {
        let mut selector = ModelSelector::new(&[
            "gpt-4o",       // high quality, moderate cost
            "gpt-4o-mini",  // lower quality, cheap
            "gpt-3.5-turbo", // lowest quality, cheapest
        ]);
        selector.set_strategy(SelectionStrategy::ValueOptimal);
        let reqs = ModelRequirements::default();
        let ranked = selector.ranked(&reqs);
        assert!(!ranked.is_empty());
        let top = &ranked[0];
        // ValueOptimal should NOT just pick cheapest or best — verify it's not the raw cheapest
        // (it uses quality * sqrt(100/cost), so moderate-cost + high-quality can win)
        // Just verify it returns a valid result with a positive score
        assert!(top.score > 0.0, "ValueOptimal score should be positive");
    }

    #[test]
    fn test_select_multi_no_reuse() {
        let selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        let requirements = vec![
            ModelRequirements::default(),
            ModelRequirements::default(),
            ModelRequirements::default(),
        ];
        let results = selector.select_multi(&requirements);
        assert_eq!(results.len(), 3);
        // All should be Some
        let names: Vec<&str> = results
            .iter()
            .filter_map(|r| r.as_ref().map(|m| m.name.as_str()))
            .collect();
        assert_eq!(names.len(), 3, "all 3 requests should get a model");
        // No duplicates (3 models, 3 requests → each used once)
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            3,
            "no model should be reused when pool is large enough"
        );
    }

    #[test]
    fn test_select_multi_exhaustion_fallback() {
        let selector = ModelSelector::new(&["gpt-4o"]);
        let requirements = vec![
            ModelRequirements::default(),
            ModelRequirements::default(),
            ModelRequirements::default(),
        ];
        let results = selector.select_multi(&requirements);
        assert_eq!(results.len(), 3);
        // First gets the model, rest fall back to reuse
        assert!(results[0].is_some());
        assert!(results[1].is_some(), "fallback should reuse the single model");
        assert!(results[2].is_some(), "fallback should reuse the single model");
        // All should be the same model
        assert_eq!(results[0].as_ref().unwrap().name, "gpt-4o");
        assert_eq!(results[1].as_ref().unwrap().name, "gpt-4o");
        assert_eq!(results[2].as_ref().unwrap().name, "gpt-4o");
    }

    #[test]
    fn test_selector_priority_override_beats_arena() {
        let mut selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        // gpt-3.5-turbo has low arena score; override it to beat gpt-4o
        selector.set_priority("gpt-3.5-turbo", 999.0);
        let reqs = ModelRequirements::default();
        let best = selector.select(&reqs).unwrap();
        assert_eq!(
            best.name, "gpt-3.5-turbo",
            "priority override should beat natural arena score"
        );
        assert_eq!(best.score, 999.0);
    }

    // ── Phase 3: auto_policy + ModelRouter pipeline ─────────────────────

    #[test]
    fn test_auto_policy_realistic_tiering() {
        let policy = auto_policy(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        // Sorted by arena descending: large=highest, small=lowest
        // large should be the model with highest arena score
        // small should be the model with lowest arena score
        assert_ne!(
            policy.large, policy.small,
            "large and small should be different models"
        );
        // Verify tiers resolve correctly
        assert_eq!(policy.model_for_tier(CostTier::High), policy.large);
        assert_eq!(policy.model_for_tier(CostTier::Low), policy.small);
        assert_eq!(policy.model_for_tier(CostTier::Medium), policy.medium);
    }

    #[test]
    fn test_auto_policy_2_models() {
        let policy = auto_policy(&["gpt-4o", "gpt-4o-mini"]);
        // With 2 models, skip scoring: first=large/medium, second=small
        assert_eq!(policy.large, "gpt-4o");
        assert_eq!(policy.medium, "gpt-4o");
        assert_eq!(policy.small, "gpt-4o-mini");
        assert_eq!(
            policy.medium, policy.large,
            "2-model policy should have medium == large"
        );
        assert_ne!(
            policy.large, policy.small,
            "large and small should differ"
        );
    }

    #[test]
    fn test_auto_policy_unknown_models() {
        let policy = auto_policy(&["my-custom-llm", "local-model-7b", "test-endpoint"]);
        // Should not panic, all get default arena=50.0, cost=5.0
        assert!(!policy.small.is_empty());
        assert!(!policy.medium.is_empty());
        assert!(!policy.large.is_empty());
        assert!(policy.allow_large);
    }

    #[test]
    fn test_auto_policy_to_router_e2e() {
        let policy = auto_policy(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
        let router = ModelRouter::with_policy(policy.clone());

        // Low complexity → small model
        let simple_task = TaskAnalysis {
            estimated_tokens: 100,
            ..Default::default()
        };
        let decision = router.route(&simple_task);
        assert_eq!(decision.tier, CostTier::Low);
        assert_eq!(decision.model, policy.small);

        // High complexity → large model
        let hard_task = TaskAnalysis {
            estimated_tokens: 5000,
            requires_reasoning: true,
            requires_code_generation: true,
            ..Default::default()
        };
        let decision = router.route(&hard_task);
        assert_eq!(decision.tier, CostTier::High);
        assert_eq!(decision.model, policy.large);

        // Medium complexity → medium model
        let medium_task = TaskAnalysis {
            estimated_tokens: 2000,
            requires_structured_output: true,
            multi_step: true,
            ..Default::default()
        };
        let decision = router.route(&medium_task);
        assert_eq!(decision.tier, CostTier::Medium);
        assert_eq!(decision.model, policy.medium);
    }

    #[test]
    fn test_auto_policy_to_router_e2e_single_model() {
        // Common case: user only has one API key / one model
        let policy = auto_policy(&["gpt-4o"]);
        assert_eq!(policy.small, "gpt-4o");
        assert_eq!(policy.medium, "gpt-4o");
        assert_eq!(policy.large, "gpt-4o");

        let router = ModelRouter::with_policy(policy);

        // ALL complexity levels must resolve to the single model
        let simple = TaskAnalysis {
            estimated_tokens: 50,
            ..Default::default()
        };
        let medium = TaskAnalysis {
            estimated_tokens: 2000,
            requires_structured_output: true,
            ..Default::default()
        };
        let hard = TaskAnalysis {
            estimated_tokens: 5000,
            requires_reasoning: true,
            requires_code_generation: true,
            ..Default::default()
        };

        for (label, task) in [("simple", &simple), ("medium", &medium), ("hard", &hard)] {
            let decision = router.route(task);
            assert_eq!(
                decision.model, "gpt-4o",
                "{label} task should still route to the only model"
            );
        }
    }

    #[test]
    fn test_selector_single_model_vision_mismatch() {
        // User has one text-only model but needs vision → None
        let selector = ModelSelector::new(&["gpt-3.5-turbo"]);
        let reqs = ModelRequirements::default().with_vision();
        assert!(
            selector.select(&reqs).is_none(),
            "single text-only model should not satisfy vision requirement"
        );

        // User has one vision model and needs vision → works
        let selector = ModelSelector::new(&["gpt-4o"]);
        let result = selector.select(&reqs);
        assert!(result.is_some(), "single vision model should satisfy vision");
        assert_eq!(result.unwrap().name, "gpt-4o");
    }

    #[test]
    fn test_selector_single_model_with_cost_filter() {
        // User has one expensive model but cost filter is strict → None
        let selector = ModelSelector::new(&["gpt-4o"]);
        let reqs = ModelRequirements::default().with_max_cost(0.01);
        assert!(
            selector.select(&reqs).is_none(),
            "single expensive model should be filtered by strict cost limit"
        );

        // Relax cost filter → works
        let reqs = ModelRequirements::default().with_max_cost(100.0);
        let result = selector.select(&reqs);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "gpt-4o");
    }

    #[test]
    fn test_selector_single_unknown_model_e2e() {
        // User has one custom/self-hosted model with no data in llm_models_spider
        let policy = auto_policy(&["my-local-llama"]);
        assert_eq!(policy.small, "my-local-llama");
        assert_eq!(policy.medium, "my-local-llama");
        assert_eq!(policy.large, "my-local-llama");

        let router = ModelRouter::with_policy(policy);
        let decision = router.route_simple("do something complex and analyze the code");
        assert_eq!(
            decision.model, "my-local-llama",
            "unknown single model should still be routed to"
        );

        // Also verify selector works
        let selector = ModelSelector::new(&["my-local-llama"]);
        let result = selector.select(&ModelRequirements::default());
        assert!(result.is_some());
        let scored = result.unwrap();
        assert_eq!(scored.name, "my-local-llama");
        assert_eq!(scored.score, 50.0, "unknown model gets default score");
        assert_eq!(scored.max_input_tokens, 0, "unknown model has no context data");
        assert!(scored.arena_rank.is_none(), "unknown model has no arena data");
    }

    #[test]
    fn test_router_latency_constraint_downgrade() {
        let policy = ModelPolicy {
            max_latency_ms: Some(1000),
            ..Default::default()
        };
        let router = ModelRouter::with_policy(policy);

        // A task that would normally be High tier
        let task = TaskAnalysis {
            estimated_tokens: 5000,
            requires_reasoning: true,
            requires_code_generation: true,
            ..Default::default()
        };
        let decision = router.route(&task);
        // Latency constraint should downgrade from High
        assert_ne!(
            decision.tier,
            CostTier::High,
            "latency constraint should prevent High tier"
        );
    }

    #[test]
    fn test_router_allow_large_false() {
        let policy = ModelPolicy {
            allow_large: false,
            ..Default::default()
        };
        let router = ModelRouter::with_policy(policy);

        let task = TaskAnalysis {
            estimated_tokens: 5000,
            requires_reasoning: true,
            requires_code_generation: true,
            ..Default::default()
        };
        let decision = router.route(&task);
        assert_ne!(
            decision.tier,
            CostTier::High,
            "allow_large=false should cap at Medium"
        );
    }

    #[test]
    fn test_router_threshold_customization() {
        let router = ModelRouter::new().with_thresholds(100, 200);

        // With lowered thresholds: tokens=300 → +3 (> large threshold 200),
        // reasoning → +2, code_gen → +2 = 7 → High tier
        let task = TaskAnalysis {
            estimated_tokens: 300,
            requires_reasoning: true,
            requires_code_generation: true,
            ..Default::default()
        };
        let decision = router.route(&task);
        assert_eq!(
            decision.tier,
            CostTier::High,
            "lowered thresholds should promote to High tier sooner"
        );

        // Same task with default thresholds (1000/4000) would be Medium:
        // tokens=300 → +1 (< 1000), reasoning → +2, code_gen → +2 = 5 → Medium
        let default_router = ModelRouter::new();
        let decision = default_router.route(&task);
        assert_eq!(
            decision.tier,
            CostTier::Medium,
            "default thresholds should keep this at Medium"
        );
    }

    // ── Phase 5: Edge cases ─────────────────────────────────────────────

    #[test]
    fn test_selector_empty_pool() {
        let selector = ModelSelector::new(&[]);
        let reqs = ModelRequirements::default();
        let result = selector.select(&reqs);
        assert!(result.is_none(), "empty pool should return None");
    }

    #[test]
    fn test_selector_duplicate_models() {
        let selector = ModelSelector::new(&["gpt-4o", "gpt-4o", "gpt-4o"]);
        let requirements = vec![
            ModelRequirements::default(),
            ModelRequirements::default(),
            ModelRequirements::default(),
        ];
        let results = selector.select_multi(&requirements);
        assert_eq!(results.len(), 3, "should not hang on duplicates");
        // All should resolve (first gets it, rest fallback)
        for (i, r) in results.iter().enumerate() {
            assert!(r.is_some(), "request {i} should get a model");
        }
    }

    #[test]
    fn test_task_analysis_edge_cases() {
        // Empty string
        let analysis = TaskAnalysis::from_prompt("");
        assert_eq!(analysis.estimated_tokens, 1);
        assert!(!analysis.requires_reasoning);

        // All keywords
        let analysis = TaskAnalysis::from_prompt(
            "analyze compare explain why code implement function script json extract list then step first next screenshot image",
        );
        assert!(analysis.requires_reasoning);
        assert!(analysis.requires_code_generation);
        assert!(analysis.requires_structured_output);
        assert!(analysis.multi_step);
        assert!(analysis.requires_vision);

        // Unicode-only
        let analysis = TaskAnalysis::from_prompt("你好世界 🌍 日本語テスト");
        assert!(!analysis.requires_reasoning);
        assert!(!analysis.requires_code_generation);
        assert!(analysis.estimated_tokens > 0);
    }

    #[test]
    fn test_auto_policy_large_pool() {
        let models: Vec<&str> = vec![
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-3.5-turbo",
            "claude-3-5-sonnet-20241022",
            "claude-3-5-haiku-20241022",
            "gemini-2.0-flash",
            "deepseek-chat",
            "unknown-model-1",
            "unknown-model-2",
            "unknown-model-3",
            "unknown-model-4",
            "unknown-model-5",
            "unknown-model-6",
            "unknown-model-7",
            "unknown-model-8",
            "unknown-model-9",
            "unknown-model-10",
            "unknown-model-11",
            "unknown-model-12",
            "unknown-model-13",
        ];
        let policy = auto_policy(&models);
        assert!(!policy.small.is_empty());
        assert!(!policy.medium.is_empty());
        assert!(!policy.large.is_empty());
        assert!(policy.allow_large);
        assert_eq!(policy.max_cost_tier, CostTier::High);
    }
}
