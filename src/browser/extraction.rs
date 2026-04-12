//! Schema-driven structured data extraction from pages.
//!
//! Given a URL and a JSON schema, the flow is: navigate → snapshot → ask the
//! LLM to extract data conforming to the schema → validate. The actual
//! LLM round-trip requires the API client (in `run.rs`), so this module
//! owns the prompt-building and validation helpers; wiring lives elsewhere.

#![allow(dead_code)] // wired in a follow-up task; keep the shape stable

use anyhow::Result;
use serde_json::Value;

/// Extraction request.
#[derive(Debug)]
pub struct ExtractionRequest {
    pub url: String,
    pub schema: Value,
    pub instructions: Option<String>,
}

/// Build the extraction prompt for the LLM.
pub fn build_extraction_prompt(
    snapshot: &str,
    schema: &Value,
    instructions: Option<&str>,
) -> String {
    let mut prompt = format!(
        "Extract structured data from this page according to the JSON schema below.\n\n\
         ## Page Content (Accessibility Snapshot)\n\n{snapshot}\n\n\
         ## Required Output Schema\n\n```json\n{}\n```\n\n\
         Return ONLY valid JSON matching the schema. No markdown, no explanation.",
        serde_json::to_string_pretty(schema).unwrap_or_default()
    );

    if let Some(inst) = instructions {
        prompt.push_str(&format!("\n\n## Additional Instructions\n\n{inst}"));
    }

    prompt
}

/// Validate extracted JSON against the schema (basic type checking).
///
/// Only root-level checks: root `type`, and `required` field presence for
/// objects. No recursion into nested schemas, no format validation, no enum
/// checks. Good enough for the initial use case; swap in `jsonschema` crate
/// if we need full validation later.
pub fn validate_extraction(data: &Value, schema: &Value) -> Result<()> {
    let schema_type = schema["type"].as_str().unwrap_or("object");

    match schema_type {
        "object" => {
            if !data.is_object() {
                anyhow::bail!("Expected object, got {}", value_type_name(data));
            }
            // Check required fields
            if let Some(required) = schema["required"].as_array() {
                for req in required {
                    if let Some(field) = req.as_str()
                        && data.get(field).is_none() {
                            anyhow::bail!("Missing required field: {field}");
                        }
                }
            }
        }
        "array" => {
            if !data.is_array() {
                anyhow::bail!("Expected array, got {}", value_type_name(data));
            }
        }
        _ => {}
    }
    Ok(())
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
