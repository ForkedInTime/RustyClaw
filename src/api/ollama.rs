/// Ollama backend — translates between Anthropic's /v1/messages format
/// and Ollama's OpenAI-compatible /v1/chat/completions format.
///
/// Usage: any model prefixed with "ollama:" is routed here instead of
/// the Anthropic API.  The translation is transparent to the rest of the
/// codebase — callers receive the same `StreamedResponse` they would get
/// from `ClaudeClient`.
///
///   /model ollama:dolphin3
///   /model ollama:qwen3:14b
///   /model ollama:llama4

use anyhow::{anyhow, Context, Result};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tracing::{debug, warn};

use crate::api::types::*;

// ─── Prefix helpers ───────────────────────────────────────────────────────────

pub const OLLAMA_PREFIX: &str = "ollama:";

pub fn is_ollama_model(model: &str) -> bool {
    model.starts_with(OLLAMA_PREFIX)
}

/// Strip "ollama:" prefix → bare model name for the Ollama API.
pub fn strip_ollama_prefix(model: &str) -> &str {
    model.strip_prefix(OLLAMA_PREFIX).unwrap_or(model)
}

// ─── Model discovery ──────────────────────────────────────────────────────────

/// Ask the local Ollama instance for its installed model list.
/// Returns `ollama:<name>` prefixed strings ready for use as config.model.
/// Returns an empty Vec if Ollama is not running or unreachable.
pub async fn list_ollama_models(base_url: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct OllamaModel { name: String }
    #[derive(Deserialize)]
    struct TagsResponse { models: Vec<OllamaModel> }

    let url = format!("{base_url}/api/tags");
    let client = Client::new();
    match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Err(_) => vec![],
        Ok(resp) if !resp.status().is_success() => vec![],
        Ok(resp) => match resp.json::<TagsResponse>().await {
            Err(_) => vec![],
            Ok(data) => data
                .models
                .into_iter()
                .map(|m| format!("{OLLAMA_PREFIX}{}", m.name))
                .collect(),
        },
    }
}

/// Check whether `model` (bare, without prefix) exists in Ollama.
pub async fn validate_ollama_model(base_url: &str, model: &str) -> Result<()> {
    let known = list_ollama_models(base_url).await;
    let full = format!("{OLLAMA_PREFIX}{model}");
    if known.is_empty() {
        // Ollama not running — surface a clear error
        return Err(anyhow!(
            "Cannot reach Ollama at {base_url}. Is it running?\n  Start with: ollama serve"
        ));
    }
    if !known.contains(&full) {
        return Err(anyhow!(
            "Model '{model}' not found in Ollama.\n  Install with: ollama pull {model}\n  Available: {}",
            known
                .iter()
                .map(|m| strip_ollama_prefix(m))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

// ─── OpenAI / Ollama wire types ───────────────────────────────────────────────

#[derive(Serialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OaiFunction,
}

#[derive(Serialize, Deserialize, Clone)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OaiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OaiFunctionDef,
}

#[derive(Serialize)]
struct OaiFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OaiTool>,
    stream: bool,
    stream_options: Option<StreamOptions>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

// ── Streaming response chunks ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct OaiChunk {
    choices: Vec<OaiChoice>,
    #[serde(default)]
    usage: Option<OaiUsage>,
}

#[derive(Deserialize)]
struct OaiChoice {
    delta: OaiDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct OaiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OaiToolCallDelta>>,
}

#[derive(Deserialize)]
struct OaiToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OaiFunctionDelta>,
}

#[derive(Deserialize)]
struct OaiFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct OaiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

// ─── Anthropic → OpenAI translation ──────────────────────────────────────────

