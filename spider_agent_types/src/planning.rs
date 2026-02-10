//! Multi-step planning mode for LLM automation.
//!
//! This module enables Claude to plan multiple steps upfront, reducing LLM round-trips.
//! Plans include checkpoints for verification and support re-planning on failure.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for planning mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanningModeConfig {
    /// Enable planning mode.
    pub enabled: bool,
    /// Maximum steps to plan at once.
    pub max_planned_steps: usize,
    /// Whether to re-plan on checkpoint failure.
    pub replan_on_failure: bool,
    /// Maximum re-plan attempts.
    pub max_replan_attempts: usize,
    /// Whether to include reasoning in plans.
    pub include_reasoning: bool,
    /// Minimum confidence to execute without checkpoints.
    pub auto_execute_threshold: f64,
}

impl Default for PlanningModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_planned_steps: 10,
            replan_on_failure: true,
            max_replan_attempts: 3,
            include_reasoning: true,
            auto_execute_threshold: 0.8,
        }
    }
}

impl PlanningModeConfig {
    /// Create a new config with planning enabled.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Set max planned steps.
    pub fn with_max_steps(mut self, max: usize) -> Self {
        self.max_planned_steps = max;
        self
    }

    /// Set re-plan behavior.
    pub fn with_replan(mut self, enabled: bool) -> Self {
        self.replan_on_failure = enabled;
        self
    }

    /// Set auto-execute threshold.
    pub fn with_auto_execute_threshold(mut self, threshold: f64) -> Self {
        self.auto_execute_threshold = threshold.clamp(0.0, 1.0);
        self
    }
}

/// A single step in an execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedStep {
    /// Unique identifier for this step.
    pub id: String,
    /// The action to execute.
    pub action: Value,
    /// Optional checkpoint to verify after this step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
    /// IDs of steps this one depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Description of what this step does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Confidence score (0.0 to 1.0).
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Whether this step is critical (plan fails if it fails).
    #[serde(default = "default_true")]
    pub critical: bool,
}

fn default_confidence() -> f64 {
    0.5
}

fn default_true() -> bool {
    true
}

impl PlannedStep {
    /// Create a new planned step.
    pub fn new(id: impl Into<String>, action: Value) -> Self {
        Self {
            id: id.into(),
            action,
            checkpoint: None,
            depends_on: Vec::new(),
            description: None,
            confidence: 0.5,
            critical: true,
        }
    }

    /// Add a checkpoint.
    pub fn with_checkpoint(mut self, checkpoint: Checkpoint) -> Self {
        self.checkpoint = Some(checkpoint);
        self
    }

    /// Add a dependency.
    pub fn depends_on(mut self, step_id: impl Into<String>) -> Self {
        self.depends_on.push(step_id.into());
        self
    }

    /// Add a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set confidence.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Set whether step is critical.
    pub fn with_critical(mut self, critical: bool) -> Self {
        self.critical = critical;
        self
    }

    /// Parse from LLM JSON response.
    pub fn from_json(value: &Value) -> Option<Self> {
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("step")
            .to_string();
        let action = value.get("action")?.clone();

        let checkpoint = value.get("checkpoint").and_then(Checkpoint::from_json);

        let depends_on = value
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let description = value
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);

        let confidence = value
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        let critical = value
            .get("critical")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Some(Self {
            id,
            action,
            checkpoint,
            depends_on,
            description,
            confidence: confidence.clamp(0.0, 1.0),
            critical,
        })
    }
}

