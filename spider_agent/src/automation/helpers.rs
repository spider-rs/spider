//! Helper functions for JSON parsing that depend on [`EngineError`].
//!
//! Contains [`best_effort_parse_json_object`] and supporting repair utilities
//! for robustly handling LLM output quirks. Pure parsing helpers (code-block
//! extraction, JSON boundary detection, truncation, hashing) live in
//! [`spider_agent_types`] and are re-exported at the parent module level.

use super::{
    extract_last_code_block, extract_last_json_array, extract_last_json_object, EngineError,
    EngineResult,
};
use serde_json::Value;

/// Best effort parse the JSON object.
///
/// Handles common LLM output quirks:
/// - Multiple ```json``` blocks (uses the LAST one, as thinking models refine answers)
/// - Reasoning/thinking text before JSON
/// - Nested JSON structures (proper brace matching)
/// - Duplicate closing braces from JS code inside JSON strings
/// - Unquoted bare-word values from small models
pub fn best_effort_parse_json_object(s: &str) -> EngineResult<Value> {
    // Try direct parse first
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Ok(v);
    }

    let trimmed = s.trim();

    // 0. Repair common LLM JSON errors: duplicate closing braces after strings in arrays.
    // LLMs often output `"}},` instead of `"},` when JS code with braces is inside a JSON string.
    // This happens because the model confuses JS braces with JSON structure braces.
    let repaired = repair_json_braces(trimmed);
    if let Ok(v) = serde_json::from_str::<Value>(&repaired) {
        return Ok(v);
    }

    // 1. Try to extract the LAST code block (thinking models refine their answer)
    if let Some(block) = extract_last_code_block(trimmed) {
        if let Ok(v) = serde_json::from_str::<Value>(block) {
            return Ok(v);
        }

        // Try repair on the code block too
        let repaired_block = repair_json_braces(block);
        if let Ok(v) = serde_json::from_str::<Value>(&repaired_block) {
            return Ok(v);
        }

        // The code block might have prose - try extracting JSON from within it
        if let Some(obj) = extract_last_json_object(block) {
            if let Ok(v) = serde_json::from_str::<Value>(obj) {
                return Ok(v);
            }
        }
    }

    // 2. Strip markdown fences if at boundaries
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|x| x.trim())
        .unwrap_or(trimmed);
    let unfenced = unfenced
        .strip_suffix("```")
        .map(|x| x.trim())
        .unwrap_or(unfenced);

    if let Ok(v) = serde_json::from_str::<Value>(unfenced) {
        return Ok(v);
    }

    // Try repair on unfenced content
    let repaired_unfenced = repair_json_braces(unfenced);
    if let Ok(v) = serde_json::from_str::<Value>(&repaired_unfenced) {
        return Ok(v);
    }

    // 3. Extract last JSON object with proper brace matching
    if let Some(obj) = extract_last_json_object(unfenced) {
        if let Ok(v) = serde_json::from_str::<Value>(obj) {
            return Ok(v);
        }
    }

    // 4. Extract last JSON array with proper bracket matching
    if let Some(arr) = extract_last_json_array(unfenced) {
        if let Ok(v) = serde_json::from_str::<Value>(arr) {
            return Ok(v);
        }
    }

    // 5. Repair unquoted bare-word values (common with small models like Gemini Nano)
    //    e.g. {"selector": a, "action": "click"} → {"selector": "a", "action": "click"}
    let quoted = repair_unquoted_json_values(unfenced);
    if let Ok(v) = serde_json::from_str::<Value>(&quoted) {
        return Ok(v);
    }
    // Also try on extracted JSON objects
    if let Some(obj) = extract_last_json_object(&quoted) {
        if let Ok(v) = serde_json::from_str::<Value>(obj) {
            return Ok(v);
        }
    }

    log::warn!(
        "best_effort_parse_json_object failed on content (first 300 chars): {}",
        &s[..s.len().min(300)]
    );

    Err(EngineError::InvalidField(
        "assistant content was not a JSON object",
    ))
}

/// Repair unquoted bare-word values in JSON — common with small models.
///
/// Converts `"key": bare_value,` → `"key": "bare_value",`
/// Only handles simple cases outside of nested structures.
fn repair_unquoted_json_values(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut result = Vec::with_capacity(len + 64);
    let mut i = 0;
    let mut in_string = false;
    let mut escape_next = false;

    while i < len {
        let b = bytes[i];

        if escape_next {
            result.push(b);
            escape_next = false;
            i += 1;
            continue;
        }

        if b == b'\\' && in_string {
            result.push(b);
            escape_next = true;
            i += 1;
            continue;
        }

        if b == b'"' {
            in_string = !in_string;
            result.push(b);
            i += 1;
            continue;
        }

        if in_string {
            result.push(b);
            i += 1;
            continue;
        }

        // Outside a string — look for `:` followed by an unquoted bare word
        if b == b':' {
            result.push(b);
            i += 1;
            // Skip whitespace after colon
            while i < len && bytes[i].is_ascii_whitespace() {
                result.push(bytes[i]);
                i += 1;
            }
            if i >= len {
                break;
            }
            let c = bytes[i];
            // Valid JSON value starts: " [ { digit - true false null
            if c == b'"' || c == b'[' || c == b'{' || c.is_ascii_digit() || c == b'-' {
                continue;
            }
            let remaining = &s[i..];
            if remaining.starts_with("true")
                || remaining.starts_with("false")
                || remaining.starts_with("null")
            {
                continue;
            }
            // Bare word — collect until , } ] or newline
            let word_start = i;
            while i < len {
                let ch = bytes[i];
                if ch == b',' || ch == b'}' || ch == b']' || ch == b'\n' || ch == b'\r' {
                    break;
                }
                i += 1;
            }
            let word = s[word_start..i].trim();
            if !word.is_empty() {
                result.push(b'"');
                // Escape any quotes inside the bare word
                for &wb in word.as_bytes() {
                    if wb == b'"' {
                        result.push(b'\\');
                    }
                    result.push(wb);
                }
                result.push(b'"');
            }
            continue;
        }

        result.push(b);
        i += 1;
    }

    String::from_utf8(result).unwrap_or_else(|_| s.to_string())
}

