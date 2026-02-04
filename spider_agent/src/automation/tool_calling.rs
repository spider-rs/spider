//! OpenAI-compatible tool calling support.
//!
//! This module provides tool schemas for browser automation actions,
//! enabling structured tool calling instead of free-form JSON parsing.
//! This can reduce parse errors by ~30%.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Mode for how actions should be formatted in LLM requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallingMode {
    /// Use JSON object mode (original behavior).
    #[default]
    JsonObject,
    /// Use OpenAI-compatible tool/function calling.
    ToolCalling,
    /// Auto-select based on model capabilities.
    Auto,
}

impl ToolCallingMode {
    /// Check if tool calling should be used for a given model.
    pub fn should_use_tools(&self, model_name: &str) -> bool {
        match self {
            ToolCallingMode::JsonObject => false,
            ToolCallingMode::ToolCalling => true,
            ToolCallingMode::Auto => {
                // Models known to support tool calling well
                let lower = model_name.to_lowercase();
                lower.contains("gpt-4")
                    || lower.contains("gpt-3.5-turbo")
                    || lower.contains("claude-3")
                    || lower.contains("claude-4")
                    || lower.contains("gemini")
                    || lower.contains("mistral")
            }
        }
    }
}

/// OpenAI-compatible function definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    /// Name of the function.
    pub name: String,
    /// Description of what the function does.
    pub description: String,
    /// JSON Schema for the function parameters.
    pub parameters: Value,
}

impl FunctionDefinition {
    /// Create a new function definition.
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// OpenAI-compatible tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Type of tool (always "function" for now).
    #[serde(rename = "type")]
    pub tool_type: String,
    /// The function definition.
    pub function: FunctionDefinition,
}

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new(function: FunctionDefinition) -> Self {
        Self {
            tool_type: "function".to_string(),
            function,
        }
    }

    /// Create from name, description, and parameters.
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self::new(FunctionDefinition::new(name, description, parameters))
    }
}

/// A tool call from the LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this tool call.
    pub id: String,
    /// Type of tool (always "function").
    #[serde(rename = "type")]
    pub call_type: String,
    /// The function call details.
    pub function: FunctionCall,
}

impl ToolCall {
    /// Parse from OpenAI response format.
    pub fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            call_type: value
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("function")
                .to_string(),
            function: FunctionCall::from_json(value.get("function")?)?,
        })
    }

    /// Convert to automation action Value.
    pub fn to_action(&self) -> Value {
        // Convert function call to enum-style action
        // e.g., { "name": "Click", "arguments": "{ \"selector\": \"btn\" }" }
        // becomes { "Click": "btn" }
        let args: Value = serde_json::from_str(&self.function.arguments).unwrap_or(Value::Null);

        match self.function.name.as_str() {
            // Simple string argument actions
            "Click" | "DoubleClick" | "RightClick" | "Clear" | "Focus" | "Blur" | "Hover"
            | "WaitFor" | "Navigate" | "OpenPage" | "Evaluate" => {
                let arg = args
                    .get("selector")
                    .or_else(|| args.get("url"))
                    .or_else(|| args.get("code"))
                    .cloned()
                    .unwrap_or(args);
                json!({ self.function.name.clone(): arg })
            }

            // Object argument actions
            "Fill" | "Select" | "ClickPoint" | "ClickHold" | "ClickHoldPoint" | "ClickDrag"
            | "ClickDragPoint" | "ScrollTo" | "ScrollToPoint" | "WaitForWithTimeout"
            | "WaitForDom" | "Type" | "Press" | "KeyDown" | "KeyUp" => {
                json!({ self.function.name.clone(): args })
            }

            // Simple number argument actions
            "Wait" | "ScrollX" | "ScrollY" | "InfiniteScroll" => {
                let arg = args
                    .get("ms")
                    .or_else(|| args.get("pixels"))
                    .or_else(|| args.get("count"))
                    .cloned()
                    .unwrap_or(args);
                json!({ self.function.name.clone(): arg })
            }

            // No argument actions
            "GoBack" | "GoForward" | "Reload" | "WaitForNavigation" | "Screenshot" => {
                json!({ self.function.name.clone(): Value::Null })
            }

            // Default: pass through
            _ => json!({ self.function.name.clone(): args }),
        }
    }
}

