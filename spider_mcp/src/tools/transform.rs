use rmcp::schemars;
use serde::Deserialize;
use serde_json::json;
use spider_transformations::transformation::content::{
    transform_content_input, ReturnFormat, TransformConfig, TransformInput,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TransformParams {
    /// Raw HTML content to transform
    pub html: String,
    /// Output format: markdown, text, or xml
    pub return_format: String,
    /// Base URL for resolving relative links
    pub url: Option<String>,
}

pub fn run(params: TransformParams) -> Result<String, String> {
    let parsed_url = params
        .url
        .as_ref()
        .and_then(|u| spider::url::Url::parse(u).ok());

    let input = TransformInput {
        url: parsed_url.as_ref(),
        content: params.html.as_bytes(),
        screenshot_bytes: None,
        encoding: None,
        selector_config: None,
        ignore_tags: None,
    };

    let conf = TransformConfig {
        return_format: ReturnFormat::from_str(&params.return_format),
        ..Default::default()
    };

    let content = transform_content_input(input, &conf);

    serde_json::to_string_pretty(&json!({
        "content": content,
        "format": params.return_format,
    }))
    .map_err(|e| e.to_string())
}
