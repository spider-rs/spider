//! Custom tool support for external API calls.

use dashmap::DashMap;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use crate::error::{AgentError, AgentResult};

/// HTTP method for API calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    /// GET request.
    Get,
    /// POST request.
    Post,
    /// PUT request.
    Put,
    /// PATCH request.
    Patch,
    /// DELETE request.
    Delete,
}

impl HttpMethod {
    fn as_reqwest_method(&self) -> reqwest::Method {
        match self {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Delete => reqwest::Method::DELETE,
        }
    }
}

/// Authentication configuration for custom tools.
#[derive(Debug, Clone)]
pub enum AuthConfig {
    /// No authentication.
    None,
    /// Bearer token authentication.
    Bearer(String),
    /// API key in header.
    ApiKey {
        /// Header name for the API key.
        header: String,
        /// API key value.
        key: String,
    },
    /// Basic authentication.
    Basic {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
    /// Custom header authentication.
    CustomHeader {
        /// Header name.
        name: String,
        /// Header value.
        value: String,
    },
}

/// Configuration for a custom tool (external API call).
#[derive(Debug, Clone)]
pub struct CustomTool {
    /// Unique name for this tool.
    pub name: String,
    /// Description of what this tool does.
    pub description: String,
    /// Base URL for the API.
    pub base_url: String,
    /// Default HTTP method.
    pub method: HttpMethod,
    /// Authentication configuration.
    pub auth: AuthConfig,
    /// Additional headers.
    pub headers: Vec<(String, String)>,
    /// Request timeout.
    pub timeout: Duration,
    /// Content type for requests.
    pub content_type: Option<String>,
}

impl CustomTool {
    /// Create a new custom tool with GET method.
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            base_url: base_url.into(),
            method: HttpMethod::Get,
            auth: AuthConfig::None,
            headers: Vec::new(),
            timeout: Duration::from_secs(30),
            content_type: None,
        }
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set HTTP method.
    pub fn with_method(mut self, method: HttpMethod) -> Self {
        self.method = method;
        self
    }

    /// Set bearer token authentication.
    pub fn with_bearer_auth(mut self, token: impl Into<String>) -> Self {
        self.auth = AuthConfig::Bearer(token.into());
        self
    }

    /// Set API key authentication.
    pub fn with_api_key(mut self, header: impl Into<String>, key: impl Into<String>) -> Self {
        self.auth = AuthConfig::ApiKey {
            header: header.into(),
            key: key.into(),
        };
        self
    }

    /// Set basic authentication.
    pub fn with_basic_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.auth = AuthConfig::Basic {
            username: username.into(),
            password: password.into(),
        };
        self
    }

    /// Set custom header authentication.
    pub fn with_custom_auth(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.auth = AuthConfig::CustomHeader {
            name: name.into(),
            value: value.into(),
        };
        self
    }

    /// Add a custom header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Set request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set content type.
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Build the headers for a request.
    fn build_headers(&self) -> AgentResult<HeaderMap> {
        let mut headers = HeaderMap::new();

        // Add authentication
        match &self.auth {
            AuthConfig::None => {}
            AuthConfig::Bearer(token) => {
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {}", token))
                        .map_err(|e| AgentError::Tool(format!("Invalid bearer token: {}", e)))?,
                );
            }
            AuthConfig::ApiKey { header, key } => {
                let header_name = HeaderName::try_from(header.as_str())
                    .map_err(|e| AgentError::Tool(format!("Invalid header name: {}", e)))?;
                let header_value = HeaderValue::from_str(key)
                    .map_err(|e| AgentError::Tool(format!("Invalid API key: {}", e)))?;
                headers.insert(header_name, header_value);
            }
            AuthConfig::Basic { username, password } => {
                let credentials = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    format!("{}:{}", username, password),
                );
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Basic {}", credentials))
                        .map_err(|e| AgentError::Tool(format!("Invalid basic auth: {}", e)))?,
                );
            }
            AuthConfig::CustomHeader { name, value } => {
                let header_name = HeaderName::try_from(name.as_str())
                    .map_err(|e| AgentError::Tool(format!("Invalid header name: {}", e)))?;
                let header_value = HeaderValue::from_str(value)
                    .map_err(|e| AgentError::Tool(format!("Invalid header value: {}", e)))?;
                headers.insert(header_name, header_value);
            }
        }

        // Add content type if specified
        if let Some(ref ct) = self.content_type {
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                HeaderValue::from_str(ct)
                    .map_err(|e| AgentError::Tool(format!("Invalid content type: {}", e)))?,
            );
        }

        // Add custom headers
        for (name, value) in &self.headers {
            let header_name = HeaderName::try_from(name.as_str())
                .map_err(|e| AgentError::Tool(format!("Invalid header name '{}': {}", name, e)))?;
            let header_value = HeaderValue::from_str(value).map_err(|e| {
                AgentError::Tool(format!("Invalid header value for '{}': {}", name, e))
            })?;
            headers.insert(header_name, header_value);
        }

        Ok(headers)
    }
}

/// Result from executing a custom tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolResult {
    /// The tool name that was executed.
    pub tool_name: String,
    /// HTTP status code.
    pub status: u16,
    /// Response body.
    pub body: String,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Whether the request was successful (2xx status).
    pub success: bool,
}

