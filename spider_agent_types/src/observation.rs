//! Page observation types for understanding page state.

use crate::AutomationUsage;

/// Observation of a page's current state.
///
/// Used by the agent to understand what's on the page and what actions
/// are available.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PageObservation {
    /// Current page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Brief description of the page.
    pub description: String,
    /// Type of page (login, search, product, etc.).
    pub page_type: String,
    /// Interactive elements on the page.
    #[serde(default)]
    pub interactive_elements: Vec<InteractiveElement>,
    /// Forms on the page.
    #[serde(default)]
    pub forms: Vec<FormInfo>,
    /// Navigation options.
    #[serde(default)]
    pub navigation: Vec<NavigationOption>,
    /// Suggested next actions.
    #[serde(default)]
    pub suggested_actions: Vec<String>,
    /// Screenshot (base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    /// Token usage for this observation.
    #[serde(default)]
    pub usage: AutomationUsage,
    /// Raw HTML (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// Page text content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
}

impl PageObservation {
    /// Create a new observation.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Set title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set page type.
    pub fn with_page_type(mut self, page_type: impl Into<String>) -> Self {
        self.page_type = page_type.into();
        self
    }

    /// Add an interactive element.
    pub fn add_element(mut self, element: InteractiveElement) -> Self {
        self.interactive_elements.push(element);
        self
    }

    /// Add a form.
    pub fn add_form(mut self, form: FormInfo) -> Self {
        self.forms.push(form);
        self
    }

    /// Add a navigation option.
    pub fn add_navigation(mut self, nav: NavigationOption) -> Self {
        self.navigation.push(nav);
        self
    }

    /// Add a suggested action.
    pub fn suggest_action(mut self, action: impl Into<String>) -> Self {
        self.suggested_actions.push(action.into());
        self
    }

    /// Set screenshot.
    pub fn with_screenshot(mut self, screenshot: impl Into<String>) -> Self {
        self.screenshot = Some(screenshot.into());
        self
    }

    /// Set usage.
    pub fn with_usage(mut self, usage: AutomationUsage) -> Self {
        self.usage = usage;
        self
    }

    /// Find an element by selector.
    pub fn find_element(&self, selector: &str) -> Option<&InteractiveElement> {
        self.interactive_elements
            .iter()
            .find(|e| e.selector == selector)
    }

    /// Find elements by type.
    pub fn elements_by_type(&self, element_type: &str) -> Vec<&InteractiveElement> {
        self.interactive_elements
            .iter()
            .filter(|e| e.element_type == element_type)
            .collect()
    }

    /// Get clickable elements.
    pub fn clickable_elements(&self) -> Vec<&InteractiveElement> {
        self.interactive_elements
            .iter()
            .filter(|e| {
                e.visible && e.enabled && (e.element_type == "button" || e.element_type == "link")
            })
            .collect()
    }

    /// Get form inputs.
    pub fn input_elements(&self) -> Vec<&InteractiveElement> {
        self.interactive_elements
            .iter()
            .filter(|e| e.element_type.starts_with("input") || e.element_type == "textarea")
            .collect()
    }
}

/// An interactive element on the page.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct InteractiveElement {
    /// CSS selector for this element.
    pub selector: String,
    /// Type of element (button, input, link, etc.).
    pub element_type: String,
    /// Visible text content.
    pub text: String,
    /// Description of the element's purpose.
    pub description: String,
    /// Whether the element is visible.
    pub visible: bool,
    /// Whether the element is enabled.
    pub enabled: bool,
    /// Element tag name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Element ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Element classes.
    #[serde(default)]
    pub classes: Vec<String>,
    /// ARIA label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<String>,
    /// Placeholder text (for inputs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Current value (for inputs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Bounding box [x, y, width, height].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
}

impl InteractiveElement {
    /// Create a new interactive element.
    pub fn new(selector: impl Into<String>, element_type: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            element_type: element_type.into(),
            visible: true,
            enabled: true,
            ..Default::default()
        }
    }

    /// Create a button element.
    pub fn button(selector: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            element_type: "button".to_string(),
            text: text.into(),
            visible: true,
            enabled: true,
            ..Default::default()
        }
    }

    /// Create an input element.
    pub fn input(selector: impl Into<String>, input_type: &str) -> Self {
        Self {
            selector: selector.into(),
            element_type: format!("input:{}", input_type),
            visible: true,
            enabled: true,
            ..Default::default()
        }
    }

    /// Create a link element.
    pub fn link(selector: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            element_type: "link".to_string(),
            text: text.into(),
            visible: true,
            enabled: true,
            ..Default::default()
        }
    }

    /// Set text.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set visibility.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Set enabled state.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set placeholder.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set aria label.
    pub fn with_aria_label(mut self, label: impl Into<String>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// Check if this element can be interacted with.
    pub fn is_actionable(&self) -> bool {
        self.visible && self.enabled
    }

    /// Get a human-readable description.
    pub fn describe(&self) -> String {
        if !self.description.is_empty() {
            return self.description.clone();
        }
        if !self.text.is_empty() {
            return format!("{}: {}", self.element_type, self.text);
        }
        if let Some(ref label) = self.aria_label {
            return format!("{}: {}", self.element_type, label);
        }
        if let Some(ref placeholder) = self.placeholder {
            return format!("{}: {}", self.element_type, placeholder);
        }
        format!("{} at {}", self.element_type, self.selector)
    }
}

