//! Extraction schema generation.
//!
//! This module provides utilities for auto-generating JSON schemas from
//! example outputs, enabling zero-config extraction.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Request to generate a schema from examples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaGenerationRequest {
    /// Example values to infer schema from.
    pub examples: Vec<Value>,
    /// Optional description of what the schema represents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether to generate strict schema (no additional properties).
    pub strict: bool,
    /// Optional name for the schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl SchemaGenerationRequest {
    /// Create a new request with one example.
    pub fn from_example(example: Value) -> Self {
        Self {
            examples: vec![example],
            description: None,
            strict: false,
            name: None,
        }
    }

    /// Create from multiple examples.
    pub fn from_examples(examples: Vec<Value>) -> Self {
        Self {
            examples,
            description: None,
            strict: false,
            name: None,
        }
    }

    /// Add a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set strict mode.
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    /// Set name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add another example.
    pub fn add_example(mut self, example: Value) -> Self {
        self.examples.push(example);
        self
    }
}

/// A generated JSON schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedSchema {
    /// The JSON Schema.
    pub schema: Value,
    /// Name for the schema.
    pub name: String,
    /// Descriptions for each field (field_path -> description).
    pub field_descriptions: HashMap<String, String>,
    /// Confidence in the generated schema (0.0 to 1.0).
    pub confidence: f64,
    /// Whether the schema was generated in strict mode.
    pub strict: bool,
    /// Number of examples used to generate.
    pub examples_used: usize,
}

impl GeneratedSchema {
    /// Create a new generated schema.
    pub fn new(schema: Value, name: impl Into<String>) -> Self {
        Self {
            schema,
            name: name.into(),
            field_descriptions: HashMap::new(),
            confidence: 0.5,
            strict: false,
            examples_used: 0,
        }
    }

    /// Set field descriptions.
    pub fn with_field_descriptions(mut self, descriptions: HashMap<String, String>) -> Self {
        self.field_descriptions = descriptions;
        self
    }

    /// Set confidence.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Mark as strict.
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    /// Set examples used.
    pub fn with_examples_used(mut self, count: usize) -> Self {
        self.examples_used = count;
        self
    }

    /// Convert to ExtractionSchema for use in automation.
    pub fn to_extraction_schema(&self) -> super::ExtractionSchema {
        super::ExtractionSchema {
            name: self.name.clone(),
            description: self.field_descriptions.get("").cloned(),
            schema: self.schema.to_string(),
            strict: self.strict,
        }
    }

    /// Get the schema as a pretty-printed string.
    pub fn schema_string(&self) -> String {
        serde_json::to_string_pretty(&self.schema).unwrap_or_default()
    }
}

/// Cache for generated schemas.
#[derive(Debug, Clone, Default)]
pub struct SchemaCache {
    /// Cached schemas by name.
    schemas: HashMap<String, GeneratedSchema>,
    /// Usage counts for analytics.
    usage_counts: HashMap<String, usize>,
}

impl SchemaCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cached schema by name.
    pub fn get(&self, name: &str) -> Option<&GeneratedSchema> {
        self.schemas.get(name)
    }

    /// Store a schema.
    pub fn store(&mut self, schema: GeneratedSchema) {
        let name = schema.name.clone();
        self.schemas.insert(name.clone(), schema);
        self.usage_counts.entry(name).or_insert(0);
    }

    /// Record usage of a schema.
    pub fn record_usage(&mut self, name: &str) {
        if let Some(count) = self.usage_counts.get_mut(name) {
            *count += 1;
        }
    }

    /// Get usage count for a schema.
    pub fn usage_count(&self, name: &str) -> usize {
        self.usage_counts.get(name).copied().unwrap_or(0)
    }

    /// Get all schema names.
    pub fn schema_names(&self) -> Vec<&str> {
        self.schemas.keys().map(|s| s.as_str()).collect()
    }

    /// Remove a schema.
    pub fn remove(&mut self, name: &str) -> Option<GeneratedSchema> {
        self.usage_counts.remove(name);
        self.schemas.remove(name)
    }

    /// Clear the cache.
    pub fn clear(&mut self) {
        self.schemas.clear();
        self.usage_counts.clear();
    }

    /// Get cache size.
    pub fn len(&self) -> usize {
        self.schemas.len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
    }
}

