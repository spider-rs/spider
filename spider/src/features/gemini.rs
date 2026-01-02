//! Gemini-specific prompts and utilities

use lazy_static::lazy_static;

const PROMPT: &str = r#"task-js-snippet-web-int.
HTML, URL, user-prompt-action -> provide pure js.
Exec-in-browser, no extra fmt/annot.
Only raw-js for function/applic.
Ex: window.location.href='https://www.google.com/search?q=Movies';"#;

const PROMPT_EXTRA: &str = r#"Provide a JSON response, e.g., {"content": ["Something"], "js": "window.location.href = 'https://www.google.com/search?q=Movies';"}. Use this structure. If no JS is needed, set "js" to ""."#;

lazy_static! {
    /// The base system prompt for driving the browser.
    pub static ref BROWSER_ACTIONS_SYSTEM_PROMPT: String = PROMPT.trim().to_string();
    /// The base system prompt for extra data.
    pub static ref BROWSER_ACTIONS_SYSTEM_EXTRA_PROMPT: String = PROMPT_EXTRA.trim().to_string();
}

/// Estimate token count for Gemini (rough approximation).
/// Gemini uses a different tokenizer, but ~4 chars per token is a reasonable estimate.
pub fn estimate_token_count(text: &str) -> usize {
    text.len() / 4
}

/// Calculate max tokens available for response given input size.
pub fn calculate_max_tokens(
    model_name: &str,
    max_tokens: u16,
    resource: &str,
    prompt: &str,
) -> usize {
    // Gemini model context windows (approximate)
    let model_max: usize = if model_name.contains("1.5-pro")
        || model_name.contains("2.0")
        || model_name.contains("2.5")
        || model_name.contains("1.5-flash")
        || model_name.contains("flash")
    {
        1_000_000 // 1M context for Gemini 1.5 Pro, 2.0+, and flash models
    } else {
        32_000 // Default conservative estimate
    };

    let system_tokens = estimate_token_count(&BROWSER_ACTIONS_SYSTEM_PROMPT);
    let resource_tokens = estimate_token_count(resource);
    let prompt_tokens = estimate_token_count(prompt);
    let input_tokens = system_tokens + resource_tokens + prompt_tokens;

    let available = model_max.saturating_sub(input_tokens);
    available.min(max_tokens as usize)
}