/// A checkpoint condition to verify after a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Type of checkpoint.
    pub checkpoint_type: CheckpointType,
    /// Expected value or condition.
    pub expected: String,
    /// Optional timeout in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Description of what we're checking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Checkpoint {
    /// Create a URL contains checkpoint.
    pub fn url_contains(pattern: impl Into<String>) -> Self {
        Self {
            checkpoint_type: CheckpointType::UrlContains,
            expected: pattern.into(),
            timeout_ms: None,
            description: None,
        }
    }

    /// Create an element exists checkpoint.
    pub fn element_exists(selector: impl Into<String>) -> Self {
        Self {
            checkpoint_type: CheckpointType::ElementExists,
            expected: selector.into(),
            timeout_ms: Some(5000),
            description: None,
        }
    }

    /// Create a text contains checkpoint.
    pub fn text_contains(text: impl Into<String>) -> Self {
        Self {
            checkpoint_type: CheckpointType::TextContains,
            expected: text.into(),
            timeout_ms: None,
            description: None,
        }
    }

    /// Create a JavaScript condition checkpoint.
    pub fn js_condition(condition: impl Into<String>) -> Self {
        Self {
            checkpoint_type: CheckpointType::JsCondition,
            expected: condition.into(),
            timeout_ms: None,
            description: None,
        }
    }

    /// Create an element not exists checkpoint.
    pub fn element_not_exists(selector: impl Into<String>) -> Self {
        Self {
            checkpoint_type: CheckpointType::ElementNotExists,
            expected: selector.into(),
            timeout_ms: Some(5000),
            description: None,
        }
    }

    /// Set timeout.
    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Parse from JSON.
    pub fn from_json(value: &Value) -> Option<Self> {
        let type_str = value.get("type").and_then(|v| v.as_str())?;
        let expected = value.get("expected").and_then(|v| v.as_str())?.to_string();

        let checkpoint_type = match type_str {
            "url_contains" => CheckpointType::UrlContains,
            "element_exists" => CheckpointType::ElementExists,
            "element_not_exists" => CheckpointType::ElementNotExists,
            "text_contains" => CheckpointType::TextContains,
            "js_condition" => CheckpointType::JsCondition,
            "page_loaded" => CheckpointType::PageLoaded,
            _ => return None,
        };

        let timeout_ms = value.get("timeout_ms").and_then(|v| v.as_u64());
        let description = value
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);

        Some(Self {
            checkpoint_type,
            expected,
            timeout_ms,
            description,
        })
    }
}

/// Type of checkpoint verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointType {
    /// Check if URL contains a pattern.
    UrlContains,
    /// Check if an element exists.
    ElementExists,
    /// Check if an element does NOT exist.
    ElementNotExists,
    /// Check if page text contains a string.
    TextContains,
    /// Evaluate a JavaScript condition.
    JsCondition,
    /// Check if page is fully loaded.
    PageLoaded,
}

/// An execution plan from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// The goal this plan aims to achieve.
    pub goal: String,
    /// Reasoning for the plan (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Steps to execute.
    pub steps: Vec<PlannedStep>,
    /// Overall confidence in the plan.
    pub confidence: f64,
    /// Whether the plan is complete (achieves the goal).
    pub is_complete: bool,
    /// Expected final state after execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_outcome: Option<String>,
}

impl ExecutionPlan {
    /// Create a new execution plan.
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            reasoning: None,
            steps: Vec::new(),
            confidence: 0.5,
            is_complete: false,
            expected_outcome: None,
        }
    }

    /// Add a step.
    pub fn add_step(mut self, step: PlannedStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Set reasoning.
    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning = Some(reasoning.into());
        self
    }

    /// Set confidence.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Mark as complete.
    pub fn complete(mut self) -> Self {
        self.is_complete = true;
        self
    }

    /// Set expected outcome.
    pub fn with_expected_outcome(mut self, outcome: impl Into<String>) -> Self {
        self.expected_outcome = Some(outcome.into());
        self
    }

    /// Parse from LLM JSON response.
    pub fn from_json(value: &Value) -> Option<Self> {
        let goal = value
            .get("goal")
            .and_then(|v| v.as_str())
            .unwrap_or("automation")
            .to_string();

        let reasoning = value
            .get("reasoning")
            .and_then(|v| v.as_str())
            .map(String::from);

        let steps = value
            .get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(PlannedStep::from_json).collect())
            .unwrap_or_default();

        let confidence = value
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        let is_complete = value
            .get("is_complete")
            .or_else(|| value.get("done"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let expected_outcome = value
            .get("expected_outcome")
            .and_then(|v| v.as_str())
            .map(String::from);

        Some(Self {
            goal,
            reasoning,
            steps,
            confidence: confidence.clamp(0.0, 1.0),
            is_complete,
            expected_outcome,
        })
    }

    /// Get the number of steps.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Check if the plan is empty.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Get the average confidence of steps.
    pub fn average_step_confidence(&self) -> f64 {
        if self.steps.is_empty() {
            0.0
        } else {
            let sum: f64 = self.steps.iter().map(|s| s.confidence).sum();
            sum / self.steps.len() as f64
        }
    }

    /// Get steps that have checkpoints.
    pub fn steps_with_checkpoints(&self) -> Vec<&PlannedStep> {
        self.steps
            .iter()
            .filter(|s| s.checkpoint.is_some())
            .collect()
    }

    /// Get critical steps.
    pub fn critical_steps(&self) -> Vec<&PlannedStep> {
        self.steps.iter().filter(|s| s.critical).collect()
    }
}

