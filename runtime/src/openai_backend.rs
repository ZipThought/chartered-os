//! OpenAI-compatible HTTP CognitionBackend.
//!
//! ONE backend implementation serves both:
//!   - Real OpenAI (`LLM_BASE_URL=https://api.openai.com/v1`,
//!     `LLM_API_KEY=sk-...`)
//!   - Local OpenAI-compatible servers, e.g. LM Studio, llama.cpp,
//!     vLLM, SGLang (`LLM_BASE_URL=http://localhost:1234/v1`, key
//!     optional)
//!
//! The wire format is the OpenAI Chat Completions API
//! (POST `<base>/chat/completions`). When `LLM_API_KEY` is set
//! and non-empty, the request carries `Authorization: Bearer ...`;
//! otherwise no auth header (local servers usually don't need one).
//!
//! Configuration order: env via `dotenvy` (`.env` in CWD or any
//! ancestor) → process env. Per-role `steward.toml` `model` overrides
//! the env's `LLM_MODEL`. The base URL and the API key are always
//! taken from env — secrets and host endpoints don't belong in TOML
//! that may be committed.
//!
//! Vendor wire-format adaptation lives here. Roles consume the
//! canonical `CognitionResponse { text, tool_call_hint }`; the kernel
//! never sees gpt-oss harmony envelopes, markdown code fences around
//! JSON, or `to=tool.<name>` recipient prefixes. `canonicalize_assistant_response`
//! strips those before returning.

use std::env;
use std::time::Duration;

