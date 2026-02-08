//! Action chain execution with conditional logic.

use super::AutomationUsage;

/// A single step in an action chain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChainStep {
    /// The instruction to execute (natural language or action spec).
    pub instruction: String,
    /// Optional condition that must be true to execute.
    pub condition: Option<ChainCondition>,
    /// Whether to continue the chain if this step fails.
    pub continue_on_failure: bool,
    /// Optional extraction prompt after this step.
    pub extract: Option<String>,
    /// Optional timeout for this step in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Maximum retries for this step.
    pub max_retries: Option<usize>,
}

impl ChainStep {
    /// Create a new chain step.
    pub fn new(instruction: impl Into<String>) -> Self {
        Self {
            instruction: instruction.into(),
            condition: None,
            continue_on_failure: false,
            extract: None,
            timeout_ms: None,
            max_retries: None,
        }
    }

    /// Add a condition for this step.
    pub fn when(mut self, condition: ChainCondition) -> Self {
        self.condition = Some(condition);
        self
    }

    /// Continue chain even if this step fails.
    pub fn allow_failure(mut self) -> Self {
        self.continue_on_failure = true;
        self
    }

    /// Extract data after this step.
    pub fn then_extract(mut self, prompt: impl Into<String>) -> Self {
        self.extract = Some(prompt.into());
        self
    }

    /// Set timeout for this step.
    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Set max retries for this step.
    pub fn with_retries(mut self, retries: usize) -> Self {
        self.max_retries = Some(retries);
        self
    }

    /// Check if this step should be executed based on condition.
    pub fn should_execute(&self, context: &ChainContext) -> bool {
        match &self.condition {
            None => true,
            Some(condition) => condition.evaluate(context),
        }
    }
}

/// Condition for conditional execution in action chains.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum ChainCondition {
    /// Execute if URL contains this string.
    UrlContains(String),
    /// Execute if URL matches this pattern (regex-like).
    UrlMatches(String),
    /// Execute if page contains this text.
    PageContains(String),
    /// Execute if an element matching this selector exists.
    ElementExists(String),
    /// Execute if previous step succeeded.
    PreviousSucceeded,
    /// Execute if previous step failed.
    PreviousFailed,
    /// Always execute (default).
    #[default]
    Always,
    /// Never execute (skip).
    Never,
    /// All conditions must be true.
    All(Vec<ChainCondition>),
    /// Any condition must be true.
    Any(Vec<ChainCondition>),
    /// Invert the condition.
    Not(Box<ChainCondition>),
}

impl std::ops::Not for ChainCondition {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::Not(Box::new(self))
    }
}

impl ChainCondition {
    /// Create a URL contains condition.
    pub fn url_contains(pattern: impl Into<String>) -> Self {
        Self::UrlContains(pattern.into())
    }

    /// Create an element exists condition.
    pub fn element_exists(selector: impl Into<String>) -> Self {
        Self::ElementExists(selector.into())
    }

    /// Create a page contains condition.
    pub fn page_contains(text: impl Into<String>) -> Self {
        Self::PageContains(text.into())
    }

    /// Negate this condition.
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        std::ops::Not::not(self)
    }

    /// Combine with AND.
    pub fn and(self, other: ChainCondition) -> Self {
        match self {
            Self::All(mut conditions) => {
                conditions.push(other);
                Self::All(conditions)
            }
            _ => Self::All(vec![self, other]),
        }
    }

    /// Combine with OR.
    pub fn or(self, other: ChainCondition) -> Self {
        match self {
            Self::Any(mut conditions) => {
                conditions.push(other);
                Self::Any(conditions)
            }
            _ => Self::Any(vec![self, other]),
        }
    }

    /// Evaluate this condition against the context.
    pub fn evaluate(&self, ctx: &ChainContext) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::UrlContains(pattern) => ctx.current_url.contains(pattern),
            Self::UrlMatches(pattern) => {
                // Simple glob-like pattern matching - * is wildcard
                simple_glob_match(pattern, &ctx.current_url)
            }
            Self::PageContains(text) => ctx.page_text.contains(text),
            Self::ElementExists(selector) => ctx.existing_selectors.contains(selector),
            Self::PreviousSucceeded => ctx.previous_succeeded,
            Self::PreviousFailed => !ctx.previous_succeeded,
            Self::Not(inner) => !inner.evaluate(ctx),
            Self::All(conditions) => conditions.iter().all(|c| c.evaluate(ctx)),
            Self::Any(conditions) => conditions.iter().any(|c| c.evaluate(ctx)),
        }
    }
}

