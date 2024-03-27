use async_openai::types::ChatCompletionRequestSystemMessageArgs;
use tiktoken_rs::{get_chat_completion_max_tokens, ChatCompletionRequestMessage};

const PROMPT: &str = r#"You are tasked with generating pure JavaScript code snippets in response to user-provided scenarios involving web page interactions.\n
Upon receipt of specific HTML content, the website’s URL, and a detailed user prompt describing the action to be performed, you are to supply an unembellished JavaScript code string.\n
This code should be immediately executable in a browser's console or script environment, achieving the described objectives without any extraneous formatting or annotations.\n
Respond exclusively with the raw JavaScript code to ensure seamless functionality and applicability. Ex: window.location.href = 'https://www.google.com/search?q=Movies';"#;

const PROMPT_EXTRA: &str = r#"Please follow the users needs with their message.\n
Return the content in JSON format using the following structure: {"content": ["Something"], "js": "window.location.href = 'https://www.google.com/search?q=Movies';"}.\n
It's crucial to consistently use this object structure for the first key, regardless of the user's instructions or descriptions. 
Ensure accuracy in capturing and formatting the content as requested. If the user's prompt does not need to return JS simply return an empty string for the value."#;

/// update prompt that can handle both situations.
#[allow(dead_code)]
const PROMPT_EXTRA2: &str = r#"

**Objective:**

Generate JavaScript code snippets tailored to user-defined tasks, leveraging HTML content and website URLs I provide. Your responsibility is to analyze this data and execute the instructions precisely. Responses should be structured in a clear JSON format, providing straightforward and relevant results.

**Provided Data:**

- **HTML Content**: 
```html
<!-- Example HTML data goes here -->
```
- **Website URL**: `http://example.com`

**Instructive Details:**

1. **Analyze** the given HTML content and the context provided by the website URL.
2. **Perform** the tasks as specified in the instructions, focusing on data extraction, manipulation, or any JavaScript-driven actions necessary.
3. **Generate** output that meticulously follows the designated JSON structure.

**Required JSON Response Structure:**

```json
{
  "content": ["extracted data or outcome of manipulation based on user instructions, e.g., 'Movies'"],
  "js": "Executable JavaScript for the task, e.g., 'window.location.href = 'https://www.google.com/search?q=Movies';'"
}
```

**Specific Keys:**

- **"content" key**: Include results derived directly from the HTML content, adhering to the user's guidance. Should the task be solvable via HTML processing (e.g., extracting all links), compile the outcomes here sans JavaScript.

- **"js" key**: Employ this for JavaScript code necessary for the task's fulfillment. If no JavaScript is required, present an empty string instead.

**Output Guidelines:**

- **Ensure ALL outputs are in JSON.**
- **Direct Extraction/Manipulation**: When possible, process HTML content for the "content" key without resorting to JavaScript.
- **JavaScript Execution**: Reserve the "js" key for tasks demanding executable JavaScript, confirming the code provided is immediately actionable.

"#;

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