/// Infer a JSON schema from a value.
pub fn infer_schema(value: &Value) -> Value {
    match value {
        Value::Null => json!({ "type": "null" }),
        Value::Bool(_) => json!({ "type": "boolean" }),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                json!({ "type": "integer" })
            } else {
                json!({ "type": "number" })
            }
        }
        Value::String(_) => json!({ "type": "string" }),
        Value::Array(arr) => {
            if arr.is_empty() {
                json!({
                    "type": "array",
                    "items": {}
                })
            } else {
                // Infer item schema from first element (could merge all for better accuracy)
                let item_schema = infer_schema(&arr[0]);
                json!({
                    "type": "array",
                    "items": item_schema
                })
            }
        }
        Value::Object(obj) => {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();

            for (key, val) in obj {
                properties.insert(key.clone(), infer_schema(val));
                // Assume all fields are required initially
                required.push(Value::String(key.clone()));
            }

            json!({
                "type": "object",
                "properties": Value::Object(properties),
                "required": required
            })
        }
    }
}

/// Infer a schema from multiple examples, merging field information.
pub fn infer_schema_from_examples(examples: &[Value]) -> Value {
    if examples.is_empty() {
        return json!({});
    }

    if examples.len() == 1 {
        return infer_schema(&examples[0]);
    }

    // For multiple examples, we need to merge schemas
    let schemas: Vec<Value> = examples.iter().map(infer_schema).collect();

    // Simple merge: use first schema's structure, but mark fields as optional
    // if they don't appear in all examples
    merge_schemas(&schemas)
}

/// Merge multiple schemas, making fields optional if not present in all.
fn merge_schemas(schemas: &[Value]) -> Value {
    if schemas.is_empty() {
        return json!({});
    }

    if schemas.len() == 1 {
        return schemas[0].clone();
    }

    // If all schemas are same type, merge
    let first_type = schemas[0].get("type").and_then(|t| t.as_str());

    if schemas.iter().all(|s| s.get("type").and_then(|t| t.as_str()) == first_type) {
        match first_type {
            Some("object") => merge_object_schemas(schemas),
            Some("array") => merge_array_schemas(schemas),
            _ => schemas[0].clone(),
        }
    } else {
        // Different types - use oneOf
        json!({
            "oneOf": schemas.to_vec()
        })
    }
}

/// Merge object schemas.
fn merge_object_schemas(schemas: &[Value]) -> Value {
    let mut all_properties: HashMap<String, Vec<&Value>> = HashMap::new();
    let mut field_counts: HashMap<String, usize> = HashMap::new();

    // Collect all properties from all schemas
    for schema in schemas {
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            for (key, val) in props {
                all_properties.entry(key.clone()).or_default().push(val);
                *field_counts.entry(key.clone()).or_insert(0) += 1;
            }
        }
    }

    // Merge each property
    let mut merged_props = serde_json::Map::new();
    let mut required = Vec::new();

    for (key, values) in &all_properties {
        // If field appears in all schemas, mark as required
        let count = field_counts.get(key).copied().unwrap_or(0);
        if count == schemas.len() {
            required.push(Value::String(key.clone()));
        }

        // Merge the property schemas
        let prop_schemas: Vec<Value> = values.iter().map(|v| (*v).clone()).collect();
        let merged = merge_schemas(&prop_schemas);
        merged_props.insert(key.clone(), merged);
    }

    json!({
        "type": "object",
        "properties": Value::Object(merged_props),
        "required": required
    })
}

