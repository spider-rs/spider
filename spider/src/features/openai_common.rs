/// The type of prompt to use.
#[derive(Debug, Clone)]
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
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(prompts) = serde_json::from_str::<Vec<String>>(s) {
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

/// The GPT configs to use for dynamic Javascript execution and other functionality.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GPTConfigs {
    /// The prompt to use for OPENAI. Example: Search for movies. This will attempt to get the code required to perform the action on the page.
    pub prompt: Prompt,
    /// The model to use. Example: gpt-4-1106-preview or gpt-3.5-turbo-16k
    pub model: String,
    /// The max tokens to use for the request.
    pub max_tokens: u16,
    /// Prompts to use for certain urls. If this is set only the urls that match exactly are ran.
    pub prompt_url_map:
        Option<hashbrown::HashMap<case_insensitive_string::CaseInsensitiveString, Self>>,
    /// The temperature between 0 - 2.
    pub temperature: Option<f32>,
    /// The user for the request.
    pub user: Option<String>,
    /// The top priority for the request.
    pub top_p: Option<f32>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Extra data, this will merge the prompts and try to get the content for you. Example: extracting data from the page.
    pub extra_ai_data: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Map to paths. If the prompt_url_map has a key called /blog and all blog pages are found like /blog/something the same prompt is perform unless an exact match is found.
    pub paths_map: bool,
}

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
    let configs = match serde_json::from_str::<GPTConfigs>(&gpt_configs_json) {
        Ok(e) => Some(e),
        _ => None,
    };
    assert!(configs.is_some())
}
