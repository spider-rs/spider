use async_openai::types::ChatCompletionRequestSystemMessageArgs;
use tiktoken_rs::{get_chat_completion_max_tokens, ChatCompletionRequestMessage};

const PROMPT: &str = r#"task-js-snippet-web-int.\n
HTML, URL, user-prompt-action -> provide pure js.\n
Exec-in-browser, no extra fmt/annot.\n
Only raw-js for function/applic.\n
Ex: window.location.href='https://www.google.com/search?q=Movies';"#;

const PROMPT_EXTRA: &str = r#"Provide a JSON response, e.g., {"content": ["Something"], "js": "window.location.href = 'https://www.google.com/search?q=Movies';"}. Use this structure. If no JS is needed, set "js" to ""."#;

lazy_static! {
    /// The base system prompt for driving the browser.
    pub static ref BROWSER_ACTIONS_SYSTEM_PROMPT: async_openai::types::ChatCompletionRequestMessage = {
        ChatCompletionRequestSystemMessageArgs::default()
                .content(PROMPT.trim())
                .build()
                .unwrap()
                .into()
    };
    /// The base system prompt for extra data.
    pub static ref BROWSER_ACTIONS_SYSTEM_EXTRA_PROMPT: async_openai::types::ChatCompletionRequestMessage = {
        ChatCompletionRequestSystemMessageArgs::default()
                .content(PROMPT_EXTRA.trim())
                .build()
                .unwrap()
                .into()
    };
    /// The prompt completion for tiktoken token counting.
    pub static ref BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION: tiktoken_rs::ChatCompletionRequestMessage = {
       tiktoken_rs::ChatCompletionRequestMessage {
            content: Some(PROMPT.trim().to_string()),
            role: "system".to_string(),
            name: None,
            function_call: None,
       }
    };
}

/// calculate the max tokens for a request
pub fn calculate_max_tokens(
    model_name: &str,
    max_tokens: u16,
    base_chat_prompt: &ChatCompletionRequestMessage,
    resource: &str,
    prompt: &str,
) -> usize {
    let messages = if prompt.is_empty() {
        vec![
            base_chat_prompt.clone(),
            ChatCompletionRequestMessage {
                content: Some(resource.to_string()),
                role: "assistant".to_string(),
                name: None,
                function_call: None,
            },
        ]
    } else {
        vec![
            base_chat_prompt.clone(),
            ChatCompletionRequestMessage {
                content: Some(resource.to_string()),
                role: "assistant".to_string(),
                name: None,
                function_call: None,
            },
            ChatCompletionRequestMessage {
                content: Some(prompt.to_string()),
                role: "user".to_string(),
                name: None,
                function_call: None,
            },
        ]
    };
    let max_tokens = match get_chat_completion_max_tokens(&model_name, &messages) {
        Ok(r) => r,
        _ => max_tokens.into(),
    }
    .min(max_tokens.into());

    max_tokens
}