/// State of plan execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanExecutionState {
    /// IDs of completed steps.
    pub completed_steps: Vec<String>,
    /// Failed steps with error messages.
    pub failed_steps: Vec<(String, String)>,
    /// Current step index.
    pub current_step: usize,
    /// Number of re-plan attempts made.
    pub replan_attempts: usize,
    /// Whether execution should continue.
    pub should_continue: bool,
    /// Last checkpoint result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checkpoint_result: Option<CheckpointResult>,
}

impl PlanExecutionState {
    /// Create a new execution state.
    pub fn new() -> Self {
        Self {
            completed_steps: Vec::new(),
            failed_steps: Vec::new(),
            current_step: 0,
            replan_attempts: 0,
            should_continue: true,
            last_checkpoint_result: None,
        }
    }

    /// Mark a step as completed.
    pub fn complete_step(&mut self, step_id: impl Into<String>) {
        self.completed_steps.push(step_id.into());
        self.current_step += 1;
    }

    /// Mark a step as failed.
    pub fn fail_step(&mut self, step_id: impl Into<String>, error: impl Into<String>) {
        self.failed_steps.push((step_id.into(), error.into()));
    }

    /// Check if a step has been completed.
    pub fn is_completed(&self, step_id: &str) -> bool {
        self.completed_steps.iter().any(|s| s == step_id)
    }

    /// Check if a step has failed.
    pub fn is_failed(&self, step_id: &str) -> bool {
        self.failed_steps.iter().any(|(s, _)| s == step_id)
    }

    /// Get total steps executed (completed + failed).
    pub fn steps_executed(&self) -> usize {
        self.completed_steps.len() + self.failed_steps.len()
    }

    /// Check if there are any failures.
    pub fn has_failures(&self) -> bool {
        !self.failed_steps.is_empty()
    }

    /// Get the first error, if any.
    pub fn first_error(&self) -> Option<&str> {
        self.failed_steps.first().map(|(_, e)| e.as_str())
    }

    /// Stop execution.
    pub fn stop(&mut self) {
        self.should_continue = false;
    }

    /// Increment re-plan attempts.
    pub fn increment_replan(&mut self) {
        self.replan_attempts += 1;
    }
}

/// Result of a checkpoint verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointResult {
    /// Whether the checkpoint passed.
    pub passed: bool,
    /// The checkpoint that was verified.
    pub checkpoint: Checkpoint,
    /// Actual value observed (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_value: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CheckpointResult {
    /// Create a passed result.
    pub fn passed(checkpoint: Checkpoint, duration_ms: u64) -> Self {
        Self {
            passed: true,
            checkpoint,
            actual_value: None,
            duration_ms,
            error: None,
        }
    }

    /// Create a failed result.
    pub fn failed(checkpoint: Checkpoint, error: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            passed: false,
            checkpoint,
            actual_value: None,
            duration_ms,
            error: Some(error.into()),
        }
    }

    /// Set actual value.
    pub fn with_actual(mut self, value: impl Into<String>) -> Self {
        self.actual_value = Some(value.into());
        self
    }
}

