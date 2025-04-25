/// The type of prompt to use.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Prompt {
    /// A single prompt to run.
    Single(String),
    /// Multiple prompts to run after one another.
    Multi(std::collections::VecDeque<String>),
}

impl std::fmt::Display for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Prompt::Single(s) => write!(f, "{}", s),
            Prompt::Multi(m) => write!(f, "{:?}", m),
        }
    }
}

#[cfg(feature = "openai")]
impl std::str::FromStr for Prompt {
    type Err = super::serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(prompts) = super::serde_json::from_str::<Vec<String>>(s) {
            Ok(Prompt::Multi(
                prompts
                    .into_iter()
                    .collect::<std::collections::VecDeque<String>>(),
            ))
        } else {
            Ok(Prompt::Single(s.to_string()))
        }
    }
}

impl Prompt {
    /// A new single prompt.
    pub fn new_single(prompt: &str) -> Self {
        Prompt::Single(prompt.into())
    }
    /// Multiple prompts to run after another.
    pub fn new_multiple(prompt: std::collections::VecDeque<String>) -> Self {
        Prompt::Multi(prompt)
    }
    /// Get the next prompt
    pub fn next(&mut self) -> Option<String> {
        match self {
            Prompt::Single(prompt) => {
                if prompt.is_empty() {
                    None
                } else {
                    Some(prompt.drain(..).collect())
                }
            }
            Prompt::Multi(prompt) => prompt.pop_front(),
        }
    }
}

impl Default for Prompt {
    fn default() -> Self {
        Prompt::Single(Default::default())
    }
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    all(
        not(feature = "regex"),
        not(feature = "openai"),
        not(feature = "cache_openai")
    ),
    derive(PartialEq)
)]
/// Structured data response format.
pub struct ResponseFormatJsonSchema {
    /// A description of what the response format is for, used by the model to determine how to respond in the format.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub description: Option<String>,
    /// The name of the response format. Must be a-z, A-Z, 0-9, or contain underscores and dashes, with a maximum length
    pub name: String,
    /// The schema for the response format, described as a JSON Schema object.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub schema: Option<String>,
    /// Whether to enable strict schema adherence when generating the output. If set to true, the model will always follow the exact schema defined in the `schema` field. Only a subset of JSON Schema is supported when `strict` is `true`. To learn more, read the [Structured Outputs guide](https://platform.openai.com/docs/guides/structured-outputs).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub strict: Option<bool>,
}

/// The GPT configs to use for dynamic Javascript execution and other functionality.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    all(
        not(feature = "regex"),
        not(feature = "openai"),
        not(feature = "cache_openai")
    ),
    derive(PartialEq)
)]
pub struct GPTConfigs {
    /// The prompt to use for the Chat. Example: Search for movies. This will attempt to get the code required to perform the action on the page.
    pub prompt: Prompt,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The model to use. Example: gpt-4-1106-preview or gpt-3.5-turbo-16k
    pub model: String,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The max tokens to use for the request.
    pub max_tokens: u16,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The temperature between 0 - 2.
    pub temperature: Option<f32>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The user for the request.
    pub user: Option<String>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The top priority for the request.
    pub top_p: Option<f32>,
    /// Prompts to use for certain urls. If this is set only the urls that match exactly are ran.
    pub prompt_url_map:
        Option<Box<hashbrown::HashMap<case_insensitive_string::CaseInsensitiveString, Box<Self>>>>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Extra data, this will merge the prompts and try to get the content for you. Example: extracting data from the page.
    pub extra_ai_data: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Map to paths. If the prompt_url_map has a key called /blog and all blog pages are found like /blog/something the same prompt is perform unless an exact match is found.
    pub paths_map: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Take a screenshot of the page after each JS script execution. The screenshot is stored as a base64.
    pub screenshot: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The API key to use for the request.
    pub api_key: Option<String>,
    #[cfg_attr(
        feature = "serde",
        serde(default),
        serde(skip_serializing, skip_deserializing)
    )]
    /// Use caching to cache the prompt. This does nothing without the 'cache_openai' flag enabled.
    pub cache: Option<AICache>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Use structured JSON mode.
    pub json_schema: Option<ResponseFormatJsonSchema>,
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// The usage used from OpenAI.
pub struct OpenAIUsage {
    /// The prompt tokens used.
    pub prompt_tokens: u32,
    /// The completion tokens used.
    pub completion_tokens: u32,
    /// The total tokens used.
    pub total_tokens: u32,
    /// Is the request cached? Useful for ignoring the tokens.
    pub cached: bool,
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// The results from OpenAI.
pub struct OpenAIReturn {
    /// The response of the AI.
    pub response: String,
    /// The usage of the request.
    pub usage: OpenAIUsage,
    /// The error of the request if any.
    pub error: Option<String>,
}

/// The OpenAI return type
// pub type OpenAIReturn = (String, OpenAIUsage);

#[cfg(feature = "cache_openai")]
/// The OpenAI cache to use.
pub type AICache = moka::future::Cache<u64, OpenAIReturn>;

#[cfg(not(feature = "cache_openai"))]
/// The OpenAI cache to use.
pub type AICache = String;

impl GPTConfigs {
    /// GPTConfigs for OpenAI chrome dynamic scripting.
    pub fn new(model: &str, prompt: &str, max_tokens: u16) -> GPTConfigs {
        Self {
            model: model.into(),
            prompt: Prompt::Single(prompt.into()),
            max_tokens,
            ..Default::default()
        }
    }

