//! OpenAI-compatible HTTP CognitionBackend.
//!
//! ONE backend implementation serves both:
//!   - Real OpenAI (`OPEN_AI_BASE_URL=https://api.openai.com/v1`,
//!     `OPEN_AI_API_KEY=sk-...`)
//!   - Local OpenAI-compatible servers, e.g. LM Studio, llama.cpp,
//!     vLLM, SGLang (`OPEN_AI_BASE_URL=http://localhost:1234/v1`, key
//!     optional)
//!
//! The wire format is the OpenAI Chat Completions API
//! (POST `<base>/chat/completions`). When `OPEN_AI_API_KEY` is set
//! and non-empty, the request carries `Authorization: Bearer ...`;
//! otherwise no auth header (local servers usually don't need one).
//!
//! Configuration order: env via `dotenvy` (`.env` in CWD or any
//! ancestor) → process env. Per-role `steward.toml` `model` overrides
//! the env's `OPEN_AI_MODEL`. The base URL and the API key are always
//! taken from env — secrets and host endpoints don't belong in TOML
//! that may be committed.
//!
//! Vendor wire-format adaptation lives here. The Actor consumes
//! `CognitionResponse { content, action_hint }`; the kernel never sees
//! gpt-oss harmony envelopes, markdown code fences, or `to=tool.<name>`
//! recipient prefixes. Harmony-envelope unwrapping is local to this
//! file (gpt-oss-specific); markdown-fence stripping and JSON-from-prose
//! extraction live in `crate::canonicalize` because every adapter needs
//! them.

use std::env;
use std::time::Duration;

use async_trait::async_trait;
use chartered_core::{
    ActionHint, CognitionBackend, CognitionError, CognitionRequest, CognitionResponse, Message,
};
use serde::{Deserialize, Serialize};

use crate::canonicalize;

#[derive(Debug, Clone)]
pub enum BackendBuildError {
    MissingEnv(&'static str),
    HttpClient(String),
}

impl std::fmt::Display for BackendBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendBuildError::MissingEnv(name) => {
                write!(f, "missing required environment variable: {name}")
            }
            BackendBuildError::HttpClient(e) => write!(f, "HTTP client init: {e}"),
        }
    }
}