/// Context for re-planning after a failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplanContext {
    /// Original goal.
    pub goal: String,
    /// Steps that were completed.
    pub completed_steps: Vec<String>,
    /// Step that failed.
    pub failed_step: String,
    /// Error that caused the failure.
    pub error: String,
    /// Current page state (URL, title, etc.).
    pub current_state: PageState,
    /// Previous plan for reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_plan: Option<ExecutionPlan>,
}

impl ReplanContext {
    /// Create a new re-plan context.
    pub fn new(
        goal: impl Into<String>,
        failed_step: impl Into<String>,
        error: impl Into<String>,
        current_state: PageState,
    ) -> Self {
        Self {
            goal: goal.into(),
            completed_steps: Vec::new(),
            failed_step: failed_step.into(),
            error: error.into(),
            current_state,
            previous_plan: None,
        }
    }

    /// Set completed steps.
    pub fn with_completed(mut self, steps: Vec<String>) -> Self {
        self.completed_steps = steps;
        self
    }

    /// Set previous plan.
    pub fn with_previous_plan(mut self, plan: ExecutionPlan) -> Self {
        self.previous_plan = Some(plan);
        self
    }

    /// Build a prompt for re-planning.
    pub fn to_prompt(&self) -> String {
        let mut prompt = String::with_capacity(1024);

        prompt.push_str("RE-PLANNING REQUIRED\n\n");
        prompt.push_str("Goal: ");
        prompt.push_str(&self.goal);
        prompt.push_str("\n\n");

        if !self.completed_steps.is_empty() {
            prompt.push_str("Completed steps:\n");
            for step in &self.completed_steps {
                prompt.push_str("- ");
                prompt.push_str(step);
                prompt.push('\n');
            }
            prompt.push('\n');
        }

        prompt.push_str("Failed step: ");
        prompt.push_str(&self.failed_step);
        prompt.push_str("\nError: ");
        prompt.push_str(&self.error);
        prompt.push_str("\n\n");

        prompt.push_str("Current state:\n");
        prompt.push_str("- URL: ");
        prompt.push_str(&self.current_state.url);
        prompt.push_str("\n- Title: ");
        prompt.push_str(&self.current_state.title);
        prompt.push('\n');

        prompt.push_str("\nPlease create a new plan to achieve the goal from the current state.");

        prompt
    }
}

/// Current page state for re-planning context.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageState {
    /// Current URL.
    pub url: String,
    /// Current title.
    pub title: String,
    /// Optional HTML snippet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_snippet: Option<String>,
}