/// Context for evaluating chain conditions.
#[derive(Debug, Clone, Default)]
pub struct ChainContext {
    /// Current page URL.
    pub current_url: String,
    /// Current page text content.
    pub page_text: String,
    /// Selectors that exist on the page.
    pub existing_selectors: Vec<String>,
    /// Whether the previous step succeeded.
    pub previous_succeeded: bool,
    /// Step index (0-based).
    pub step_index: usize,
}

impl ChainContext {
    /// Create a new context.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            current_url: url.into(),
            previous_succeeded: true, // Start optimistically
            ..Default::default()
        }
    }

    /// Update URL.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.current_url = url.into();
        self
    }

    /// Update page text.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.page_text = text.into();
        self
    }

    /// Add existing selector.
    pub fn add_selector(&mut self, selector: impl Into<String>) {
        self.existing_selectors.push(selector.into());
    }

    /// Mark previous step result.
    pub fn set_previous_result(&mut self, succeeded: bool) {
        self.previous_succeeded = succeeded;
    }

    /// Advance to next step.
    pub fn advance(&mut self) {
        self.step_index += 1;
    }
}

/// Result of an action chain execution.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ChainResult {
    /// Whether all required steps completed successfully.
    pub success: bool,
    /// Number of steps executed.
    pub steps_executed: usize,
    /// Number of steps that succeeded.
    pub steps_succeeded: usize,
    /// Number of steps that failed.
    pub steps_failed: usize,
    /// Number of steps skipped (due to conditions).
    pub steps_skipped: usize,
    /// Results from each step.
    pub step_results: Vec<ChainStepResult>,
    /// Extracted data from steps with extraction.
    #[serde(default)]
    pub extractions: Vec<serde_json::Value>,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Total token usage.
    #[serde(default)]
    pub total_usage: AutomationUsage,
    /// Final URL after chain execution.
    pub final_url: Option<String>,
    /// Error that caused chain to stop (if any).
    pub error: Option<String>,
}

impl ChainResult {
    /// Create an empty result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a step result.
    pub fn add_step(&mut self, result: ChainStepResult) {
        if result.executed {
            self.steps_executed += 1;
            if result.success {
                self.steps_succeeded += 1;
            } else {
                self.steps_failed += 1;
            }
        } else {
            self.steps_skipped += 1;
        }

        if let Some(ref extracted) = result.extracted {
            self.extractions.push(extracted.clone());
        }

        self.duration_ms += result.duration_ms;
        self.total_usage.accumulate(&result.usage);
        self.step_results.push(result);
    }

    /// Mark as complete.
    pub fn complete(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    /// Set final URL.
    pub fn with_final_url(mut self, url: impl Into<String>) -> Self {
        self.final_url = Some(url.into());
        self
    }

    /// Set error.
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self.success = false;
        self
    }
}

/// Result of a single step in an action chain.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ChainStepResult {
    /// Step index (0-based).
    pub index: usize,
    /// The instruction that was executed.
    pub instruction: String,
    /// Whether the step was executed (false if condition not met).
    pub executed: bool,
    /// Whether the step succeeded (if executed).
    pub success: bool,
    /// Action taken (if executed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_taken: Option<String>,
    /// Error message (if failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Extracted data (if extraction was requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted: Option<serde_json::Value>,
    /// Token usage for this step.
    #[serde(default)]
    pub usage: AutomationUsage,
    /// Number of retries used.
    pub retries: usize,
}

impl ChainStepResult {
    /// Create a result for an executed step.
    pub fn executed(index: usize, instruction: impl Into<String>, success: bool) -> Self {
        Self {
            index,
            instruction: instruction.into(),
            executed: true,
            success,
            ..Default::default()
        }
    }

    /// Create a result for a skipped step.
    pub fn skipped(index: usize, instruction: impl Into<String>) -> Self {
        Self {
            index,
            instruction: instruction.into(),
            executed: false,
            success: false, // N/A for skipped
            ..Default::default()
        }
    }