/// A function call from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    /// Name of the function to call.
    pub name: String,
    /// Arguments as a JSON string.
    pub arguments: String,
}

impl FunctionCall {
    /// Parse from JSON.
    pub fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            name: value.get("name")?.as_str()?.to_string(),
            arguments: value.get("arguments")?.as_str()?.to_string(),
        })
    }

    /// Parse the arguments as JSON.
    pub fn parse_arguments(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_str(&self.arguments)
    }
}

/// Generator for automation action tool schemas.
pub struct ActionToolSchemas;

impl ActionToolSchemas {
    /// Get all tool definitions for automation actions.
    pub fn all() -> Vec<ToolDefinition> {
        vec![
            // Click actions
            Self::click(),
            Self::click_point(),
            Self::click_all(),
            Self::double_click(),
            Self::double_click_point(),
            Self::right_click(),
            Self::right_click_point(),
            Self::click_hold(),
            Self::click_hold_point(),
            Self::wait_for_and_click(),
            // Drag actions
            Self::click_drag(),
            Self::click_drag_point(),
            // Input actions
            Self::fill(),
            Self::type_text(),
            Self::clear(),
            Self::press(),
            Self::key_down(),
            Self::key_up(),
            // Select/Focus
            Self::select(),
            Self::focus(),
            Self::blur(),
            Self::hover(),
            Self::hover_point(),
            // Scroll
            Self::scroll_x(),
            Self::scroll_y(),
            Self::scroll_to(),
            Self::scroll_to_point(),
            Self::infinite_scroll(),
            // Wait
            Self::wait(),
            Self::wait_for(),
            Self::wait_for_with_timeout(),
            Self::wait_for_navigation(),
            Self::wait_for_dom(),
            // Navigate
            Self::navigate(),
            Self::open_page(),
            Self::go_back(),
            Self::go_forward(),
            Self::reload(),
            // Advanced
            Self::evaluate(),
            Self::screenshot(),
        ]
    }

    /// Get a subset of commonly used tools.
    pub fn common() -> Vec<ToolDefinition> {
        vec![
            Self::click(),
            Self::click_point(),
            Self::fill(),
            Self::press(),
            Self::scroll_y(),
            Self::wait(),
            Self::wait_for(),
            Self::navigate(),
            Self::evaluate(),
        ]
    }

