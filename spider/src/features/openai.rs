use async_openai::types::ChatCompletionRequestSystemMessageArgs;
use tiktoken_rs::{get_chat_completion_max_tokens, ChatCompletionRequestMessage};

lazy_static! {
    static ref BROWSER_ACTIONS_SYSTEM_PROMPT: async_openai::types::ChatCompletionRequestMessage = {
        ChatCompletionRequestSystemMessageArgs::default()
                .content(r#"You are an expert assistant that is dedicated to curating valid Javascript to use for a website. 
You will receive the website HTML from an assistant and a user prompt on what actions to do. 
ONLY RESPOND WITH VALID JAVASCRIPT STRING."#.trim())
                .build()
                .unwrap()
                .into()
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