/// Merge array schemas.
fn merge_array_schemas(schemas: &[Value]) -> Value {
    // Collect all item schemas
    let item_schemas: Vec<Value> = schemas
        .iter()
        .filter_map(|s| s.get("items").cloned())
        .collect();

    let merged_items = if item_schemas.is_empty() {
        json!({})
    } else {
        merge_schemas(&item_schemas)
    };

    json!({
        "type": "array",
        "items": merged_items
    })
}

/// Generate a schema from a request.
pub fn generate_schema(request: &SchemaGenerationRequest) -> GeneratedSchema {
    let schema = infer_schema_from_examples(&request.examples);

    // Calculate confidence based on number of examples
    let confidence = match request.examples.len() {
        0 => 0.0,
        1 => 0.5,
        2..=5 => 0.7,
        _ => 0.9,
    };

    let name = request
        .name
        .clone()
        .unwrap_or_else(|| "generated_schema".to_string());

    let mut generated = GeneratedSchema::new(schema, name)
        .with_confidence(confidence)
        .with_examples_used(request.examples.len());

    if request.strict {
        generated = generated.strict();
    }

    if let Some(desc) = &request.description {
        let mut descriptions = HashMap::new();
        descriptions.insert(String::new(), desc.clone());
        generated = generated.with_field_descriptions(descriptions);
    }

    generated
}

/// Refine a schema by adding more examples.
pub fn refine_schema(current: &GeneratedSchema, new_examples: &[Value]) -> GeneratedSchema {
    // Infer schema from new examples
    let new_schema = infer_schema_from_examples(new_examples);

    // Merge with current schema
    let merged = merge_schemas(&[current.schema.clone(), new_schema]);

    let total_examples = current.examples_used + new_examples.len();
    let confidence = match total_examples {
        0..=2 => 0.5,
        3..=5 => 0.7,
        6..=10 => 0.85,
        _ => 0.95,
    };

    GeneratedSchema::new(merged, &current.name)
        .with_field_descriptions(current.field_descriptions.clone())
        .with_confidence(confidence)
        .with_examples_used(total_examples)
}