use async_trait::async_trait;
use chartered_core::{
    CognitionBackend, CognitionError, CognitionRequest, CognitionResponse, Message, ToolCallHint,
};
use serde::{Deserialize, Serialize};

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
/// every role hitting the same `LLM_BASE_URL`.
pub struct OpenAiBackendFactory {
    base_url: String,
    default_model: Option<String>,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAiBackendFactory {
    /// Read `LLM_BASE_URL`, `LLM_MODEL`, `LLM_API_KEY` from env once.
    /// `LLM_BASE_URL` is mandatory; `LLM_MODEL` provides the default
    /// model for backends whose per-role `model` is unset; `LLM_API_KEY`
    /// is optional.
    pub fn from_env() -> Result<Self, BackendBuildError> {
        let base_url =
            env::var("LLM_BASE_URL").map_err(|_| BackendBuildError::MissingEnv("LLM_BASE_URL"))?;
        let default_model = env::var("LLM_MODEL").ok();
        let api_key = env::var("LLM_API_KEY").ok().filter(|s| !s.is_empty());
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
    /// `model` from `steward.toml`; falls back to `LLM_MODEL` env when
    /// unset.
    pub fn build(
        &self,
        id: impl Into<String>,
        model_override: Option<String>,
    ) -> Result<OpenAiCompatibleBackend, BackendBuildError> {
        let model = model_override
            .or_else(|| self.default_model.clone())
            .ok_or(BackendBuildError::MissingEnv(
                "LLM_MODEL (set env or steward.toml [<role>] model)",
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

        let raw_text = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();
        let canonical = canonicalize_assistant_response(&raw_text);
        let usage = parsed.usage.unwrap_or_default();

        Ok(CognitionResponse {
            text: canonical.text,
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            cache_hit_tokens: usage.cached_tokens,
            tool_call_hint: canonical.tool_call_hint,
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

struct CanonicalAssistantResponse {
    text: String,
    tool_call_hint: Option<ToolCallHint>,
}

fn canonicalize_assistant_response(raw: &str) -> CanonicalAssistantResponse {
    if let Some(harmony) = parse_harmony_assistant_block(raw) {
        let body = strip_code_fences(harmony.body);
        let json_text = extract_json_object(body).unwrap_or(body);
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) {
            if let Some(recipient) = harmony.recipient
                && value.get("tool").is_none()
                && value.get("halt").is_none()
            {
                let tool_name = strip_harmony_recipient_namespace(recipient).to_string();
                if !tool_name.is_empty() {
                    return CanonicalAssistantResponse {
                        text: json_text.to_string(),
                        tool_call_hint: Some(ToolCallHint {
                            tool: tool_name,
                            params: value,
                        }),
                    };
                }
            }
            return CanonicalAssistantResponse {
                text: json_text.to_string(),
                tool_call_hint: None,
            };
        }
        return CanonicalAssistantResponse {
            text: body.to_string(),
            tool_call_hint: None,
        };
    }
    let cleaned = strip_code_fences(raw);
    let extracted = extract_json_object(cleaned).unwrap_or(cleaned);
    CanonicalAssistantResponse {
        text: extracted.to_string(),
        tool_call_hint: None,
    }
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

fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```JSON"))
        .or_else(|| s.strip_prefix("```"))
        .map(str::trim)
        .unwrap_or(s);
    s.trim_end_matches("```").trim()
}

/// Walk `text` and return the first balanced top-level JSON object
/// substring. String/escape state is tracked so braces inside JSON
/// strings do not confuse depth tracking. Returns None when no
/// balanced object is present.
fn extract_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'"' => in_string = true,
            b'}' => {
                depth -= 1;
                if depth == 0
                    && let Some(s) = start
                {
                    return Some(&text[s..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_from_env_requires_base_url() {
        // Use unsafe block since set/remove env are unsafe in newer Rust.
        unsafe {
            env::remove_var("LLM_BASE_URL");
        }
        let err = match OpenAiBackendFactory::from_env() {
            Ok(_) => panic!("expected MissingEnv error"),
            Err(e) => e,
        };
        assert!(matches!(err, BackendBuildError::MissingEnv("LLM_BASE_URL")));
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
    fn canonicalize_plain_text_passes_through() {
        let c = canonicalize_assistant_response("ALLOW: ok\nDENY: no");
        assert_eq!(c.text, "ALLOW: ok\nDENY: no");
        assert!(c.tool_call_hint.is_none());
    }

    #[test]
    fn canonicalize_markdown_fenced_json_strips_fences() {
        let c = canonicalize_assistant_response(
            "```json\n{\"tool\":\"write_file\",\"params\":{}}\n```",
        );
        assert_eq!(c.text, r#"{"tool":"write_file","params":{}}"#);
        assert!(c.tool_call_hint.is_none());
    }

    #[test]
    fn canonicalize_harmony_envelope_with_full_action_returns_text_no_hint() {
        let raw = "<|channel|>commentary <|message|>{\"tool\":\"write_file\",\"params\":{\"path\":\"x\"}}<|return|>";
        let c = canonicalize_assistant_response(raw);
        assert_eq!(c.text, r#"{"tool":"write_file","params":{"path":"x"}}"#);
        assert!(c.tool_call_hint.is_none());
    }

    #[test]
    fn canonicalize_harmony_with_tool_recipient_extracts_hint() {
        // gpt-oss harmony: tool name in the header (`to=...`),
        // params in the JSON body. Adapter populates the hint.
        let raw = "<|channel|>commentary to=tool.write_file <|constrain|>json<|message|>{\"path\":\"hello.txt\",\"content\":\"hi\"}";
        let c = canonicalize_assistant_response(raw);
        let hint = c.tool_call_hint.expect("expected hint");
        assert_eq!(hint.tool, "write_file");
        assert_eq!(hint.params["path"].as_str(), Some("hello.txt"));
        assert_eq!(hint.params["content"].as_str(), Some("hi"));
    }

    #[test]
    fn canonicalize_harmony_with_functions_recipient() {
        let raw = "<|start|>assistant<|channel|>commentary to=functions.write_file<|constrain|>json<|message|>{\"path\":\"x\"}<|call|>";
        let c = canonicalize_assistant_response(raw);
        let hint = c.tool_call_hint.expect("expected hint");
        assert_eq!(hint.tool, "write_file");
    }

    #[test]
    fn canonicalize_harmony_evaluator_lines_no_recipient_no_json_returns_body() {
        let raw = "<|channel|>analysis<|message|>ALLOW: looks fine\nDENY: but suspicious<|end|>";
        let c = canonicalize_assistant_response(raw);
        assert_eq!(c.text, "ALLOW: looks fine\nDENY: but suspicious");
        assert!(c.tool_call_hint.is_none());
    }

    #[test]
    fn extract_json_object_handles_braces_inside_strings() {
        let raw = r#"prefix {"a":"} not done","b":1} suffix"#;
        assert_eq!(
            extract_json_object(raw),
            Some(r#"{"a":"} not done","b":1}"#)
        );
    }
}
