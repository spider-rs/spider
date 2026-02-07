//! Smart model routing for optimal performance and cost.
//!
//! Routes requests to appropriate models based on:
//! - Task complexity
//! - Token count estimates
//! - Latency requirements
//! - Cost constraints

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
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 + 1
        assert_eq!(estimate_tokens(""), 1);
    }
}
