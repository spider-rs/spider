//! Helper functions for JSON parsing and text processing.
//!
//! Contains utilities for handling LLM output parsing, including
//! best-effort JSON extraction from markdown code blocks.

use super::{AutomationUsage, EngineError, EngineResult};
use serde_json::Value;

/// Extract the assistant's text content from an OpenAI-compatible response.
///
/// Handles various response formats:
/// - Standard `choices[0].message.content` string
/// - Array of content blocks with `text` or `content` fields
/// - `output_text` field (some providers)
pub fn extract_assistant_content(root: &Value) -> Option<String> {
    let choice0 = root.get("choices")?.as_array()?.first()?;
    let msg = choice0.get("message").or_else(|| choice0.get("delta"))?;

    if let Some(c) = msg.get("content") {
        if let Some(s) = c.as_str() {
            return Some(s.to_string());
        }
        if let Some(arr) = c.as_array() {
            let mut out = String::new();
            for block in arr {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                } else if let Some(t) = block.get("content").and_then(|v| v.as_str()) {
                    out.push_str(t);
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }
    }

    root.get("output_text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract token usage from an OpenAI-compatible response.
///
/// The response format follows the OpenAI API structure:
/// ```json
/// {
///   "usage": {
///     "prompt_tokens": 123,
///     "completion_tokens": 456,
///     "total_tokens": 579
///   }
/// }
/// ```
///
/// Returns a default `AutomationUsage` if the usage field is missing or malformed.
pub fn extract_usage(root: &Value) -> AutomationUsage {
    let usage = match root.get("usage") {
        Some(u) => u,
        None => return AutomationUsage::default(),
    };

    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    // total_tokens from the response (used for verification if needed)
    let _total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| (prompt_tokens + completion_tokens) as u64) as u32;

    AutomationUsage::new(prompt_tokens, completion_tokens)
}

/// Extract the LAST ```json``` or ``` code block from text.
///
/// Thinking/reasoning models often output multiple blocks, refining their answer.
/// The last block is typically the final, valid JSON.
pub fn extract_last_code_block(s: &str) -> Option<&str> {
    let mut last_block: Option<&str> = None;
    let mut search_start = 0;

    // Find all ```json blocks and keep track of the last one
    while let Some(rel_start) = s[search_start..].find("```json") {
        let abs_start = search_start + rel_start + 7; // skip "```json"
        if abs_start < s.len() {
            if let Some(rel_end) = s[abs_start..].find("```") {
                let block = s[abs_start..abs_start + rel_end].trim();
                if !block.is_empty() {
                    last_block = Some(block);
                }
                search_start = abs_start + rel_end + 3;
            } else {
                // No closing fence, take rest of string
                let block = s[abs_start..].trim();
                if !block.is_empty() {
                    last_block = Some(block);
                }
                break;
            }
        } else {
            break;
        }
    }

    // If no ```json found, try generic ``` blocks
    if last_block.is_none() {
        search_start = 0;
        while let Some(rel_start) = s[search_start..].find("```") {
            let after_fence = search_start + rel_start + 3;
            if after_fence >= s.len() {
                break;
            }

            // Skip language identifier if present (e.g., ```javascript)
            let rest = &s[after_fence..];
            let content_start = rest
                .find('\n')
                .map(|i| after_fence + i + 1)
                .unwrap_or(after_fence);

            if content_start < s.len() {
                if let Some(rel_end) = s[content_start..].find("```") {
                    let block = s[content_start..content_start + rel_end].trim();
                    // Only consider blocks that look like JSON
                    if !block.is_empty() && (block.starts_with('{') || block.starts_with('[')) {
                        last_block = Some(block);
                    }
                    search_start = content_start + rel_end + 3;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    last_block
}

/// Extract the last balanced JSON object or array from text.
///
/// Uses proper brace matching to handle nested structures.
/// Returns the byte range (start, end) of the extracted JSON.
pub fn extract_last_json_boundaries(s: &str, open: char, close: char) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let open_byte = open as u8;
    let close_byte = close as u8;

    // Find the last closing brace/bracket
    let mut end_pos = None;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == close_byte {
            end_pos = Some(i);
            break;
        }
    }

    let end_pos = end_pos?;

    // Walk backwards from end_pos, counting braces to find the matching opener
    let mut depth = 0i32;
    let mut in_string = false;
    let mut pos = end_pos + 1;

    while pos > 0 {
        pos -= 1;
        let ch = bytes[pos];

        if ch == b'"' && !is_escaped(bytes, pos) {
            in_string = !in_string;
            continue;
        }

        if in_string {
            continue;
        }

        if ch == close_byte {
            depth += 1;
        } else if ch == open_byte {
            depth -= 1;
            if depth == 0 {
                return Some((pos, end_pos + 1));
            }
        }
    }

    None
}

/// Check if a quote at position is escaped by counting preceding backslashes.
fn is_escaped(bytes: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return false;
    }

    let mut backslash_count = 0;
    let mut check_pos = pos - 1;
    while bytes[check_pos] == b'\\' {
        backslash_count += 1;
        if check_pos == 0 {
            break;
        }
        check_pos -= 1;
    }

    // Odd number of backslashes means the quote is escaped
    backslash_count % 2 == 1
}

/// Extract the last JSON object from text with proper brace matching.
pub fn extract_last_json_object(s: &str) -> Option<&str> {
    extract_last_json_boundaries(s, '{', '}').map(|(start, end)| &s[start..end])
}

/// Extract the last JSON array from text with proper brace matching.
pub fn extract_last_json_array(s: &str) -> Option<&str> {
    extract_last_json_boundaries(s, '[', ']').map(|(start, end)| &s[start..end])
}

/// Best effort parse the JSON object.
///
/// Handles common LLM output quirks:
/// - Multiple ```json``` blocks (uses the LAST one, as thinking models refine answers)
/// - Reasoning/thinking text before JSON
/// - Nested JSON structures (proper brace matching)
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

    log::warn!(
        "best_effort_parse_json_object failed on content (first 300 chars): {}",
        &s[..s.len().min(300)]
    );

    Err(EngineError::InvalidField(
        "assistant content was not a JSON object",
    ))
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

/// Take the last `max_bytes` of a UTF-8 string without splitting code points.
///
/// Returns a string with a `...[truncated]...` prefix when truncated.
pub fn truncate_utf8_tail(s: &str, max_bytes: usize) -> String {
    let bytes = s.as_bytes();
    if bytes.len() <= max_bytes {
        return s.to_string();
    }

    let mut start = bytes.len().saturating_sub(max_bytes);
    while start < bytes.len() && !s.is_char_boundary(start) {
        start += 1;
    }

    let tail = &s[start..];
    let mut out = String::with_capacity(tail.len() + 20);
    out.push_str("...[truncated]...");
    out.push_str(tail);
    out
}

/// FNV-1a 64-bit hash function for cheap content hashing.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_utf8_tail() {
        let s = "Hello, World!";
        assert_eq!(truncate_utf8_tail(s, 100), s);

        let truncated = truncate_utf8_tail(s, 5);
        assert!(truncated.starts_with("...[truncated]..."));
        assert!(truncated.ends_with("orld!"));
    }

    #[test]
    fn test_extract_last_json_object() {
        let s = "Some text before {\"key\": \"value\"} and after";
        let json = extract_last_json_object(s);
        assert_eq!(json, Some("{\"key\": \"value\"}"));
    }

    #[test]
    fn test_extract_last_code_block() {
        let s = "```json\n{\"a\": 1}\n```\nSome text\n```json\n{\"b\": 2}\n```";
        let block = extract_last_code_block(s);
        assert_eq!(block, Some("{\"b\": 2}"));
    }

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
            "Here's my thinking...\n\nThe answer is: {\"key\": \"value\"}"
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["key"], "value");
    }

    #[test]
    fn test_repair_json_braces_evaluate() {
        // LLM outputs `}}` instead of `}` after Evaluate strings containing JS code with braces
        let broken = r#"{"label":"test","done":false,"steps":[{"Evaluate":"document.title = JSON.stringify({a:1});"}},{"Wait":300}],"extracted":{"level":7}}"#;
        let result = best_effort_parse_json_object(broken);
        assert!(result.is_ok(), "Should repair double-brace error: {:?}", result.err());
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
    fn test_fnv1a64() {
        let hash = fnv1a64(b"hello");
        assert_ne!(hash, 0);

        // Same input should produce same hash
        assert_eq!(fnv1a64(b"hello"), fnv1a64(b"hello"));

        // Different input should produce different hash
        assert_ne!(fnv1a64(b"hello"), fnv1a64(b"world"));
    }
}
