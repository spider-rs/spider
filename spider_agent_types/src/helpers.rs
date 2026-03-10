//! Helper functions for JSON parsing and text processing.
//!
//! Contains utilities for handling LLM output parsing, including
//! best-effort JSON extraction from markdown code blocks.

use crate::AutomationUsage;
use serde_json::Value;

/// Extract the assistant's text content from an LLM API response.
///
/// Handles various response formats (tried in order):
/// - OpenAI: `choices[0].message.content` string or array of content blocks
/// - Anthropic Messages API: root-level `content` array with `type: "text"` blocks
/// - `output_text` field (some providers)
pub fn extract_assistant_content(root: &Value) -> Option<String> {
    // 1. OpenAI-compatible: choices[0].message.content
    if let Some(choices) = root.get("choices").and_then(|v| v.as_array()) {
        if let Some(choice0) = choices.first() {
            let msg = choice0.get("message").or_else(|| choice0.get("delta"));
            if let Some(msg) = msg {
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
            }
        }
    }

    // 2. Anthropic Messages API: root.content[] with type:"text"
    if let Some(content_arr) = root.get("content").and_then(|v| v.as_array()) {
        let mut out = String::new();
        for block in content_arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                }
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }

    // 3. output_text fallback (some providers)
    root.get("output_text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract thinking/reasoning content from an LLM API response.
///
/// Handles:
/// - Anthropic: root-level `content` array with `type: "thinking"` blocks
/// - OpenAI: `choices[0].message.reasoning_content` field
///
/// Returns `None` if no thinking content is present.
pub fn extract_thinking_content(root: &Value) -> Option<String> {
    // Anthropic: content[] with type:"thinking"
    if let Some(content_arr) = root.get("content").and_then(|v| v.as_array()) {
        let mut out = String::new();
        for block in content_arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("thinking") {
                if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }

    // OpenAI: choices[0].message.reasoning_content
    if let Some(choices) = root.get("choices").and_then(|v| v.as_array()) {
        if let Some(choice0) = choices.first() {
            if let Some(msg) = choice0.get("message") {
                if let Some(s) = msg.get("reasoning_content").and_then(|v| v.as_str()) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }

    None
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
        // Count the LLM call even when the provider omits usage details.
        None => return AutomationUsage::with_api_calls(0, 0, 1),
    };

    // OpenAI: prompt_tokens / completion_tokens
    // Anthropic: input_tokens / output_tokens
    let prompt_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let completion_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    // total_tokens from the response (used for verification if needed)
    let _total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| (prompt_tokens + completion_tokens) as u64)
        as u32;

    AutomationUsage::with_api_calls(prompt_tokens, completion_tokens, 1)
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
    fn test_fnv1a64() {
        let hash = fnv1a64(b"hello");
        assert_ne!(hash, 0);

        // Same input should produce same hash
        assert_eq!(fnv1a64(b"hello"), fnv1a64(b"hello"));

        // Different input should produce different hash
        assert_ne!(fnv1a64(b"hello"), fnv1a64(b"world"));
    }

    // ── Anthropic Messages API format tests ──

    #[test]
    fn test_extract_assistant_content_anthropic() {
        let resp = serde_json::json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "thinking", "thinking": "Let me reason..."},
                {"type": "text", "text": "{\"label\": \"test\"}"}
            ]
        });
        let content = extract_assistant_content(&resp);
        assert_eq!(content, Some("{\"label\": \"test\"}".to_string()));
    }

    #[test]
    fn test_extract_assistant_content_anthropic_multi_text() {
        let resp = serde_json::json!({
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world"}
            ]
        });
        assert_eq!(
            extract_assistant_content(&resp),
            Some("Hello world".to_string())
        );
    }

    #[test]
    fn test_extract_assistant_content_openai_still_works() {
        let resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "hello world"
                }
            }]
        });
        assert_eq!(
            extract_assistant_content(&resp),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_extract_thinking_content_anthropic() {
        let resp = serde_json::json!({
            "content": [
                {"type": "thinking", "thinking": "Step 1: analyze"},
                {"type": "thinking", "thinking": "Step 2: decide"},
                {"type": "text", "text": "{\"label\": \"done\"}"}
            ]
        });
        let thinking = extract_thinking_content(&resp);
        assert_eq!(
            thinking,
            Some("Step 1: analyze\nStep 2: decide".to_string())
        );
    }

    #[test]
    fn test_extract_thinking_content_openai_reasoning() {
        let resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "{\"done\": true}",
                    "reasoning_content": "I need to think about this..."
                }
            }]
        });
        let thinking = extract_thinking_content(&resp);
        assert_eq!(thinking, Some("I need to think about this...".to_string()));
    }

    #[test]
    fn test_extract_thinking_content_none_when_absent() {
        let resp = serde_json::json!({
            "choices": [{
                "message": { "content": "hello" }
            }]
        });
        assert!(extract_thinking_content(&resp).is_none());
    }

    #[test]
    fn test_extract_usage_anthropic() {
        let resp = serde_json::json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50
            }
        });
        let usage = extract_usage(&resp);
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.api_calls, 1);
    }

    #[test]
    fn test_extract_usage_openai_still_works() {
        let resp = serde_json::json!({
            "usage": {
                "prompt_tokens": 200,
                "completion_tokens": 75,
                "total_tokens": 275
            }
        });
        let usage = extract_usage(&resp);
        assert_eq!(usage.prompt_tokens, 200);
        assert_eq!(usage.completion_tokens, 75);
    }

    #[test]
    fn test_extract_assistant_content_prefers_openai_over_anthropic() {
        // If both formats are present (unlikely but tests priority)
        let resp = serde_json::json!({
            "choices": [{"message": {"content": "openai"}}],
            "content": [{"type": "text", "text": "anthropic"}]
        });
        assert_eq!(extract_assistant_content(&resp), Some("openai".to_string()));
    }
}
