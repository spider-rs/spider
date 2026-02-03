//! Action types and results for automation.

use super::AutomationUsage;

/// Types of actions that can be performed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ActionType {
    /// Navigate to a URL.
    Navigate,
    /// Click an element.
    Click,
    /// Type text into an element.
    Type,
    /// Clear an input field.
    Clear,
    /// Select an option from a dropdown.
    Select,
    /// Check/uncheck a checkbox.
    Check,
    /// Scroll the page or element.
    Scroll,
    /// Wait for an element or condition.
    Wait,
    /// Take a screenshot.
    Screenshot,
    /// Execute JavaScript.
    Script,
    /// Press a key or key combination.
    KeyPress,
    /// Hover over an element.
    Hover,
    /// Drag and drop.
    DragDrop,
    /// Submit a form.
    Submit,
    /// Go back in history.
    Back,
    /// Go forward in history.
    Forward,
    /// Refresh the page.
    Refresh,
    /// Extract data from the page.
    Extract,
    /// Custom action.
    Custom(String),
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Navigate => write!(f, "navigate"),
            Self::Click => write!(f, "click"),
            Self::Type => write!(f, "type"),
            Self::Clear => write!(f, "clear"),
            Self::Select => write!(f, "select"),
            Self::Check => write!(f, "check"),
            Self::Scroll => write!(f, "scroll"),
            Self::Wait => write!(f, "wait"),
            Self::Screenshot => write!(f, "screenshot"),
            Self::Script => write!(f, "script"),
            Self::KeyPress => write!(f, "keypress"),
            Self::Hover => write!(f, "hover"),
            Self::DragDrop => write!(f, "drag_drop"),
            Self::Submit => write!(f, "submit"),
            Self::Back => write!(f, "back"),
            Self::Forward => write!(f, "forward"),
            Self::Refresh => write!(f, "refresh"),
            Self::Extract => write!(f, "extract"),
            Self::Custom(name) => write!(f, "custom:{}", name),
        }
    }
}

/// Result of an action execution.
#[derive(Debug, Clone, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ActionResult {
    /// Whether the action succeeded.
    pub success: bool,
    /// Description of action taken.
    pub action_taken: String,
    /// Type of action performed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_type: Option<ActionType>,
    /// Screenshot after action (base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Token usage for this action.
    #[serde(default)]
    pub usage: AutomationUsage,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Element selector used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// URL before action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_before: Option<String>,
    /// URL after action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_after: Option<String>,
}

impl ActionResult {
    /// Create a successful action result.
    pub fn success(action: impl Into<String>) -> Self {
        Self {
            success: true,
            action_taken: action.into(),
            ..Default::default()
        }
    }

    /// Create a failed action result.
    pub fn failure(action: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            success: false,
            action_taken: action.into(),
            error: Some(error.into()),
            ..Default::default()
        }
    }

    /// Set the action type.
    pub fn with_type(mut self, action_type: ActionType) -> Self {
        self.action_type = Some(action_type);
        self
    }

    /// Set the screenshot.
    pub fn with_screenshot(mut self, screenshot: impl Into<String>) -> Self {
        self.screenshot = Some(screenshot.into());
        self
    }

    /// Set the selector.
    pub fn with_selector(mut self, selector: impl Into<String>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    /// Set duration.
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    /// Set usage stats.
    pub fn with_usage(mut self, usage: AutomationUsage) -> Self {
        self.usage = usage;
        self
    }

    /// Set URLs before/after.
    pub fn with_urls(mut self, before: impl Into<String>, after: impl Into<String>) -> Self {
        self.url_before = Some(before.into());
        self.url_after = Some(after.into());
        self
    }
}

/// Record of an action taken during automation.
#[derive(Debug, Clone, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ActionRecord {
    /// Step number (1-indexed).
    pub step: usize,
    /// Description of action.
    pub action: String,
    /// Whether it succeeded.
    pub success: bool,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// URL before action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_before: Option<String>,
    /// URL after action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_after: Option<String>,
    /// Error if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Number of retries needed.
    pub retries: usize,
}

impl ActionRecord {
    /// Create a new action record.
    pub fn new(step: usize, action: impl Into<String>, success: bool) -> Self {
        Self {
            step,
            action: action.into(),
            success,
            ..Default::default()
        }
    }

    /// Create from an ActionResult.
    pub fn from_result(step: usize, result: &ActionResult) -> Self {
        Self {
            step,
            action: result.action_taken.clone(),
            success: result.success,
            duration_ms: result.duration_ms,
            url_before: result.url_before.clone(),
            url_after: result.url_after.clone(),
            error: result.error.clone(),
            retries: 0,
        }
    }

    /// Set retries.
    pub fn with_retries(mut self, retries: usize) -> Self {
        self.retries = retries;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_result() {
        let result = ActionResult::success("Clicked login button")
            .with_type(ActionType::Click)
            .with_selector("button.login")
            .with_duration(150);

        assert!(result.success);
        assert_eq!(result.action_type, Some(ActionType::Click));
        assert_eq!(result.selector, Some("button.login".to_string()));
    }

    #[test]
    fn test_action_record() {
        let result = ActionResult::success("Typed email").with_duration(100);

        let record = ActionRecord::from_result(1, &result);

        assert_eq!(record.step, 1);
        assert!(record.success);
        assert_eq!(record.duration_ms, 100);
    }

    #[test]
    fn test_action_type_display() {
        assert_eq!(ActionType::Click.to_string(), "click");
        assert_eq!(ActionType::Navigate.to_string(), "navigate");
        assert_eq!(ActionType::Custom("foo".to_string()).to_string(), "custom:foo");
    }
}