/// Build a prompt for LLM-assisted schema generation.
pub fn build_schema_generation_prompt(request: &SchemaGenerationRequest) -> String {
    let mut prompt = String::with_capacity(2048);

    prompt.push_str("SCHEMA GENERATION REQUEST\n\n");

    if let Some(desc) = &request.description {
        prompt.push_str("Description: ");
        prompt.push_str(desc);
        prompt.push_str("\n\n");
    }

    prompt.push_str("Examples:\n");
    for (i, example) in request.examples.iter().enumerate() {
        prompt.push_str(&format!("Example {}:\n", i + 1));
        prompt.push_str(&serde_json::to_string_pretty(example).unwrap_or_default());
        prompt.push_str("\n\n");
    }

    prompt.push_str("TASK:\n");
    prompt.push_str("Generate a JSON Schema that describes the structure of the examples above.\n");
    prompt.push_str("Return a JSON object with:\n");
    prompt.push_str("- schema: the JSON Schema\n");
    prompt.push_str("- field_descriptions: object mapping field paths to descriptions\n");
    prompt.push_str("- confidence: confidence in the schema (0.0-1.0)\n");

    if request.strict {
        prompt.push_str("\nIMPORTANT: Generate a strict schema (additionalProperties: false)\n");
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_simple_types() {
        assert_eq!(infer_schema(&json!(null)), json!({"type": "null"}));
        assert_eq!(infer_schema(&json!(true)), json!({"type": "boolean"}));
        assert_eq!(infer_schema(&json!(42)), json!({"type": "integer"}));
        assert_eq!(infer_schema(&json!(3.14)), json!({"type": "number"}));
        assert_eq!(infer_schema(&json!("hello")), json!({"type": "string"}));
    }

    #[test]
    fn test_infer_array() {
        let arr = json!([1, 2, 3]);
        let schema = infer_schema(&arr);
        assert_eq!(schema["type"], "array");
        assert_eq!(schema["items"]["type"], "integer");
    }

    #[test]
    fn test_infer_object() {
        let obj = json!({
            "name": "John",
            "age": 30,
            "active": true
        });

        let schema = infer_schema(&obj);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["name"]["type"], "string");
        assert_eq!(schema["properties"]["age"]["type"], "integer");
        assert_eq!(schema["properties"]["active"]["type"], "boolean");
    }

    #[test]
    fn test_infer_nested() {
        let obj = json!({
            "user": {
                "name": "John",
                "tags": ["a", "b"]
            }
        });

        let schema = infer_schema(&obj);
        assert_eq!(schema["properties"]["user"]["type"], "object");
        assert_eq!(schema["properties"]["user"]["properties"]["tags"]["type"], "array");
    }

    #[test]
    fn test_infer_from_multiple_examples() {
        let examples = vec![
            json!({"name": "John", "age": 30}),
            json!({"name": "Jane", "age": 25, "city": "NYC"}),
        ];

        let schema = infer_schema_from_examples(&examples);
        assert_eq!(schema["type"], "object");

        // name and age should be required (in both)
        let required: Vec<_> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(required.contains(&"name"));
        assert!(required.contains(&"age"));
    }

    #[test]
    fn test_generate_schema() {
        let request = SchemaGenerationRequest::from_example(json!({
            "product": "Widget",
            "price": 9.99
        }))
        .with_name("product_schema")
        .with_description("Product data");

        let generated = generate_schema(&request);
        assert_eq!(generated.name, "product_schema");
        assert_eq!(generated.examples_used, 1);
        assert!(generated.confidence > 0.0);
    }

    #[test]
    fn test_schema_cache() {
        let mut cache = SchemaCache::new();

        let schema = GeneratedSchema::new(json!({"type": "object"}), "test");
        cache.store(schema);

        assert!(cache.get("test").is_some());
        assert_eq!(cache.len(), 1);

        cache.record_usage("test");
        assert_eq!(cache.usage_count("test"), 1);

        cache.remove("test");
        assert!(cache.get("test").is_none());
    }

    #[test]
    fn test_refine_schema() {
        let initial = GeneratedSchema::new(
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                }
            }),
            "test",
        )
        .with_examples_used(1);

        // Add multiple examples to exceed the threshold for higher confidence
        let new_examples = vec![
            json!({"name": "John", "email": "john@test.com"}),
            json!({"name": "Jane", "email": "jane@test.com"}),
            json!({"name": "Bob", "email": "bob@test.com"}),
        ];

        let refined = refine_schema(&initial, &new_examples);
        assert_eq!(refined.examples_used, 4);
        // 4 examples = 3-5 range = 0.7 confidence, initial (1 example) = 0.5
        assert!(refined.confidence >= initial.confidence);
    }

    #[test]
    fn test_to_extraction_schema() {
        let generated = GeneratedSchema::new(
            json!({"type": "object"}),
            "my_schema",
        )
        .strict();

        let extraction = generated.to_extraction_schema();
        assert_eq!(extraction.name, "my_schema");
        assert!(extraction.strict);
    }

    #[test]
    fn test_schema_generation_request_builder() {
        let request = SchemaGenerationRequest::from_example(json!({"a": 1}))
            .add_example(json!({"a": 2}))
            .with_name("test")
            .with_description("Test schema")
            .strict();

        assert_eq!(request.examples.len(), 2);
        assert_eq!(request.name, Some("test".to_string()));
        assert!(request.strict);
    }

    #[test]
    fn test_build_schema_generation_prompt() {
        let request = SchemaGenerationRequest::from_example(json!({"x": 1}))
            .with_description("A simple schema");

        let prompt = build_schema_generation_prompt(&request);
        assert!(prompt.contains("SCHEMA GENERATION REQUEST"));
        assert!(prompt.contains("A simple schema"));
        assert!(prompt.contains("Example 1"));
    }
}