/// Registry for custom tools.
#[derive(Debug, Default)]
pub struct CustomToolRegistry {
    tools: DashMap<String, Arc<CustomTool>>,
}

impl CustomToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
        }
    }

    /// Register a custom tool.
    pub fn register(&self, tool: CustomTool) {
        self.tools.insert(tool.name.clone(), Arc::new(tool));
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<CustomTool>> {
        self.tools.get(name).map(|r| r.clone())
    }

    /// Remove a tool.
    pub fn remove(&self, name: &str) -> Option<Arc<CustomTool>> {
        self.tools.remove(name).map(|(_, v)| v)
    }

    /// List all registered tools.
    pub fn list(&self) -> Vec<String> {
        self.tools.iter().map(|e| e.key().clone()).collect()
    }

    /// Check if a tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Clear all tools.
    pub fn clear(&self) {
        self.tools.clear();
    }

    /// Execute a custom tool.
    pub async fn execute(
        &self,
        name: &str,
        client: &reqwest::Client,
        path: Option<&str>,
        query: Option<&[(&str, &str)]>,
        body: Option<&str>,
    ) -> AgentResult<CustomToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| AgentError::Tool(format!("Custom tool '{}' not found", name)))?;

        // Build URL
        let mut url = tool.base_url.clone();
        if let Some(p) = path {
            if !url.ends_with('/') && !p.starts_with('/') {
                url.push('/');
            }
            url.push_str(p);
        }

        // Build request
        let mut request = client
            .request(tool.method.as_reqwest_method(), &url)
            .timeout(tool.timeout)
            .headers(tool.build_headers()?);

        // Add query parameters
        if let Some(q) = query {
            request = request.query(q);
        }

        // Add body
        if let Some(b) = body {
            request = request.body(b.to_string());
        }

        // Execute
        let response = request.send().await?;

        let status = response.status().as_u16();
        let success = response.status().is_success();

        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let body = response.text().await?;

        Ok(CustomToolResult {
            tool_name: name.to_string(),
            status,
            body,
            headers,
            success,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_tool_builder() {
        let tool = CustomTool::new("my_api", "https://api.example.com")
            .with_description("My custom API")
            .with_method(HttpMethod::Post)
            .with_bearer_auth("secret_token")
            .with_header("X-Custom", "value")
            .with_timeout(Duration::from_secs(60))
            .with_content_type("application/json");

        assert_eq!(tool.name, "my_api");
        assert_eq!(tool.base_url, "https://api.example.com");
        assert_eq!(tool.description, "My custom API");
        assert_eq!(tool.method, HttpMethod::Post);
        assert_eq!(tool.timeout, Duration::from_secs(60));
        assert_eq!(tool.content_type, Some("application/json".to_string()));
        assert_eq!(tool.headers.len(), 1);
        assert!(matches!(tool.auth, AuthConfig::Bearer(_)));
    }

    #[test]
    fn test_custom_tool_registry() {
        let registry = CustomToolRegistry::new();

        // Register tools
        let tool1 = CustomTool::new("api_1", "https://api1.example.com");
        let tool2 = CustomTool::new("api_2", "https://api2.example.com");

        registry.register(tool1);
        registry.register(tool2);

        // Check registration
        assert!(registry.contains("api_1"));
        assert!(registry.contains("api_2"));
        assert!(!registry.contains("api_3"));

        // List tools
        let tools = registry.list();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"api_1".to_string()));
        assert!(tools.contains(&"api_2".to_string()));

        // Get tool
        let tool = registry.get("api_1");
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().base_url, "https://api1.example.com");

        // Remove tool
        let removed = registry.remove("api_1");
        assert!(removed.is_some());
        assert!(!registry.contains("api_1"));

        // Clear
        registry.clear();
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_auth_config_variants() {
        let tool =
            CustomTool::new("test", "https://example.com").with_api_key("X-API-Key", "my_key");
        assert!(matches!(tool.auth, AuthConfig::ApiKey { .. }));

        let tool = CustomTool::new("test", "https://example.com").with_basic_auth("user", "pass");
        assert!(matches!(tool.auth, AuthConfig::Basic { .. }));

        let tool = CustomTool::new("test", "https://example.com")
            .with_custom_auth("X-Custom-Auth", "token123");
        assert!(matches!(tool.auth, AuthConfig::CustomHeader { .. }));
    }

    #[test]
    fn test_http_method_conversion() {
        assert_eq!(HttpMethod::Get.as_reqwest_method(), reqwest::Method::GET);
        assert_eq!(HttpMethod::Post.as_reqwest_method(), reqwest::Method::POST);
        assert_eq!(HttpMethod::Put.as_reqwest_method(), reqwest::Method::PUT);
        assert_eq!(
            HttpMethod::Patch.as_reqwest_method(),
            reqwest::Method::PATCH
        );
        assert_eq!(
            HttpMethod::Delete.as_reqwest_method(),
            reqwest::Method::DELETE
        );
    }

    #[test]
    fn test_custom_tool_result() {
        let result = CustomToolResult {
            tool_name: "my_api".to_string(),
            status: 200,
            body: r#"{"success": true}"#.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            success: true,
        };

        assert_eq!(result.tool_name, "my_api");
        assert_eq!(result.status, 200);
        assert!(result.success);
    }
}
