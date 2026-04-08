/// Anthropic API client — port of services/api/claude.ts

pub mod ollama;
pub mod openai_compat;
pub mod types;

use anyhow::{anyhow, Context, Result};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::{header, Client};
use std::collections::HashMap;
use tracing::{debug, warn};

pub use ollama::{is_ollama_model, list_ollama_models, strip_ollama_prefix, OllamaClient};
pub use openai_compat::{
    is_openai_compat_model, parse_provider_model, OpenAiCompatClient, PROVIDERS,
};
pub use types::*;

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
const DEFAULT_MAX_TOKENS: u32 = 8096;

#[derive(Clone)]
#[allow(dead_code)] // api_key retained for future authenticated-header injection
pub struct ClaudeClient {
    client: Client,
    api_key: String,
    base_url: String,
}

impl ClaudeClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        let mut headers = header::HeaderMap::new();
        headers.insert("anthropic-version", ANTHROPIC_VERSION.parse()?);
        headers.insert("x-api-key", api_key.parse()?);
        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);

        let client = Client::builder()
            .default_headers(headers)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            api_key,
            base_url: ANTHROPIC_API_BASE.to_string(),
        })
    }

    /// Non-streaming API call — mirrors callModel() in services/api/claude.ts
    #[allow(dead_code)] // used by SDK/headless mode (non-streaming path)
    pub async fn messages(
        &self,
        request: MessagesRequest,
    ) -> Result<MessagesResponse> {
        let url = format!("{}/v1/messages", self.base_url);
        debug!("POST {url} model={}", request.model);

        let mut builder = self.client.post(&url).json(&request);
        if !request.betas.is_empty() {
            builder = builder.header("anthropic-beta", request.betas.join(","));
        }
        if let Some(ref sid) = request.session_id {
            builder = builder.header("X-Claude-Code-Session-Id", sid.as_str());
        }

        let resp = builder
            .send()
            .await
            .context("API request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("API error {status}: {body}"));
        }

        resp.json::<MessagesResponse>()
            .await
            .context("Failed to parse API response")
    }

    /// Streaming API call — collects all SSE events into a StreamedResponse.
    /// Mirrors the streaming path in services/api/claude.ts.
    /// The `on_text` callback is called with each text delta for live output.
    pub async fn messages_stream(
        &self,
        mut request: MessagesRequest,
        mut on_text: impl FnMut(&str),
    ) -> Result<StreamedResponse> {
        request.stream = Some(true);
        let url = format!("{}/v1/messages", self.base_url);
        debug!("POST {url} stream=true model={}", request.model);

        let mut builder = self.client.post(&url).json(&request);
        if !request.betas.is_empty() {
            builder = builder.header("anthropic-beta", request.betas.join(","));
        }
        if let Some(ref sid) = request.session_id {
            builder = builder.header("X-Claude-Code-Session-Id", sid.as_str());
        }

        let resp = builder
            .send()
            .await
            .context("Streaming API request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("API stream error {status}: {body}"));
        }

        let mut stream = resp.bytes_stream().eventsource();

        // Accumulator state
        let mut result = StreamedResponse::default();
        // Per-block accumulators: index → (type, text/json buffer)
        let mut text_blocks: HashMap<usize, String> = HashMap::with_capacity(4);
        let mut tool_blocks: HashMap<usize, (String, String, String)> = HashMap::with_capacity(4); // id, name, json
        let mut thinking_blocks: HashMap<usize, (String, String)> = HashMap::with_capacity(4); // thinking, sig

        while let Some(event) = stream.next().await {
            let event = event.context("SSE stream error")?;
            if event.data == "[DONE]" {
                break;
            }
            let parsed: StreamEvent = match serde_json::from_str(&event.data) {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to parse SSE event: {e}: {}", event.data);
                    continue;
                }
            };

            match parsed {
                StreamEvent::MessageStart { message } => {
                    result.usage.input_tokens = message.usage.input_tokens;
                }
                StreamEvent::ContentBlockStart { index, content_block } => {
                    match content_block {
                        StreamContentBlock::Text { text } => {
                            text_blocks.insert(index, text);
                        }
                        StreamContentBlock::ToolUse { id, name } => {
                            tool_blocks.insert(index, (id, name, String::new()));
                        }
                        StreamContentBlock::Thinking { thinking } => {
                            thinking_blocks.insert(index, (thinking, String::new()));
                        }
                    }
                }
                StreamEvent::ContentBlockDelta { index, delta } => match delta {
                    ContentDelta::TextDelta { text } => {
                        on_text(&text);
                        text_blocks.entry(index).or_default().push_str(&text);
                    }
                    ContentDelta::InputJsonDelta { partial_json } => {
                        if let Some((_, _, json)) = tool_blocks.get_mut(&index) {
                            json.push_str(&partial_json);
                        }
                    }
                    ContentDelta::ThinkingDelta { thinking } => {
                        if let Some((t, _)) = thinking_blocks.get_mut(&index) {
                            t.push_str(&thinking);
                        }
                    }
                    ContentDelta::SignatureDelta { signature } => {
                        if let Some((_, s)) = thinking_blocks.get_mut(&index) {
                            s.push_str(&signature);
                        }
                    }
                },
                StreamEvent::ContentBlockStop { index } => {
                    if let Some(text) = text_blocks.remove(&index) {
                        // Skip whitespace-only text blocks that appear alongside thinking blocks —
                        // sending them back to the API causes a 400 (v2.1.92 fix).
                        if !text.trim().is_empty() {
                            result.content.push(ContentBlock::Text { text });
                        }
                    } else if let Some((id, name, json)) = tool_blocks.remove(&index) {
                        let mut input: serde_json::Value = serde_json::from_str(&json)
                            .unwrap_or(serde_json::Value::Object(Default::default()));
                        // Normalize: the API sometimes emits array/object fields as
                        // JSON-encoded strings. Re-parse any string values that look
                        // like JSON arrays or objects (v2.1.89/92 fix).
                        normalize_tool_input(&mut input);
                        result.content.push(ContentBlock::ToolUse { id, name, input });
                    } else if let Some((thinking, signature)) = thinking_blocks.remove(&index) {
                        result.content.push(ContentBlock::Thinking { thinking, signature });
                    }
                }
                StreamEvent::MessageDelta { delta, usage } => {
                    result.stop_reason = delta.stop_reason;
                    if let Some(u) = usage {
                        result.usage.output_tokens = u.output_tokens;
                    }
                }
                StreamEvent::MessageStop | StreamEvent::Ping => {}
                StreamEvent::Error { error } => {
                    return Err(anyhow!("Stream error {}: {}", error.r#type, error.message));
                }
            }
        }

        Ok(result)
    }
}