    /// Set action taken.
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action_taken = Some(action.into());
        self
    }

    /// Set error.
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self.success = false;
        self
    }

    /// Set duration.
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    /// Set extracted data.
    pub fn with_extracted(mut self, data: serde_json::Value) -> Self {
        self.extracted = Some(data);
        self
    }

    /// Set usage.
    pub fn with_usage(mut self, usage: AutomationUsage) -> Self {
        self.usage = usage;
        self
    }

    /// Set retries.
    pub fn with_retries(mut self, retries: usize) -> Self {
        self.retries = retries;
        self
    }
}

/// Builder for creating action chains.
#[derive(Debug, Clone, Default)]
pub struct ChainBuilder {
    steps: Vec<ChainStep>,
}

impl ChainBuilder {
    /// Create a new chain builder.
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Add a step to the chain.
    pub fn step(mut self, step: ChainStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Add a simple instruction step.
    pub fn then(mut self, instruction: impl Into<String>) -> Self {
        self.steps.push(ChainStep::new(instruction));
        self
    }

    /// Add a conditional step.
    pub fn when(mut self, condition: ChainCondition, instruction: impl Into<String>) -> Self {
        self.steps.push(ChainStep::new(instruction).when(condition));
        self
    }

    /// Build the chain.
    pub fn build(self) -> Vec<ChainStep> {
        self.steps
    }
}

/// Simple glob-like pattern matching.
/// Supports * as wildcard for any characters.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();

    if parts.len() == 1 {
        // No wildcards - exact match
        return pattern == text;
    }

    let mut pos = 0;

    // First part must match at the beginning
    if !parts[0].is_empty() {
        if !text.starts_with(parts[0]) {
            return false;
        }
        pos = parts[0].len();
    }

    // Middle parts can match anywhere after current position
    for part in &parts[1..parts.len() - 1] {
        if part.is_empty() {
            continue;
        }
        if let Some(found) = text[pos..].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }

    // Last part must match at the end
    let last = parts[parts.len() - 1];
    if !last.is_empty() {
        text[pos..].ends_with(last)
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_step() {
        let step = ChainStep::new("Click login button")
            .when(ChainCondition::element_exists("button.login"))
            .allow_failure()
            .then_extract("Extract user info");

        assert!(step.continue_on_failure);
        assert!(step.extract.is_some());
        assert!(step.condition.is_some());
    }

    #[test]
    fn test_chain_condition_evaluate() {
        let ctx = ChainContext::new("https://example.com/dashboard").with_text("Welcome, user!");

        assert!(ChainCondition::url_contains("dashboard").evaluate(&ctx));
        assert!(ChainCondition::page_contains("Welcome").evaluate(&ctx));
        assert!(!ChainCondition::url_contains("login").evaluate(&ctx));
    }

    #[test]
    fn test_chain_condition_compound() {
        let ctx = ChainContext::new("https://example.com/dashboard");

        let condition =
            ChainCondition::url_contains("example").and(ChainCondition::url_contains("dashboard"));

        assert!(condition.evaluate(&ctx));

        let condition =
            ChainCondition::url_contains("login").or(ChainCondition::url_contains("dashboard"));

        assert!(condition.evaluate(&ctx));
    }

    #[test]
    fn test_chain_result() {
        let mut result = ChainResult::new();

        result.add_step(ChainStepResult::executed(0, "Step 1", true).with_duration(100));
        result.add_step(ChainStepResult::skipped(1, "Step 2"));
        result.add_step(ChainStepResult::executed(2, "Step 3", false).with_error("Failed"));

        assert_eq!(result.steps_executed, 2);
        assert_eq!(result.steps_succeeded, 1);
        assert_eq!(result.steps_failed, 1);
        assert_eq!(result.steps_skipped, 1);
    }

    #[test]
    fn test_chain_builder() {
        let chain = ChainBuilder::new()
            .then("Navigate to login")
            .then("Enter credentials")
            .when(
                ChainCondition::element_exists("button.submit"),
                "Click submit",
            )
            .build();

        assert_eq!(chain.len(), 3);
        assert!(chain[2].condition.is_some());
    }
}