/// Information about a form on the page.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FormInfo {
    /// CSS selector for the form.
    pub selector: String,
    /// Form name attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Form action URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Form method (GET, POST).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Form fields.
    #[serde(default)]
    pub fields: Vec<FormField>,
    /// Description of what this form does.
    pub description: String,
}

impl FormInfo {
    /// Create a new form info.
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            ..Default::default()
        }
    }

    /// Set name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set action.
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Add a field.
    pub fn add_field(mut self, field: FormField) -> Self {
        self.fields.push(field);
        self
    }

    /// Get required fields.
    pub fn required_fields(&self) -> Vec<&FormField> {
        self.fields.iter().filter(|f| f.required).collect()
    }

    /// Get empty required fields.
    pub fn empty_required_fields(&self) -> Vec<&FormField> {
        self.fields
            .iter()
            .filter(|f| f.required && f.value.as_ref().map(|v| v.is_empty()).unwrap_or(true))
            .collect()
    }
}

/// A field in a form.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FormField {
    /// Field name attribute.
    pub name: String,
    /// Field type (text, email, password, etc.).
    pub field_type: String,
    /// Field label text.
    pub label: String,
    /// Whether the field is required.
    pub required: bool,
    /// Current value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Placeholder text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// CSS selector.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// Options for select fields.
    #[serde(default)]
    pub options: Vec<String>,
}

impl FormField {
    /// Create a new form field.
    pub fn new(name: impl Into<String>, field_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            field_type: field_type.into(),
            ..Default::default()
        }
    }

    /// Set label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Set required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set selector.
    pub fn with_selector(mut self, selector: impl Into<String>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    /// Set current value.
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }
}

/// Result of a single action execution via `act()`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ActResult {
    /// Whether the action was executed successfully.
    pub success: bool,
    /// Description of the action that was taken.
    pub action_taken: String,
    /// The specific action executed (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_type: Option<String>,
    /// Base64-encoded screenshot after the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    /// Error message if the action failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Token usage for this action.
    #[serde(default)]
    pub usage: AutomationUsage,
}

impl ActResult {
    /// Create a successful action result.
    pub fn success(action_taken: impl Into<String>) -> Self {
        Self {
            success: true,
            action_taken: action_taken.into(),
            ..Default::default()
        }
    }

    /// Create a failed action result.
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(error.into()),
            ..Default::default()
        }
    }

    /// Set the action type.
    pub fn with_action_type(mut self, action_type: impl Into<String>) -> Self {
        self.action_type = Some(action_type.into());
        self
    }

    /// Set the screenshot.
    pub fn with_screenshot(mut self, screenshot: impl Into<String>) -> Self {
        self.screenshot = Some(screenshot.into());
        self
    }

    /// Set usage stats.
    pub fn with_usage(mut self, usage: AutomationUsage) -> Self {
        self.usage = usage;
        self
    }
}

/// A navigation option on the page.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NavigationOption {
    /// Link text.
    pub text: String,
    /// Target URL.
    pub url: String,
    /// CSS selector.
    pub selector: String,
    /// Whether this is the current page.
    pub is_current: bool,
    /// Category (main nav, footer, sidebar, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

impl NavigationOption {
    /// Create a new navigation option.
    pub fn new(
        text: impl Into<String>,
        url: impl Into<String>,
        selector: impl Into<String>,
    ) -> Self {
        Self {
            text: text.into(),
            url: url.into(),
            selector: selector.into(),
            is_current: false,
            category: None,
        }
    }

    /// Mark as current page.
    pub fn current(mut self) -> Self {
        self.is_current = true;
        self
    }

    /// Set category.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_observation() {
        let obs = PageObservation::new("https://example.com")
            .with_title("Example")
            .with_page_type("homepage")
            .add_element(InteractiveElement::button("btn.login", "Login"))
            .add_element(InteractiveElement::input("input.email", "email"))
            .suggest_action("Click login button");

        assert_eq!(obs.clickable_elements().len(), 1);
        assert_eq!(obs.input_elements().len(), 1);
        assert_eq!(obs.suggested_actions.len(), 1);
    }

    #[test]
    fn test_interactive_element() {
        let elem = InteractiveElement::button("button.submit", "Submit")
            .with_description("Submit the form")
            .with_aria_label("Submit form");

        assert!(elem.is_actionable());
        assert_eq!(elem.describe(), "Submit the form");

        let disabled = elem.clone().enabled(false);
        assert!(!disabled.is_actionable());
    }

    #[test]
    fn test_form_info() {
        let form = FormInfo::new("form#login")
            .with_action("/login")
            .add_field(
                FormField::new("email", "email")
                    .required()
                    .with_label("Email"),
            )
            .add_field(FormField::new("password", "password").required());

        assert_eq!(form.required_fields().len(), 2);
        assert_eq!(form.empty_required_fields().len(), 2);
    }
}