/// Normalize streamed tool input: when the API emits array/object fields as
/// JSON-encoded strings (e.g. `"[\"a\",\"b\"]"` instead of `["a","b"]`),
/// re-parse them so downstream tools see the intended shape.
/// Fix landed in Claude Code v2.1.89/92.
fn normalize_tool_input(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                normalize_tool_input(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                normalize_tool_input(v);
            }
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim_start();
            if trimmed.starts_with('[') || trimmed.starts_with('{') {
                if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(s) {
                    if matches!(parsed, serde_json::Value::Array(_) | serde_json::Value::Object(_)) {
                        normalize_tool_input(&mut parsed);
                        *val = parsed;
                    }
                }
            }
        }
        _ => {}
    }
}

pub fn default_model() -> &'static str {
    DEFAULT_MODEL
}

pub fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

// ─── Unified backend ──────────────────────────────────────────────────────────

/// Routes API calls to the Anthropic, Ollama, or OpenAI-compatible backend
/// based on the model prefix.  All code above the query engine works with
/// `ApiBackend` instead of `ClaudeClient` directly.
#[derive(Clone)]
pub enum ApiBackend {
    Anthropic(ClaudeClient),
    Ollama(OllamaClient),
    OpenAiCompat(OpenAiCompatClient),
}

impl ApiBackend {
    /// Create the right backend for `model`.
    /// `api_key` is required for Anthropic; ignored for Ollama/OpenAI-compat.
    pub fn new(model: &str, api_key: &str, ollama_host: &str) -> Result<Self> {
        if is_ollama_model(model) {
            Ok(Self::Ollama(OllamaClient::new(ollama_host)?))
        } else if is_openai_compat_model(model) {
            Ok(Self::OpenAiCompat(OpenAiCompatClient::from_model(model)?))
        } else {
            Ok(Self::Anthropic(ClaudeClient::new(api_key)?))
        }
    }

    /// Streaming call — identical interface to `ClaudeClient::messages_stream`.
    pub async fn messages_stream(
        &self,
        request: MessagesRequest,
        on_text: impl FnMut(&str),
    ) -> Result<StreamedResponse> {
        match self {
            Self::Anthropic(c) => c.messages_stream(request, on_text).await,
            Self::Ollama(c) => c.messages_stream(request, on_text).await,
            Self::OpenAiCompat(c) => c.messages_stream(request, on_text).await,
        }
    }

    /// Non-streaming call (falls through to streaming + collect for non-Anthropic backends).
    #[allow(dead_code)] // SDK/headless non-streaming path
    pub async fn messages(&self, request: MessagesRequest) -> Result<MessagesResponse> {
        match self {
            Self::Anthropic(c) => c.messages(request).await,
            Self::Ollama(c) => {
                let streamed = c.messages_stream(request, |_| {}).await?;
                Ok(MessagesResponse {
                    id: uuid::Uuid::new_v4().to_string(),
                    content: streamed.content,
                    stop_reason: streamed.stop_reason,
                    usage: streamed.usage,
                })
            }
            Self::OpenAiCompat(c) => {
                let streamed = c.messages_stream(request, |_| {}).await?;
                Ok(MessagesResponse {
                    id: uuid::Uuid::new_v4().to_string(),
                    content: streamed.content,
                    stop_reason: streamed.stop_reason,
                    usage: streamed.usage,
                })
            }
        }
    }

    /// The Ollama host URL if this is an Ollama backend.
    #[allow(dead_code)]
    pub fn ollama_host(&self) -> Option<&str> {
        match self {
            Self::Ollama(c) => Some(&c.base_url),
            _ => None,
        }
    }

    /// Provider display name (for status bar / logging).
    #[allow(dead_code)]
    pub fn provider_name(&self) -> &str {
        match self {
            Self::Anthropic(_) => "Anthropic",
            Self::Ollama(_) => "Ollama",
            Self::OpenAiCompat(c) => &c.provider_name,
        }
    }

    /// True if the model has been detected as not supporting tools.
    #[allow(dead_code)]
    pub fn tools_disabled(&self) -> bool {
        match self {
            Self::Ollama(c) => c.tools_disabled(),
            Self::OpenAiCompat(c) => c.tools_disabled(),
            Self::Anthropic(_) => false,
        }
    }

    /// Returns true the first time called after tools are disabled — for a one-time user notice.
    pub fn take_tools_notice(&self) -> bool {
        match self {
            Self::Ollama(c) => c.take_tools_notice(),
            Self::OpenAiCompat(c) => c.take_tools_notice(),
            Self::Anthropic(_) => false,
        }
    }
}