/// Translate a list of Anthropic `Message`s into OpenAI-format messages.
/// System prompt is handled separately (passed as role:system first message).
fn translate_messages(system: &str, messages: &[Message]) -> Vec<OaiMessage> {
    let mut out = Vec::with_capacity(messages.len() + 1);

    if !system.is_empty() {
        out.push(OaiMessage {
            role: "system".into(),
            content: Some(serde_json::Value::String(system.to_string())),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    for msg in messages {
        match msg.role {
            Role::User => {
                // A user message may be: plain text OR a batch of tool results
                // (or both in edge cases — handle by splitting).
                let text_blocks: Vec<&ContentBlock> = msg
                    .content
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::Text { .. }))
                    .collect();
                let result_blocks: Vec<&ContentBlock> = msg
                    .content
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .collect();

                // Text user message
                if !text_blocks.is_empty() {
                    let text = text_blocks
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    out.push(OaiMessage {
                        role: "user".into(),
                        content: Some(serde_json::Value::String(text)),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }

                // Tool result messages (one OAI message per result)
                for block in result_blocks {
                    if let ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                        let text = content
                            .iter()
                            .map(|c| {
                                let ToolResultContent::Text { text } = c;
                                text.as_str()
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        out.push(OaiMessage {
                            role: "tool".into(),
                            content: Some(serde_json::Value::String(text)),
                            tool_calls: None,
                            tool_call_id: Some(tool_use_id.clone()),
                        });
                    }
                }
            }

            Role::Assistant => {
                // Collect text and tool_use blocks
                let mut text_parts: Vec<&str> = Vec::new();
                let mut tool_calls: Vec<OaiToolCall> = Vec::new();

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.as_str()),
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(OaiToolCall {
                                id: id.clone(),
                                call_type: "function".into(),
                                function: OaiFunction {
                                    name: name.clone(),
                                    arguments: serde_json::to_string(input)
                                        .unwrap_or_else(|_| "{}".into()),
                                },
                            });
                        }
                        // Thinking/Image blocks are Anthropic-specific — skip for Ollama
                        ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } | ContentBlock::Image { .. } => {}
                    }
                }

                let content = if text_parts.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::String(text_parts.join("\n")))
                };

                out.push(OaiMessage {
                    role: "assistant".into(),
                    content,
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                    tool_call_id: None,
                });
            }
        }
    }

    out
}