    // Click actions
    fn click() -> ToolDefinition {
        ToolDefinition::function(
            "Click",
            "Click an element by CSS selector",
            json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the element to click"
                    }
                },
                "required": ["selector"]
            }),
        )
    }

    fn click_point() -> ToolDefinition {
        ToolDefinition::function(
            "ClickPoint",
            "Click at specific x,y coordinates",
            json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "X coordinate" },
                    "y": { "type": "number", "description": "Y coordinate" }
                },
                "required": ["x", "y"]
            }),
        )
    }

    fn click_all() -> ToolDefinition {
        ToolDefinition::function(
            "ClickAll",
            "Click all elements matching a selector",
            json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for elements to click"
                    }
                },
                "required": ["selector"]
            }),
        )
    }

    fn double_click() -> ToolDefinition {
        ToolDefinition::function(
            "DoubleClick",
            "Double-click an element by CSS selector",
            json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the element"
                    }
                },
                "required": ["selector"]
            }),
        )
    }

    fn double_click_point() -> ToolDefinition {
        ToolDefinition::function(
            "DoubleClickPoint",
            "Double-click at specific coordinates",
            json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                },
                "required": ["x", "y"]
            }),
        )
    }

    fn right_click() -> ToolDefinition {
        ToolDefinition::function(
            "RightClick",
            "Right-click an element (context menu)",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    fn right_click_point() -> ToolDefinition {
        ToolDefinition::function(
            "RightClickPoint",
            "Right-click at specific coordinates",
            json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                },
                "required": ["x", "y"]
            }),
        )
    }

    fn click_hold() -> ToolDefinition {
        ToolDefinition::function(
            "ClickHold",
            "Click and hold an element (long press)",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" },
                    "hold_ms": { "type": "integer", "description": "Hold duration in milliseconds" }
                },
                "required": ["selector", "hold_ms"]
            }),
        )
    }

    fn click_hold_point() -> ToolDefinition {
        ToolDefinition::function(
            "ClickHoldPoint",
            "Click and hold at coordinates",
            json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "hold_ms": { "type": "integer" }
                },
                "required": ["x", "y", "hold_ms"]
            }),
        )
    }

    fn wait_for_and_click() -> ToolDefinition {
        ToolDefinition::function(
            "WaitForAndClick",
            "Wait for an element to appear then click it",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    // Drag actions
    fn click_drag() -> ToolDefinition {
        ToolDefinition::function(
            "ClickDrag",
            "Drag from one element to another",
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source element selector" },
                    "to": { "type": "string", "description": "Target element selector" }
                },
                "required": ["from", "to"]
            }),
        )
    }

    fn click_drag_point() -> ToolDefinition {
        ToolDefinition::function(
            "ClickDragPoint",
            "Drag from one point to another",
            json!({
                "type": "object",
                "properties": {
                    "from_x": { "type": "number" },
                    "from_y": { "type": "number" },
                    "to_x": { "type": "number" },
                    "to_y": { "type": "number" }
                },
                "required": ["from_x", "from_y", "to_x", "to_y"]
            }),
        )
    }

    // Input actions
    fn fill() -> ToolDefinition {
        ToolDefinition::function(
            "Fill",
            "Clear and type text into an input element",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "Input element selector" },
                    "value": { "type": "string", "description": "Text to type" }
                },
                "required": ["selector", "value"]
            }),
        )
    }

    fn type_text() -> ToolDefinition {
        ToolDefinition::function(
            "Type",
            "Type text into the focused element",
            json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string", "description": "Text to type" }
                },
                "required": ["value"]
            }),
        )
    }

    fn clear() -> ToolDefinition {
        ToolDefinition::function(
            "Clear",
            "Clear an input field",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    fn press() -> ToolDefinition {
        ToolDefinition::function(
            "Press",
            "Press a keyboard key",
            json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Key name (Enter, Tab, Escape, ArrowDown, Space, etc.)"
                    }
                },
                "required": ["key"]
            }),
        )
    }

    fn key_down() -> ToolDefinition {
        ToolDefinition::function(
            "KeyDown",
            "Hold down a key",
            json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" }
                },
                "required": ["key"]
            }),
        )
    }

    fn key_up() -> ToolDefinition {
        ToolDefinition::function(
            "KeyUp",
            "Release a key",
            json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" }
                },
                "required": ["key"]
            }),
        )
    }

    // Select/Focus
    fn select() -> ToolDefinition {
        ToolDefinition::function(
            "Select",
            "Select an option from a dropdown",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "Select element selector" },
                    "value": { "type": "string", "description": "Option value to select" }
                },
                "required": ["selector", "value"]
            }),
        )
    }

    fn focus() -> ToolDefinition {
        ToolDefinition::function(
            "Focus",
            "Focus an element",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    fn blur() -> ToolDefinition {
        ToolDefinition::function(
            "Blur",
            "Remove focus from an element",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    fn hover() -> ToolDefinition {
        ToolDefinition::function(
            "Hover",
            "Hover over an element",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    fn hover_point() -> ToolDefinition {
        ToolDefinition::function(
            "HoverPoint",
            "Hover at specific coordinates",
            json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                },
                "required": ["x", "y"]
            }),
        )
    }

    // Scroll
    fn scroll_x() -> ToolDefinition {
        ToolDefinition::function(
            "ScrollX",
            "Scroll horizontally",
            json!({
                "type": "object",
                "properties": {
                    "pixels": { "type": "integer", "description": "Pixels to scroll (negative = left)" }
                },
                "required": ["pixels"]
            }),
        )
    }

    fn scroll_y() -> ToolDefinition {
        ToolDefinition::function(
            "ScrollY",
            "Scroll vertically",
            json!({
                "type": "object",
                "properties": {
                    "pixels": { "type": "integer", "description": "Pixels to scroll (negative = up)" }
                },
                "required": ["pixels"]
            }),
        )
    }

    fn scroll_to() -> ToolDefinition {
        ToolDefinition::function(
            "ScrollTo",
            "Scroll element into view",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    fn scroll_to_point() -> ToolDefinition {
        ToolDefinition::function(
            "ScrollToPoint",
            "Scroll to specific coordinates",
            json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "integer" }
                },
                "required": ["x", "y"]
            }),
        )
    }

    fn infinite_scroll() -> ToolDefinition {
        ToolDefinition::function(
            "InfiniteScroll",
            "Scroll to bottom repeatedly for infinite scroll pages",
            json!({
                "type": "object",
                "properties": {
                    "count": { "type": "integer", "description": "Number of scroll iterations" }
                },
                "required": ["count"]
            }),
        )
    }

    // Wait
    fn wait() -> ToolDefinition {
        ToolDefinition::function(
            "Wait",
            "Wait for a fixed duration",
            json!({
                "type": "object",
                "properties": {
                    "ms": { "type": "integer", "description": "Milliseconds to wait" }
                },
                "required": ["ms"]
            }),
        )
    }

    fn wait_for() -> ToolDefinition {
        ToolDefinition::function(
            "WaitFor",
            "Wait for an element to appear",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                },
                "required": ["selector"]
            }),
        )
    }

    fn wait_for_with_timeout() -> ToolDefinition {
        ToolDefinition::function(
            "WaitForWithTimeout",
            "Wait for an element with timeout",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" },
                    "timeout": { "type": "integer", "description": "Timeout in milliseconds" }
                },
                "required": ["selector", "timeout"]
            }),
        )
    }

    fn wait_for_navigation() -> ToolDefinition {
        ToolDefinition::function(
            "WaitForNavigation",
            "Wait for page navigation to complete",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        )
    }

    fn wait_for_dom() -> ToolDefinition {
        ToolDefinition::function(
            "WaitForDom",
            "Wait for DOM mutations to settle",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "Optional selector to watch" },
                    "timeout": { "type": "integer" }
                },
                "required": ["timeout"]
            }),
        )
    }

    // Navigate
    fn navigate() -> ToolDefinition {
        ToolDefinition::function(
            "Navigate",
            "Navigate to a URL (replaces current page)",
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to navigate to" }
                },
                "required": ["url"]
            }),
        )
    }

    fn open_page() -> ToolDefinition {
        ToolDefinition::function(
            "OpenPage",
            "Open URL in a new tab (for concurrent browsing)",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ],
                        "description": "URL(s) to open in new tabs"
                    }
                },
                "required": ["url"]
            }),
        )
    }

    fn go_back() -> ToolDefinition {
        ToolDefinition::function(
            "GoBack",
            "Go back in browser history",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        )
    }

    fn go_forward() -> ToolDefinition {
        ToolDefinition::function(
            "GoForward",
            "Go forward in browser history",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        )
    }

    fn reload() -> ToolDefinition {
        ToolDefinition::function(
            "Reload",
            "Reload the current page",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        )
    }

    // Advanced
    fn evaluate() -> ToolDefinition {
        ToolDefinition::function(
            "Evaluate",
            "Execute JavaScript code in the page",
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "JavaScript code to execute" }
                },
                "required": ["code"]
            }),
        )
    }

    fn screenshot() -> ToolDefinition {
        ToolDefinition::function(
            "Screenshot",
            "Take a screenshot of the page",
            json!({
                "type": "object",
                "properties": {
                    "full_page": { "type": "boolean", "description": "Capture full page vs viewport" }
                },
                "required": []
            }),
        )
    }
}