impl std::error::Error for BackendBuildError {}

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Builds `OpenAiCompatibleBackend` instances against a single shared
/// `reqwest::Client`. The runtime constructs one factory per binary
/// invocation, and each role (Actor, Evaluator-per-Frame, Tester,
/// Judge) gets a backend through it. Sharing the client means one
/// connection pool, one TLS session cache, one keep-alive set across
/// every role hitting the same `OPEN_AI_BASE_URL`.
pub struct OpenAiBackendFactory {
    base_url: String,
    default_model: Option<String>,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAiBackendFactory {
    /// Read `OPEN_AI_BASE_URL`, `OPEN_AI_MODEL`, `OPEN_AI_API_KEY` from env once.
    /// `OPEN_AI_BASE_URL` is mandatory; `OPEN_AI_MODEL` provides the default
    /// model for backends whose per-role `model` is unset; `OPEN_AI_API_KEY`
    /// is optional.
    pub fn from_env() -> Result<Self, BackendBuildError> {
        let base_url =
            env::var("OPEN_AI_BASE_URL").map_err(|_| BackendBuildError::MissingEnv("OPEN_AI_BASE_URL"))?;
        let default_model = env::var("OPEN_AI_MODEL").ok();
        let api_key = env::var("OPEN_AI_API_KEY").ok().filter(|s| !s.is_empty());
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .build()
            .map_err(|e| BackendBuildError::HttpClient(e.to_string()))?;
        Ok(Self {
            base_url,
            default_model,
            api_key,
            client,
        })
    }

    /// Build a backend for one role. `model_override` is the per-role
    /// `model` from `steward.toml`; falls back to `OPEN_AI_MODEL` env when
    /// unset.
    pub fn build(
        &self,
        id: impl Into<String>,
        model_override: Option<String>,
    ) -> Result<OpenAiCompatibleBackend, BackendBuildError> {
        let model = model_override
            .or_else(|| self.default_model.clone())
            .ok_or(BackendBuildError::MissingEnv(
                "OPEN_AI_MODEL (set env or steward.toml [<role>] model)",
            ))?;
        Ok(OpenAiCompatibleBackend {
            id: id.into(),
            base_url: self.base_url.clone(),
            model,
            api_key: self.api_key.clone(),
            client: self.client.clone(),
        })
    }
}

pub struct OpenAiCompatibleBackend {
    id: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAiCompatibleBackend {
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl CognitionBackend for OpenAiCompatibleBackend {
    fn id(&self) -> &str {
        &self.id
    }

    async fn complete(
        &self,
        request: &CognitionRequest,
    ) -> Result<CognitionResponse, CognitionError> {
        let body = ChatRequest {
            model: &self.model,
            messages: &request.messages,
            max_tokens: request.max_output_tokens,
        };

        let url = chat_completions_url(&self.base_url);
        let mut http_req = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            http_req = http_req.bearer_auth(key);
        }

        let resp = http_req
            .send()
            .await
            .map_err(|e| CognitionError(format!("HTTP request to {url}: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(CognitionError(format!(
                "HTTP {status} from {url}: {body_text}"
            )));
        }

        let parsed: ChatResponse = resp
            .json()
            .await
            .map_err(|e| CognitionError(format!("decoding JSON from {url}: {e}")))?;

        let raw_content = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();
        let (content, action_hint) = canonicalize_openai_content(&raw_content);
        let verdict_lines = canonicalize::canonicalize_verdict_lines(&content);
        let judge_output = canonicalize::canonicalize_judge_output(&content);
        let usage = parsed.usage.unwrap_or_default();

        Ok(CognitionResponse {
            content,
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            cache_hit_tokens: usage.cached_tokens,
            action_hint,
            verdict_lines,
            judge_output,
        })
    }
}

fn chat_completions_url(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    /// Borrows the kernel's `&[Message]` directly. `MessageRole`'s
    /// `#[serde(rename_all = "lowercase")]` produces the OpenAI-required
    /// "system"/"user"/"assistant" string forms; no intermediate Vec or
    /// String clone of the prompt body.
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: String,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    cached_tokens: u32,
}

// ---------------------------------------------------------------------
// Canonicalization: vendor wire-format → kernel-canonical response.
//
// Models served via OpenAI-compatible endpoints emit several shapes:
//   1. Plain text (or plain JSON for tool calls).
//   2. Markdown-fenced text/JSON: ```json\n{...}\n```
//   3. gpt-oss harmony envelope:
//        <|start|>{header}<|message|>{content}<|end|>
//      with an optional `to=<recipient>` token in the header naming
//      a tool/function the model is calling.
//
// `canonicalize_assistant_response` strips envelopes and fences, and
// extracts a `ToolCallHint` when the wire format separates the tool
// name from its params (harmony `to=` header + JSON body).
// ---------------------------------------------------------------------

/// Convert an OpenAI-compatible assistant message into the kernel's
/// `(content, action_hint)` pair. Three vendor shapes handled:
///   - Plain text (or markdown-fenced JSON / prose-around-JSON):
///     delegate to `canonicalize::canonicalize_action_hint`.
///   - gpt-oss harmony envelope without `to=` recipient: same as plain
///     text, after stripping the envelope.
///   - gpt-oss harmony envelope WITH `to=` recipient: the recipient
///     names the tool; the body is the params JSON. Produce
///     `ActionHint::Propose` directly, bypassing the body's lack of
///     `"tool"` field. (gpt-oss specific.)
fn canonicalize_openai_content(raw: &str) -> (String, Option<ActionHint>) {
    if let Some(harmony) = parse_harmony_assistant_block(raw) {
        let body = canonicalize::strip_code_fences(harmony.body);
        let json_text = canonicalize::extract_first_json_object(body).unwrap_or(body);
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) {
            // Recipient-named tool: the tool name lives in the header,
            // the body is the params. Build Propose from both.
            if let Some(recipient) = harmony.recipient
                && value.get("tool").is_none()
                && value.get("halt").is_none()
            {
                let tool_name = strip_harmony_recipient_namespace(recipient).to_string();
                if !tool_name.is_empty() {
                    return (
                        json_text.to_string(),
                        Some(ActionHint::Propose {
                            tool: tool_name,
                            params: value,
                        }),
                    );
                }
            }
            // Body carries the full Action envelope.
            let hint = canonicalize::parse_action_value(&value);
            return (json_text.to_string(), hint);
        }
        // Harmony body wasn't JSON — pure reasoning.
        return (body.to_string(), None);
    }
    // No harmony envelope — fall through to the shared util.
    let stripped = canonicalize::strip_code_fences(raw);
    let extracted = canonicalize::extract_first_json_object(stripped).unwrap_or(stripped);
    let hint = canonicalize::canonicalize_action_hint(raw);
    (extracted.to_string(), hint)
}