/// Repair common JSON brace errors produced by LLMs.
///
/// When LLMs embed JavaScript code (with curly braces) inside JSON strings,
/// they often add extra closing braces, producing patterns like `"}}` instead of `"}`.
/// This function attempts to fix these by removing duplicate braces that break JSON structure.
fn repair_json_braces(s: &str) -> String {
    // Strategy: walk the string tracking JSON structure (respecting string boundaries).
    // When we see `"}}"` followed by `,` or `]`, the inner `}}` likely has one extra `}`.
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut result = Vec::with_capacity(len);
    let mut i = 0;
    let mut in_string = false;
    let mut escape_next = false;

    while i < len {
        let b = bytes[i];

        if escape_next {
            escape_next = false;
            result.push(b);
            i += 1;
            continue;
        }

        if b == b'\\' && in_string {
            escape_next = true;
            result.push(b);
            i += 1;
            continue;
        }

        if b == b'"' {
            in_string = !in_string;
            result.push(b);
            i += 1;
            continue;
        }

        if in_string {
            result.push(b);
            i += 1;
            continue;
        }

        // Outside a string: look for `}}` patterns that are likely errors.
        // Pattern: `}` followed by `}` then `,` or `]` or `}`
        // If the original string doesn't parse but removing one `}` would fix it,
        // skip one `}`.
        if b == b'}' && i + 1 < len && bytes[i + 1] == b'}' {
            // Look ahead past the `}}` for `,` or `]`
            let after = if i + 2 < len { bytes[i + 2] } else { 0 };
            if after == b',' || after == b']' || after == b'}' {
                // Skip one `}` - keep only one
                result.push(b'}');
                i += 2; // skip both `}`, but only emit one
                continue;
            }
        }

        result.push(b);
        i += 1;
    }

    String::from_utf8(result).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_best_effort_parse_simple() {
        let result = best_effort_parse_json_object("{\"key\": \"value\"}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["key"], "value");
    }

    #[test]
    fn test_best_effort_parse_with_fences() {
        let result = best_effort_parse_json_object("```json\n{\"key\": \"value\"}\n```");
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["key"], "value");
    }

    #[test]
    fn test_best_effort_parse_with_prose() {
        let result = best_effort_parse_json_object(
            "Here's my thinking...\n\nThe answer is: {\"key\": \"value\"}",
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["key"], "value");
    }

    #[test]
    fn test_repair_json_braces_evaluate() {
        // LLM outputs `}}` instead of `}` after Evaluate strings containing JS code with braces
        let broken = r#"{"label":"test","done":false,"steps":[{"Evaluate":"document.title = JSON.stringify({a:1});"}},{"Wait":300}],"extracted":{"level":7}}"#;
        let result = best_effort_parse_json_object(broken);
        assert!(
            result.is_ok(),
            "Should repair double-brace error: {:?}",
            result.err()
        );
        let val = result.unwrap();
        assert_eq!(val["label"], "test");
        let steps = val["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2);
        assert!(steps[0].get("Evaluate").is_some());
        assert!(steps[1].get("Wait").is_some());
    }

    #[test]
    fn test_repair_json_braces_valid_not_broken() {
        // Valid JSON with nested objects should NOT be broken by repair
        let valid = r#"{"a":{"b":{"c":1}},"d":[1,2]}"#;
        let result = best_effort_parse_json_object(valid);
        assert!(result.is_ok());
        let val = result.unwrap();
        assert_eq!(val["a"]["b"]["c"], 1);
    }

    #[test]
    fn test_repair_unquoted_json_values_bare_word() {
        // Common small model output: "selector": a instead of "selector": "a"
        let input = r#"{"label": "Click the checkbox", "selector": a, "action": "click"}"#;
        let repaired = repair_unquoted_json_values(input);
        let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        assert_eq!(parsed["selector"], "a");
        assert_eq!(parsed["action"], "click");
        assert_eq!(parsed["label"], "Click the checkbox");
    }

    #[test]
    fn test_repair_unquoted_json_values_multiple() {
        let input = r#"{"a": hello, "b": world, "c": 42}"#;
        let repaired = repair_unquoted_json_values(input);
        let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        assert_eq!(parsed["a"], "hello");
        assert_eq!(parsed["b"], "world");
        assert_eq!(parsed["c"], 42);
    }

    #[test]
    fn test_repair_unquoted_json_values_preserves_valid() {
        let input = r#"{"a": "quoted", "b": 123, "c": true, "d": null, "e": [1]}"#;
        let repaired = repair_unquoted_json_values(input);
        let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        assert_eq!(parsed["a"], "quoted");
        assert_eq!(parsed["b"], 123);
        assert_eq!(parsed["c"], true);
        assert!(parsed["d"].is_null());
    }

    #[test]
    fn test_best_effort_parse_unquoted_values() {
        // End-to-end: markdown-fenced response with unquoted bare word
        let input = "```json\n{\"label\": \"Click the checkbox\", \"selector\": a, \"action\": \"click\"}\n```";
        let result = best_effort_parse_json_object(input);
        assert!(
            result.is_ok(),
            "Should parse unquoted bare word: {:?}",
            result.err()
        );
        let val = result.unwrap();
        assert_eq!(val["selector"], "a");
        assert_eq!(val["action"], "click");
    }
}