    /// GPTConfigs for OpenAI chrome dynamic scripting and caching.
    pub fn new_cache(
        model: &str,
        prompt: &str,
        max_tokens: u16,
        cache: Option<AICache>,
    ) -> GPTConfigs {
        Self {
            model: model.into(),
            prompt: Prompt::Single(prompt.into()),
            max_tokens,
            cache,
            ..Default::default()
        }
    }

    /// GPTConfigs for OpenAI chrome dynamic scripting multi chain prompts.
    pub fn new_multi<I, S>(model: &str, prompt: I, max_tokens: u16) -> GPTConfigs
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            model: model.into(),
            prompt: Prompt::Multi(prompt.into_iter().map(|s| s.as_ref().to_string()).collect()),
            max_tokens,
            ..Default::default()
        }
    }

    /// GPTConfigs for OpenAI chrome dynamic scripting multi chain prompts with prompt caching. The feature flag 'cache_openai' is required.
    pub fn new_multi_cache<I, S>(
        model: &str,
        prompt: I,
        max_tokens: u16,
        cache: Option<AICache>,
    ) -> GPTConfigs
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            model: model.into(),
            prompt: Prompt::Multi(prompt.into_iter().map(|s| s.as_ref().to_string()).collect()),
            max_tokens,
            cache,
            ..Default::default()
        }
    }

    /// Set extra AI data to return results.
    pub fn set_extra(&mut self, extra_ai_data: bool) -> &mut Self {
        self.extra_ai_data = extra_ai_data;
        self
    }
}

/// Custom deserialization for `Prompt`
#[cfg(feature = "serde")]
mod prompt_deserializer {
    use super::Prompt;
    use serde::{
        de::{self, SeqAccess, Visitor},
        Deserialize, Deserializer,
    };
    use std::collections::VecDeque;
    use std::fmt;

    struct PromptVisitor;

    impl<'de> Visitor<'de> for PromptVisitor {
        type Value = Prompt;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or an array of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Prompt::Single(value.to_owned()))
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
        where
            S: SeqAccess<'de>,
        {
            let mut strings = VecDeque::new();
            while let Some(value) = seq.next_element()? {
                strings.push_back(value);
            }
            Ok(Prompt::Multi(strings))
        }
    }

    impl<'de> Deserialize<'de> for Prompt {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(PromptVisitor)
        }
    }
}

#[test]
#[cfg(feature = "openai")]
fn deserialize_gpt_configs() {
    let gpt_configs_json = "{\"prompt\":\"change background blue\",\"model\":\"gpt-3.5-turbo-16k\",\"max_tokens\":256,\"temperature\":0.54,\"top_p\":0.17}";
    let configs = match crate::features::serde_json::from_str::<GPTConfigs>(&gpt_configs_json) {
        Ok(e) => Some(e),
        _ => None,
    };
    assert!(configs.is_some())
}