/// Parse tool calls from an OpenAI-compatible response.
pub fn parse_tool_calls(response: &Value) -> Vec<ToolCall> {
    let choices = match response.get("choices").and_then(|v| v.as_array()) {
        Some(c) => c,
        None => return Vec::new(),
    };

    let message = match choices.first().and_then(|c| c.get("message")) {
        Some(m) => m,
        None => return Vec::new(),
    };

    let tool_calls = match message.get("tool_calls").and_then(|v| v.as_array()) {
        Some(tc) => tc,
        None => return Vec::new(),
    };

    tool_calls
        .iter()
        .filter_map(|tc| ToolCall::from_json(tc))
        .collect()
}

/// Convert tool calls to automation step actions.
pub fn tool_calls_to_steps(calls: &[ToolCall]) -> Vec<Value> {
    calls.iter().map(|tc| tc.to_action()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_calling_mode() {
        assert!(ToolCallingMode::ToolCalling.should_use_tools("gpt-4"));
        assert!(!ToolCallingMode::JsonObject.should_use_tools("gpt-4"));
        assert!(ToolCallingMode::Auto.should_use_tools("gpt-4-turbo"));
        assert!(ToolCallingMode::Auto.should_use_tools("claude-3-opus"));
        assert!(!ToolCallingMode::Auto.should_use_tools("llama-2-70b"));
    }

    #[test]
    fn test_tool_definition_creation() {
        let tool = ToolDefinition::function(
            "Click",
            "Click an element",
            json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string" }
                }
            }),
        );

        assert_eq!(tool.tool_type, "function");
        assert_eq!(tool.function.name, "Click");
    }

    #[test]
    fn test_all_schemas_generated() {
        let schemas = ActionToolSchemas::all();
        assert!(!schemas.is_empty());

        // Check for some key actions
        let names: Vec<_> = schemas.iter().map(|t| &t.function.name).collect();
        assert!(names.contains(&&"Click".to_string()));
        assert!(names.contains(&&"Fill".to_string()));
        assert!(names.contains(&&"Navigate".to_string()));
        assert!(names.contains(&&"Screenshot".to_string()));
    }

    #[test]
    fn test_tool_call_parsing() {
        let response = json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "Click",
                            "arguments": "{\"selector\": \"button.submit\"}"
                        }
                    }]
                }
            }]
        });

        let calls = parse_tool_calls(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "Click");
    }

    #[test]
    fn test_tool_call_to_action() {
        let call = ToolCall {
            id: "call_1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "Click".to_string(),
                arguments: r#"{"selector": "button"}"#.to_string(),
            },
        };

        let action = call.to_action();
        assert_eq!(action, json!({"Click": "button"}));
    }

    #[test]
    fn test_fill_action_conversion() {
        let call = ToolCall {
            id: "call_2".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "Fill".to_string(),
                arguments: r#"{"selector": "input", "value": "hello"}"#.to_string(),
            },
        };

        let action = call.to_action();
        assert_eq!(
            action,
            json!({"Fill": {"selector": "input", "value": "hello"}})
        );
    }

    #[test]
    fn test_navigate_action_conversion() {
        let call = ToolCall {
            id: "call_3".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "Navigate".to_string(),
                arguments: r#"{"url": "https://example.com"}"#.to_string(),
            },
        };

        let action = call.to_action();
        assert_eq!(action, json!({"Navigate": "https://example.com"}));
    }

    #[test]
    fn test_common_schemas() {
        let common = ActionToolSchemas::common();
        assert!(common.len() < ActionToolSchemas::all().len());
        assert!(common.len() >= 5); // Should have at least the basics
    }
}
