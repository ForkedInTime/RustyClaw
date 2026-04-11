/// WebSearchTool — port of tools/WebSearchTool/WebSearchTool.ts
/// Uses the Anthropic web search beta API (web-search-2025-03-05).
use super::{Tool, ToolContext, ToolOutput, async_trait};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

pub struct WebSearchTool {
    pub api_key: String,
    pub model: String,
}

#[derive(Deserialize)]
struct WebSearchInput {
    query: String,
    #[serde(default)]
    allowed_domains: Option<Vec<String>>,
    #[serde(default)]
    blocked_domains: Option<Vec<String>>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Returns summarized results \
        with sources. Use for questions about recent events, documentation, \
        or anything requiring up-to-date information."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "allowed_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only include results from these domains"
                },
                "blocked_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Exclude results from these domains"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let input: WebSearchInput = serde_json::from_value(input)?;

        // Build the web_search tool definition for the Anthropic beta API
        let mut web_search_tool = json!({
            "type": "web_search_20250305",
            "name": "web_search"
        });

        if let Some(allowed) = &input.allowed_domains {
            web_search_tool["allowed_domains"] = json!(allowed);
        }
        if let Some(blocked) = &input.blocked_domains {
            web_search_tool["blocked_domains"] = json!(blocked);
        }

        let request_body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [{
                "role": "user",
                "content": format!("Search for: {}", input.query)
            }],
            "tools": [web_search_tool]
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "web-search-2025-03-05")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Ok(ToolOutput::error(format!(
                "Web search API error {status}: {body}"
            )));
        }

        let resp: serde_json::Value = response.json().await?;

        // Extract text content from the response
        let mut result = String::new();
        if let Some(content) = resp["content"].as_array() {
            for block in content {
                match block["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            result.push_str(text);
                        }
                    }
                    Some("web_search_result") => {
                        // Include source URLs
                        if let Some(results) = block["results"].as_array() {
                            for r in results {
                                if let (Some(title), Some(url)) =
                                    (r["title"].as_str(), r["url"].as_str())
                                {
                                    result.push_str(&format!("\n[{title}]({url})"));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if result.trim().is_empty() {
            return Ok(ToolOutput::error("No search results returned"));
        }

        Ok(ToolOutput::success(result))
    }
}