impl PageState {
    /// Create a new page state.
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            html_snippet: None,
        }
    }

    /// Add an HTML snippet.
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html_snippet = Some(html.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planning_mode_config() {
        let config = PlanningModeConfig::enabled()
            .with_max_steps(5)
            .with_replan(false);

        assert!(config.enabled);
        assert_eq!(config.max_planned_steps, 5);
        assert!(!config.replan_on_failure);
    }

    #[test]
    fn test_planned_step() {
        let step = PlannedStep::new("step1", serde_json::json!({"Click": "button"}))
            .with_checkpoint(Checkpoint::element_exists(".success"))
            .with_confidence(0.9)
            .depends_on("step0");

        assert_eq!(step.id, "step1");
        assert!(step.checkpoint.is_some());
        assert_eq!(step.confidence, 0.9);
        assert_eq!(step.depends_on, vec!["step0"]);
    }

    #[test]
    fn test_checkpoint_types() {
        let url_check = Checkpoint::url_contains("/dashboard");
        assert_eq!(url_check.checkpoint_type, CheckpointType::UrlContains);

        let elem_check = Checkpoint::element_exists(".modal").with_timeout(10000);
        assert_eq!(elem_check.checkpoint_type, CheckpointType::ElementExists);
        assert_eq!(elem_check.timeout_ms, Some(10000));

        let js_check = Checkpoint::js_condition("document.readyState === 'complete'");
        assert_eq!(js_check.checkpoint_type, CheckpointType::JsCondition);
    }

    #[test]
    fn test_execution_plan() {
        let plan = ExecutionPlan::new("Login to dashboard")
            .add_step(PlannedStep::new(
                "s1",
                serde_json::json!({"Fill": {"selector": "input", "value": "user"}}),
            ))
            .add_step(PlannedStep::new(
                "s2",
                serde_json::json!({"Click": "button"}),
            ))
            .with_confidence(0.85)
            .complete();

        assert_eq!(plan.goal, "Login to dashboard");
        assert_eq!(plan.step_count(), 2);
        assert!(plan.is_complete);
        assert_eq!(plan.confidence, 0.85);
    }

    #[test]
    fn test_plan_execution_state() {
        let mut state = PlanExecutionState::new();

        state.complete_step("step1");
        assert!(state.is_completed("step1"));
        assert!(!state.is_failed("step1"));
        assert_eq!(state.steps_executed(), 1);

        state.fail_step("step2", "Selector not found");
        assert!(state.is_failed("step2"));
        assert!(state.has_failures());
        assert_eq!(state.first_error(), Some("Selector not found"));
    }

    #[test]
    fn test_checkpoint_result() {
        let checkpoint = Checkpoint::url_contains("/success");

        let passed = CheckpointResult::passed(checkpoint.clone(), 100);
        assert!(passed.passed);

        let failed =
            CheckpointResult::failed(checkpoint, "URL did not match", 200).with_actual("/error");
        assert!(!failed.passed);
        assert_eq!(failed.actual_value, Some("/error".to_string()));
    }

    #[test]
    fn test_plan_parsing() {
        let json = serde_json::json!({
            "goal": "Test automation",
            "reasoning": "This plan will test the form",
            "steps": [
                {
                    "id": "s1",
                    "action": { "Click": "button" },
                    "confidence": 0.9,
                    "checkpoint": {
                        "type": "element_exists",
                        "expected": ".result"
                    }
                }
            ],
            "confidence": 0.85,
            "is_complete": true
        });

        let plan = ExecutionPlan::from_json(&json).unwrap();
        assert_eq!(plan.goal, "Test automation");
        assert_eq!(plan.step_count(), 1);
        assert!(plan.is_complete);

        let step = &plan.steps[0];
        assert_eq!(step.id, "s1");
        assert!(step.checkpoint.is_some());
    }

    #[test]
    fn test_replan_context() {
        let context = ReplanContext::new(
            "Complete checkout",
            "click_payment",
            "Payment button not found",
            PageState::new("https://shop.com/cart", "Shopping Cart"),
        )
        .with_completed(vec!["add_to_cart".to_string()]);

        let prompt = context.to_prompt();
        assert!(prompt.contains("RE-PLANNING REQUIRED"));
        assert!(prompt.contains("Complete checkout"));
        assert!(prompt.contains("Payment button not found"));
        assert!(prompt.contains("add_to_cart"));
    }

    #[test]
    fn test_average_step_confidence() {
        let plan = ExecutionPlan::new("Test")
            .add_step(PlannedStep::new("s1", serde_json::json!({})).with_confidence(0.8))
            .add_step(PlannedStep::new("s2", serde_json::json!({})).with_confidence(0.6));

        assert_eq!(plan.average_step_confidence(), 0.7);
    }

    #[test]
    fn test_critical_steps() {
        let plan = ExecutionPlan::new("Test")
            .add_step(PlannedStep::new("s1", serde_json::json!({})).with_critical(true))
            .add_step(PlannedStep::new("s2", serde_json::json!({})).with_critical(false))
            .add_step(PlannedStep::new("s3", serde_json::json!({})).with_critical(true));

        let critical = plan.critical_steps();
        assert_eq!(critical.len(), 2);
    }
}