/// Translate Anthropic `ToolDefinition`s to OpenAI tool format.
fn translate_tools(tools: &[ToolDefinition]) -> Vec<OaiTool> {
    tools
        .iter()
        .map(|t| OaiTool {
            tool_type: "function".into(),
            function: OaiFunctionDef {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect()
}

// ─── Ollama client ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct OllamaClient {
    client: Client,
    pub base_url: String,
    /// Set to true after the first 400 "does not support tools" — skips tools on all future calls.
    no_tools: Arc<AtomicBool>,
    /// Set to true after the user has already been notified about text-only mode.
    tools_notice_sent: Arc<AtomicBool>,
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let client = Client::builder()
            .build()
            .context("Failed to build Ollama HTTP client")?;
        Ok(Self {
            client,
            base_url: base_url.into(),
            no_tools: Arc::new(AtomicBool::new(false)),
            tools_notice_sent: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Returns true if this model has been detected as not supporting tools.
    pub fn tools_disabled(&self) -> bool {
        self.no_tools.load(Ordering::Relaxed)
    }

    /// Returns true the first time called after tools are disabled — used to send a one-time notice.
    pub fn take_tools_notice(&self) -> bool {
        self.no_tools.load(Ordering::Relaxed)
            && !self.tools_notice_sent.swap(true, Ordering::Relaxed)
    }

    /// Streaming call to Ollama — translates request and response.
    /// Drop-in replacement for `ClaudeClient::messages_stream`.
    pub async fn messages_stream(
        &self,
        request: MessagesRequest,
        mut on_text: impl FnMut(&str),
    ) -> Result<StreamedResponse> {
        let model = strip_ollama_prefix(&request.model).to_string();
        let url = format!("{}/v1/chat/completions", self.base_url);
        debug!("POST {url} model={model} (via Ollama)");

        let no_tools = self.no_tools.load(Ordering::Relaxed);

        // Extract the plain system string from SystemContent
        let system_str: String = match &request.system {
            crate::api::types::SystemContent::Plain(s) => s.clone(),
            crate::api::types::SystemContent::Blocks(blocks) => {
                blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n")
            }
        };

        // When running in text-only mode, patch the system prompt so the model
        // isn't told it has tools when none are sent — avoids confused/stuck responses.
        let system = if no_tools {
            let patched = system_str
                .lines()
                .filter(|l| {
                    !l.contains("You have access to tools") &&
                    !l.contains("Use tools to actually")
                })
                .collect::<Vec<_>>()
                .join("\n");
            // Append a clear note that this is text-only mode
            format!("{patched}\n- Text-only mode: answer from knowledge, no file/command access.")
        } else {
            system_str.clone()
        };

        let oai_messages = translate_messages(&system, &request.messages);
        // Skip tools entirely if we already know this model doesn't support them
        let oai_tools = if no_tools {
            vec![]
        } else {
            translate_tools(&request.tools)
        };

        let mut oai_request = OaiRequest {
            model,
            messages: oai_messages,
            tools: oai_tools,
            stream: true,
            stream_options: Some(StreamOptions { include_usage: true }),
        };

        let resp = self
            .client
            .post(&url)
            .json(&oai_request)
            .send()
            .await
            .context("Ollama request failed — is Ollama running?")?;

        let status = resp.status();
        let resp = if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            // If the model doesn't support tools, cache that and retry without them
            if status.as_u16() == 400 && body.contains("does not support tools") {
                self.no_tools.store(true, Ordering::Relaxed);
                debug!("Model does not support tools — disabling tools for this session");
                // Rebuild messages with patched system (no tool references) and no tools
                let patched_system = system_str
                    .lines()
                    .filter(|l| {
                        !l.contains("You have access to tools") &&
                        !l.contains("Use tools to actually")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let patched_system = format!(
                    "{patched_system}\n- Text-only mode: answer from knowledge, no file/command access."
                );
                oai_request.messages = translate_messages(&patched_system, &request.messages);
                oai_request.tools = vec![];
                self.client
                    .post(&url)
                    .json(&oai_request)
                    .send()
                    .await
                    .context("Ollama request failed — is Ollama running?")?
            } else {
                return Err(anyhow!("Ollama error {status}: {body}"));
            }
        } else {
            resp
        };

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Ollama error {status}: {body}"));
        }

        // ── Parse SSE stream ─────────────────────────────────────────────────

        let mut stream = resp.bytes_stream().eventsource();

        let mut result = StreamedResponse::default();

        // Text accumulator
        let mut text_buf = String::new();
        // Tool call accumulators: index → (id, name, arguments_so_far)
        let mut tool_bufs: HashMap<usize, (String, String, String)> = HashMap::new();
        let mut finish_reason: Option<String> = None;

        while let Some(event) = stream.next().await {
            let event = event.context("Ollama SSE stream error")?;
            if event.data == "[DONE]" {
                break;
            }

            let chunk: OaiChunk = match serde_json::from_str(&event.data) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to parse Ollama SSE chunk: {e}: {}", event.data);
                    continue;
                }
            };

            // Capture usage if provided (Ollama sends it in the final chunk)
            if let Some(usage) = chunk.usage {
                result.usage.input_tokens = usage.prompt_tokens;
                result.usage.output_tokens = usage.completion_tokens;
            }

            for choice in chunk.choices {
                // Capture finish reason
                if let Some(fr) = choice.finish_reason {
                    finish_reason = Some(fr);
                }

                let delta = choice.delta;

                // Text delta
                if let Some(text) = delta.content {
                    if !text.is_empty() {
                        on_text(&text);
                        text_buf.push_str(&text);
                    }
                }

                // Tool call deltas
                if let Some(tc_deltas) = delta.tool_calls {
                    for tc in tc_deltas {
                        let entry = tool_bufs
                            .entry(tc.index)
                            .or_insert_with(|| (String::new(), String::new(), String::new()));

                        if let Some(id) = tc.id {
                            entry.0 = id;
                        }
                        if let Some(func) = tc.function {
                            if let Some(name) = func.name {
                                entry.1 = name;
                            }
                            if let Some(args) = func.arguments {
                                entry.2.push_str(&args);
                            }
                        }
                    }
                }
            }
        }

        // ── Assemble final ContentBlocks ─────────────────────────────────────

        if !text_buf.is_empty() {
            result.content.push(ContentBlock::Text { text: text_buf });
        }

        // Sort tool calls by index for deterministic ordering
        let mut tool_entries: Vec<(usize, (String, String, String))> =
            tool_bufs.into_iter().collect();
        tool_entries.sort_by_key(|(idx, _)| *idx);

        for (_, (id, name, args)) in tool_entries {
            let input = serde_json::from_str(&args)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            result.content.push(ContentBlock::ToolUse { id, name, input });
        }

        // ── Map finish_reason → StopReason ───────────────────────────────────

        result.stop_reason = match finish_reason.as_deref() {
            Some("stop") | Some("eos") => Some(StopReason::EndTurn),
            Some("tool_calls") | Some("function_call") => Some(StopReason::ToolUse),
            Some("length") => Some(StopReason::MaxTokens),
            Some("stop_sequence") => Some(StopReason::StopSequence),
            _ => Some(StopReason::EndTurn),
        };

        Ok(result)
    }
}