/// gpt-oss harmony assistant block parts.
/// `recipient` is set when the message header carried `to=<name>`
/// (the model is calling a function/tool); `body` is the message
/// content between `<|message|>` and the next `<|...|>` token.
struct HarmonyMessage<'a> {
    recipient: Option<&'a str>,
    body: &'a str,
}

fn parse_harmony_assistant_block(text: &str) -> Option<HarmonyMessage<'_>> {
    let last_msg = text.rfind("<|message|>")?;
    let header = &text[..last_msg];
    let after_msg = &text[last_msg + "<|message|>".len()..];
    let body_end = after_msg.find("<|").unwrap_or(after_msg.len());
    let body = after_msg[..body_end].trim();
    let recipient = header.rfind("to=").map(|i| {
        let rest = &header[i + 3..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '<')
            .unwrap_or(rest.len());
        rest[..end].trim()
    });
    Some(HarmonyMessage { recipient, body })
}

fn strip_harmony_recipient_namespace(s: &str) -> &str {
    s.trim_start_matches("functions.")
        .trim_start_matches("tool.")
        .trim_start_matches("tools.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_from_env_requires_base_url() {
        // Use unsafe block since set/remove env are unsafe in newer Rust.
        unsafe {
            env::remove_var("OPEN_AI_BASE_URL");
        }
        let err = match OpenAiBackendFactory::from_env() {
            Ok(_) => panic!("expected MissingEnv error"),
            Err(e) => e,
        };
        assert!(matches!(err, BackendBuildError::MissingEnv("OPEN_AI_BASE_URL")));
    }

    #[test]
    fn chat_completions_url_preserves_configured_api_version() {
        assert_eq!(
            chat_completions_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("http://localhost:1234/v1/"),
            "http://localhost:1234/v1/chat/completions"
        );
    }

    #[test]
    fn canonicalize_plain_text_yields_no_action_hint() {
        let (content, hint) = canonicalize_openai_content("ALLOW: ok\nDENY: no");
        assert_eq!(content, "ALLOW: ok\nDENY: no");
        assert!(hint.is_none());
    }

    #[test]
    fn canonicalize_markdown_fenced_action_yields_propose() {
        let (_content, hint) =
            canonicalize_openai_content("```json\n{\"tool\":\"write_file\",\"params\":{}}\n```");
        match hint {
            Some(ActionHint::Propose { tool, .. }) => assert_eq!(tool, "write_file"),
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn canonicalize_harmony_envelope_with_full_action_yields_propose() {
        let raw = "<|channel|>commentary <|message|>{\"tool\":\"write_file\",\"params\":{\"path\":\"x\"}}<|return|>";
        let (_content, hint) = canonicalize_openai_content(raw);
        match hint {
            Some(ActionHint::Propose { tool, params }) => {
                assert_eq!(tool, "write_file");
                assert_eq!(params["path"].as_str(), Some("x"));
            }
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn canonicalize_harmony_with_tool_recipient_extracts_propose() {
        // gpt-oss harmony: tool name in the header (`to=...`),
        // params in the JSON body. The recipient names the tool.
        let raw = "<|channel|>commentary to=tool.write_file <|constrain|>json<|message|>{\"path\":\"hello.txt\",\"content\":\"hi\"}";
        let (_content, hint) = canonicalize_openai_content(raw);
        match hint {
            Some(ActionHint::Propose { tool, params }) => {
                assert_eq!(tool, "write_file");
                assert_eq!(params["path"].as_str(), Some("hello.txt"));
                assert_eq!(params["content"].as_str(), Some("hi"));
            }
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn canonicalize_harmony_with_functions_recipient() {
        let raw = "<|start|>assistant<|channel|>commentary to=functions.write_file<|constrain|>json<|message|>{\"path\":\"x\"}<|call|>";
        let (_content, hint) = canonicalize_openai_content(raw);
        match hint {
            Some(ActionHint::Propose { tool, .. }) => assert_eq!(tool, "write_file"),
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn canonicalize_harmony_evaluator_lines_no_action_hint() {
        let raw = "<|channel|>analysis<|message|>ALLOW: looks fine\nDENY: but suspicious<|end|>";
        let (content, hint) = canonicalize_openai_content(raw);
        assert_eq!(content, "ALLOW: looks fine\nDENY: but suspicious");
        assert!(hint.is_none());
    }
}
