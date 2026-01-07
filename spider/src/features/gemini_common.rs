//! Common Gemini types and configurations

use crate::features::openai_common::{Prompt, ResponseFormatJsonSchema};

/// Default Gemini model - uses the latest flash model shortcut
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";

/// The Gemini configs to use for dynamic Javascript execution and other functionality.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    all(
        not(feature = "regex"),
        not(feature = "openai"),
        not(feature = "cache_openai"),
        not(feature = "gemini"),
        not(feature = "cache_gemini")
    ),
    derive(PartialEq)
)]
pub struct GeminiConfigs {
    /// The prompt to use for the Chat. Example: Search for movies. This will attempt to get the code required to perform the action on the page.
    pub prompt: Prompt,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The model to use. Example: gemini-flash-latest, gemini-2.0-flash, gemini-1.5-pro
    pub model: String,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The max tokens to use for the request (max_output_tokens in Gemini).
    pub max_tokens: u16,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The temperature between 0 - 2.
    pub temperature: Option<f32>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The top_p for the request.
    pub top_p: Option<f32>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The top_k for the request (Gemini-specific).
    pub top_k: Option<i32>,
    /// Prompts to use for certain urls. If this is set only the urls that match exactly are ran.
    pub prompt_url_map:
        Option<Box<hashbrown::HashMap<case_insensitive_string::CaseInsensitiveString, Box<Self>>>>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Extra data, this will merge the prompts and try to get the content for you. Example: extracting data from the page.
    pub extra_ai_data: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Map to paths. If the prompt_url_map has a key called /blog and all blog pages are found like /blog/something the same prompt is performed unless an exact match is found.
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
    /// Use caching to cache the prompt. This does nothing without the 'cache_gemini' flag enabled.
    pub cache: Option<GeminiCache>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Use structured JSON mode.
    pub json_schema: Option<ResponseFormatJsonSchema>,
}

/// The usage used from Gemini.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GeminiUsage {
    /// The prompt tokens used.
    pub prompt_tokens: u32,
    /// The completion tokens used.
    pub completion_tokens: u32,
    /// The total tokens used.
    pub total_tokens: u32,
    /// Is the request cached? Useful for ignoring the tokens.
    pub cached: bool,
}

/// The results from Gemini.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GeminiReturn {
    /// The response of the AI.
    pub response: String,
    /// The usage of the request.
    pub usage: GeminiUsage,
    /// The error of the request if any.
    pub error: Option<String>,
}

#[cfg(feature = "cache_gemini")]
/// The Gemini cache to use.
pub type GeminiCache = moka::future::Cache<u64, GeminiReturn>;

#[cfg(not(feature = "cache_gemini"))]
/// The Gemini cache to use.
pub type GeminiCache = String;

impl GeminiConfigs {
    /// GeminiConfigs for Gemini chrome dynamic scripting.
    pub fn new(model: &str, prompt: &str, max_tokens: u16) -> GeminiConfigs {
        Self {
            model: model.into(),
            prompt: Prompt::Single(prompt.into()),
            max_tokens,
            ..Default::default()
        }
    }

    /// GeminiConfigs with default model (gemini-flash-latest).
    pub fn new_default(prompt: &str, max_tokens: u16) -> GeminiConfigs {
        Self::new(DEFAULT_GEMINI_MODEL, prompt, max_tokens)
    }

    /// GeminiConfigs for Gemini chrome dynamic scripting and caching.
    pub fn new_cache(
        model: &str,
        prompt: &str,
        max_tokens: u16,
        cache: Option<GeminiCache>,
    ) -> GeminiConfigs {
        Self {
            model: model.into(),
            prompt: Prompt::Single(prompt.into()),
            max_tokens,
            cache,
            ..Default::default()
        }
    }

    /// GeminiConfigs for Gemini chrome dynamic scripting multi chain prompts.
    pub fn new_multi<I, S>(model: &str, prompt: I, max_tokens: u16) -> GeminiConfigs
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

    /// GeminiConfigs for Gemini chrome dynamic scripting multi chain prompts with prompt caching. The feature flag 'cache_gemini' is required.
    pub fn new_multi_cache<I, S>(
        model: &str,
        prompt: I,
        max_tokens: u16,
        cache: Option<GeminiCache>,
    ) -> GeminiConfigs
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

    /// Set the top_k parameter (Gemini-specific).
    pub fn set_top_k(&mut self, top_k: Option<i32>) -> &mut Self {
        self.top_k = top_k;
        self
    }
}

#[test]
#[cfg(feature = "gemini")]
fn deserialize_gemini_configs() {
    let gemini_configs_json = r#"{"prompt":"change background blue","model":"gemini-flash-latest","max_tokens":256,"temperature":0.54,"top_p":0.17}"#;
    let configs = match serde_json::from_str::<GeminiConfigs>(&gemini_configs_json) {
        Ok(e) => Some(e),
        _ => None,
    };
    assert!(configs.is_some())
}
