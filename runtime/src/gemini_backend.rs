//! Gemini cognition backend — Google's native generativelanguage API.
//!
//! Spec correspondence: `SPECIFICATION.md §Cognition Layer` ("Cognition
//! is commodity") — this is a parallel implementation to
//! `OpenAiCompatibleBackend`, swapping at the `CognitionBackend` trait.
//! The kernel sees the same `CognitionRequest` / `CognitionResponse`;
//! the wire-shape translation lives here.
//!
//! Endpoint: `https://generativelanguage.googleapis.com/v1beta/models/<model>:generateContent?key=<key>`.
//! Auth: query-parameter `key=`, not Bearer header.
//!
//! Role mapping:
//!   `Message::System`     → top-level `systemInstruction.parts[].text`
//!                           (concatenated when multiple System messages
//!                           appear; Gemini accepts only one).
//!   `Message::User`       → `contents[].role = "user"`.
//!   `Message::Assistant`  → `contents[].role = "model"`.
//!
//! Native tools (`googleSearch`, `codeExecution`, `urlContext`) are
//! intentionally NOT forwarded. Per `SPECIFICATION.md §Tools`, LLM-side
//! native tools are cognition, not Tools — any capability that needs
//! governance is exposed as a registered Charter Tool. This backend
//! ships the message body only.
//!
//! `GEMINI_GENERATE_CONTENT_API` env var is read for informational
//! purposes but ignored — this backend always uses `generateContent`
//! (single response, not the streaming variant). Streaming support is
//! deferred.

use std::env;
use std::time::Duration;

use std::sync::Arc;

use async_trait::async_trait;
use chartered_core::{
    CognitionBackend, CognitionError, CognitionRequest, CognitionResponse, Message, MessageRole,
};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use crate::canonicalize;
use crate::openai_backend::BackendBuildError;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Per-deployment Gemini backend factory. Reads `GEMINI_*` env once,
/// shares the `reqwest::Client` across roles (one connection pool, one
/// TLS session cache).
pub struct GeminiBackendFactory {
    base_url: String,
    default_model: Option<String>,
    api_key: String,
    client: reqwest::Client,
}

impl GeminiBackendFactory {
    /// Read `GEMINI_API_KEY`, `GEMINI_MODEL_ID` (optional default
    /// model), and `GEMINI_BASE_URL` (optional override of the default
    /// `generativelanguage.googleapis.com/v1beta`) from env.
    pub fn from_env() -> Result<Self, BackendBuildError> {
        let api_key = env::var("GEMINI_API_KEY")
            .map_err(|_| BackendBuildError::MissingEnv("GEMINI_API_KEY"))?;
        if api_key.is_empty() {
            return Err(BackendBuildError::MissingEnv("GEMINI_API_KEY (empty)"));
        }
        let base_url = env::var("GEMINI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let default_model = env::var("GEMINI_MODEL_ID").ok().filter(|s| !s.is_empty());
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

    pub fn build(
        &self,
        id: impl Into<String>,
        model_override: Option<String>,
    ) -> Result<GeminiBackend, BackendBuildError> {
        let model = model_override.or_else(|| self.default_model.clone()).ok_or(
            BackendBuildError::MissingEnv("GEMINI_MODEL_ID (set env or steward.toml [<role>] model)"),
        )?;
        Ok(GeminiBackend {
            id: id.into(),
            base_url: self.base_url.clone(),
            model,
            api_key: self.api_key.clone(),
            client: self.client.clone(),
            model_info: Arc::new(OnceCell::new()),
        })
    }
}

pub struct GeminiBackend {
    id: String,
    base_url: String,
    model: String,
    api_key: String,
    client: reqwest::Client,
    /// Lazily-fetched per-model token limits and capabilities, from
    /// `GET /v1beta/models/<model>`. Cached after the first call so we
    /// don't pay the metadata round-trip on every `complete()`.
    /// `outputTokenLimit` is the API's hard cap; for thinking models
    /// the kernel's generic `max_output_tokens` hint (typically 1024)
    /// is too small to leave room for both reasoning and answer, so
    /// the adapter ignores the hint and uses the model's full limit.
    model_info: Arc<OnceCell<ModelInfo>>,
}

#[derive(Clone, Debug)]
struct ModelInfo {
    output_token_limit: u32,
    thinking: bool,
}

#[derive(Deserialize)]
struct ModelInfoResponse {
    /// Required per ai.google.dev/api/models > Resource: Model. Absence
    /// from a 200 response is a schema deviation; serde decode fails
    /// and the adapter surfaces it as a CognitionError rather than
    /// substituting a fallback (AGENTS.md §Engineering Law > Error
    /// Discipline > Fallback prohibition).
    #[serde(rename = "outputTokenLimit")]
    output_token_limit: u32,
    /// Required per ai.google.dev/api/models. Same handling as
    /// `output_token_limit` — absence is a decode failure.
    thinking: bool,
}

impl GeminiBackend {
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Lazily fetch (and cache) the model's metadata from
    /// `GET {base}/models/{model}?key=...`. Subsequent calls hit the
    /// `OnceCell` cache. Returns `CognitionError` if the metadata
    /// endpoint is unreachable or the response can't be decoded — the
    /// kernel surfaces this as an actor failure with operator-visible
    /// reason, never silently substituting defaults.
    async fn model_info(&self) -> Result<ModelInfo, CognitionError> {
        self.model_info
            .get_or_try_init(|| async { self.fetch_model_info().await })
            .await
            .cloned()
    }

    async fn fetch_model_info(&self) -> Result<ModelInfo, CognitionError> {
        let url = format!(
            "{}/models/{}",
            self.base_url.trim_end_matches('/'),
            self.model
        );
        let url_display = format!("{url}?key=<redacted>");
        let resp = self
            .client
            .get(&url)
            .query(&[("key", &self.api_key)])
            .send()
            .await
            .map_err(|e| CognitionError(format!("HTTP request to {url_display}: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CognitionError(format!(
                "HTTP {status} from {url_display}: {body}"
            )));
        }

        let parsed: ModelInfoResponse = resp
            .json()
            .await
            .map_err(|e| CognitionError(format!("decoding JSON from {url_display}: {e}")))?;

        Ok(ModelInfo {
            output_token_limit: parsed.output_token_limit,
            thinking: parsed.thinking,
        })
    }
}

/// Choose the actual `maxOutputTokens` to send to Gemini. Thinking
/// models need their full output budget for reasoning + answer; the
/// kernel's generic 1024 hint isn't meaningful for them, so the
/// adapter overrides with `outputTokenLimit`. Non-thinking models
/// honor the kernel's hint (capped at `outputTokenLimit`).
fn effective_max_output_tokens(info: &ModelInfo, kernel_hint: Option<u32>) -> u32 {
    if info.thinking {
        return info.output_token_limit;
    }
    match kernel_hint {
        Some(h) => h.min(info.output_token_limit),
        None => info.output_token_limit,
    }
}

#[async_trait]
impl CognitionBackend for GeminiBackend {
    fn id(&self) -> &str {
        &self.id
    }

    async fn complete(
        &self,
        request: &CognitionRequest,
    ) -> Result<CognitionResponse, CognitionError> {
        // Effective max-output-tokens comes from the model's metadata
        // (per ai.google.dev/api/models > Model.outputTokenLimit), not
        // the kernel's generic hint. Thinking models in particular need
        // most of their budget for the reasoning phase; 1024 leaves no
        // room for the answer. Metadata is fetched lazily once and
        // cached.
        let info = self.model_info().await?;
        let effective_max = effective_max_output_tokens(&info, request.max_output_tokens);
        let body = build_request_body(&request.messages, Some(effective_max));
        let url = generate_content_url(&self.base_url, &self.model);

        let resp = self
            .client
            .post(&url)
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .await
            .map_err(|e| CognitionError(format!("HTTP request to {url}: {e}")))?;

        let status = resp.status();
        let url_display = redact_key(&url);
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(CognitionError(format!(
                "HTTP {status} from {url_display}: {body_text}"
            )));
        }

        let parsed: GeminiResponse = resp.json().await.map_err(|e| {
            CognitionError(format!("decoding JSON from {url_display}: {e}"))
        })?;

        let content = parsed
            .candidates
            .first()
            .and_then(|c| c.content.as_ref())
            .map(|inner| {
                inner
                    .parts
                    .iter()
                    .filter_map(|p| p.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        // Gemini commonly emits reasoning prose followed by the action
        // JSON ("I'll write the file. {...}"). Universal extraction
        // lives in the shared canonicalize util (markdown fences,
        // prose-around-JSON). Gemini-native function-calling structures
        // aren't yet wired — when we add them, parse here and return an
        // ActionHint directly without going through the shared util.
        let action_hint = canonicalize::canonicalize_action_hint(&content);
        let verdict_lines = canonicalize::canonicalize_verdict_lines(&content);
        let judge_output = canonicalize::canonicalize_judge_output(&content);

        let usage = parsed.usage_metadata.unwrap_or_default();

        Ok(CognitionResponse {
            content,
            input_tokens: usage.prompt_token_count,
            output_tokens: usage.candidates_token_count,
            cache_hit_tokens: usage.cached_content_token_count,
            action_hint,
            verdict_lines,
            judge_output,
        })
    }
}

fn generate_content_url(base_url: &str, model: &str) -> String {
    format!(
        "{}/models/{}:generateContent",
        base_url.trim_end_matches('/'),
        model
    )
}

/// Strip the `key=` query parameter from a URL for error messages so
/// the API key never lands in cognition.jsonl or operator logs.
fn redact_key(url: &str) -> String {
    if let Some(idx) = url.find("?key=") {
        format!("{}?key=<redacted>", &url[..idx])
    } else if let Some(idx) = url.find("&key=") {
        format!("{}&key=<redacted>", &url[..idx])
    } else {
        url.to_string()
    }
}

/// Translate kernel `Vec<Message>` → Gemini request body. System
/// messages collapse into `systemInstruction`; user/assistant pairs map
/// to `contents[].role = "user" | "model"`. Model identity travels in
/// the URL path (`models/<model>:generateContent`), not in the body —
/// Gemini's schema is strict and rejects extra fields.
fn build_request_body(
    messages: &[Message],
    max_output_tokens: Option<u32>,
) -> GeminiRequest {
    let mut system_text = String::new();
    let mut contents: Vec<GeminiContent> = Vec::with_capacity(messages.len());

    for msg in messages {
        match msg.role {
            MessageRole::System => {
                if !system_text.is_empty() {
                    system_text.push('\n');
                }
                system_text.push_str(&msg.content);
            }
            MessageRole::User => contents.push(GeminiContent {
                role: "user",
                parts: vec![GeminiPart {
                    text: Some(msg.content.clone()),
                }],
            }),
            MessageRole::Assistant => contents.push(GeminiContent {
                role: "model",
                parts: vec![GeminiPart {
                    text: Some(msg.content.clone()),
                }],
            }),
        }
    }

    let system_instruction = if system_text.is_empty() {
        None
    } else {
        Some(GeminiContent {
            role: "system",
            parts: vec![GeminiPart {
                text: Some(system_text),
            }],
        })
    };

    GeminiRequest {
        contents,
        system_instruction,
        generation_config: max_output_tokens.map(|n| GenerationConfig {
            max_output_tokens: Some(n),
        }),
        // Native tools (googleSearch, codeExecution, urlContext)
        // intentionally omitted — see module header.
    }
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize)]
struct GeminiContent {
    role: &'static str,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Serialize)]
struct GenerationConfig {
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(rename = "usageMetadata", default)]
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<RespContent>,
}

#[derive(Deserialize)]
struct RespContent {
    #[serde(default)]
    parts: Vec<RespPart>,
}

#[derive(Deserialize)]
struct RespPart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Default)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount", default)]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount", default)]
    candidates_token_count: u32,
    #[serde(rename = "cachedContentTokenCount", default)]
    cached_content_token_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_content_url_appends_model_and_method() {
        assert_eq!(
            generate_content_url("https://generativelanguage.googleapis.com/v1beta", "gemma-3-27b-it"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemma-3-27b-it:generateContent"
        );
        assert_eq!(
            generate_content_url("https://example.com/v1beta/", "m"),
            "https://example.com/v1beta/models/m:generateContent"
        );
    }

    #[test]
    fn effective_max_overrides_kernel_hint_for_thinking_models() {
        let info = ModelInfo {
            output_token_limit: 8192,
            thinking: true,
        };
        // Kernel hint of 1024 is ignored; thinking model needs the full budget.
        assert_eq!(effective_max_output_tokens(&info, Some(1024)), 8192);
        assert_eq!(effective_max_output_tokens(&info, None), 8192);
    }

    #[test]
    fn effective_max_honors_kernel_hint_for_non_thinking_models() {
        let info = ModelInfo {
            output_token_limit: 8192,
            thinking: false,
        };
        // Kernel cap of 1024 is honored.
        assert_eq!(effective_max_output_tokens(&info, Some(1024)), 1024);
        // None → use model max.
        assert_eq!(effective_max_output_tokens(&info, None), 8192);
        // Kernel hint above model max is clamped down.
        assert_eq!(effective_max_output_tokens(&info, Some(16384)), 8192);
    }

    #[test]
    fn redact_key_hides_query_param() {
        assert_eq!(
            redact_key("https://x/models/y:generateContent?key=AIza-secret"),
            "https://x/models/y:generateContent?key=<redacted>"
        );
        assert_eq!(
            redact_key("https://x/models/y:generateContent?foo=1&key=AIza-secret"),
            "https://x/models/y:generateContent?foo=1&key=<redacted>"
        );
        assert_eq!(
            redact_key("https://x/models/y:generateContent"),
            "https://x/models/y:generateContent"
        );
    }

    #[test]
    fn build_request_body_collapses_system_messages_and_maps_roles() {
        let msgs = vec![
            Message::system("first system line"),
            Message::user("hello"),
            Message::system("second system line"),
            Message::assistant("hi"),
            Message::user("how are you"),
        ];
        let req = build_request_body(&msgs, Some(64));

        // Both System messages join into one systemInstruction.
        let si = req.system_instruction.expect("system instruction set");
        assert_eq!(si.role, "system");
        let si_text = si.parts[0].text.as_deref().unwrap_or("");
        assert!(si_text.contains("first system line"));
        assert!(si_text.contains("second system line"));
        assert!(si_text.contains('\n'));

        // Assistant → "model", User → "user".
        assert_eq!(req.contents.len(), 3);
        assert_eq!(req.contents[0].role, "user");
        assert_eq!(req.contents[1].role, "model");
        assert_eq!(req.contents[2].role, "user");

        // generationConfig honored.
        assert_eq!(
            req.generation_config
                .as_ref()
                .and_then(|c| c.max_output_tokens),
            Some(64)
        );
    }

    #[test]
    fn build_request_body_no_system_omits_field() {
        let msgs = vec![Message::user("hello")];
        let req = build_request_body(&msgs, None);
        assert!(req.system_instruction.is_none());
        assert!(req.generation_config.is_none());
    }
}
