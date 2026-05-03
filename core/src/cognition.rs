//! Cognition backend: the LLM-call abstraction.
//!
//! All four roles (Actor, Evaluator, Tester, Judge) call out to a
//! CognitionBackend rather than embedding LLM logic. The trait boundary
//! is the call itself, not the role's logic. One role implementation
//! consumes the trait; the swap to fake or real happens here.
//!
//! Spec §Cognition Layer: "The Runtime manages cognition as
//! infrastructure — assembling prompts, managing prefix caching,
//! dispatching to the preconfigured model backend, enforcing resource
//! limits."
//!
//! Vendor wire-format knowledge (gpt-oss harmony envelopes, OpenAI
//! tool_calls, Anthropic tool_use blocks, markdown code fences around
//! JSON) is the adapter's responsibility. Adapters canonicalize their
//! raw output into `CognitionResponse { text, tool_call_hint }` before
//! returning. Roles consume that canonical shape and never branch on
//! backend kind.
//!
//! `FakeCognitionBackend` is the test-grade stand-in: tests enqueue the
//! responses they expect the LLM to produce; the backend dequeues one
//! per `complete()` call. The role's prompt assembly, response parsing,
//! and decision derivation run identically against fake and real
//! backends — surfacing integration bugs (parse failures, prompt
//! malformation, error propagation) at fake-mode CI rather than live.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CognitionRequest {
    pub messages: Vec<Message>,
    pub max_output_tokens: Option<u32>,
}

/// Adapter-canonicalized assistant response. `text` is the bare body
/// the role consumes for line-based or JSON parsing — adapters strip
/// vendor envelopes (gpt-oss harmony, markdown fences, etc.) before
/// returning. `tool_call_hint` is set when the wire format separated
/// the tool name from its params (harmony `to=` header + JSON body,
/// OpenAI native `tool_calls`, etc.) — the Actor consumes the hint
/// directly without re-parsing `text`.
#[derive(Debug, Clone, Serialize)]
pub struct CognitionResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_hit_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_hint: Option<ToolCallHint>,
}

/// Adapter-extracted structured tool call. When present, the Actor
/// builds a `ToolCall` from this directly without parsing `text`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallHint {
    pub tool: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CognitionError(pub String);

impl std::fmt::Display for CognitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CognitionError {}

/// The LLM-call abstraction. Spec §Vocabulary > Runtime: "Per-deployment
/// process that hosts the Actor loop ... dispatches to the preconfigured
/// model backend." Implementations: `FakeCognitionBackend` (queue-based
/// test stand-in); `OpenAiCompatibleBackend` (production HTTP).
///
/// `complete` takes `&CognitionRequest` so wrappers like `LoggingBackend`
/// can both pass the request to the inner backend and serialize it for
/// the cognition log without paying a deep clone of the prompt.
#[async_trait]
pub trait CognitionBackend: Send + Sync {
    fn id(&self) -> &str;
    async fn complete(
        &self,
        request: &CognitionRequest,
    ) -> Result<CognitionResponse, CognitionError>;
}

/// Test-grade backend: tests enqueue the LLM responses they expect; the
/// backend dequeues one per `complete()` call. Empty queue → error
/// (test underspecification surfaces rather than producing a
/// fabricated-green pass).
pub struct FakeCognitionBackend {
    id: String,
    queue: Mutex<VecDeque<CognitionResponse>>,
    calls: Mutex<Vec<CognitionRequest>>,
}

impl FakeCognitionBackend {
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self {
            id: id.into(),
            queue: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Enqueue a canonical text response.
    pub fn enqueue(&self, text: impl Into<String>) -> &Self {
        self.queue.lock().unwrap().push_back(CognitionResponse {
            text: text.into(),
            input_tokens: 0,
            output_tokens: 0,
            cache_hit_tokens: 0,
            tool_call_hint: None,
        });
        self
    }

    /// Enqueue a structured tool-call hint. Used to simulate adapters
    /// that pre-extract `(tool, params)` from a tool-use wire format.
    pub fn enqueue_hint(&self, tool: impl Into<String>, params: serde_json::Value) -> &Self {
        self.queue.lock().unwrap().push_back(CognitionResponse {
            text: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_hit_tokens: 0,
            tool_call_hint: Some(ToolCallHint {
                tool: tool.into(),
                params,
            }),
        });
        self
    }

    pub fn enqueue_with_tokens(
        &self,
        text: impl Into<String>,
        input_tokens: u32,
        output_tokens: u32,
        cache_hit_tokens: u32,
    ) -> &Self {
        self.queue.lock().unwrap().push_back(CognitionResponse {
            text: text.into(),
            input_tokens,
            output_tokens,
            cache_hit_tokens,
            tool_call_hint: None,
        });
        self
    }

    pub fn pending(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// Snapshot the request log — every CognitionRequest the backend has
    /// served, in call order. Lets tests inspect what the role assembled.
    pub fn calls(&self) -> Vec<CognitionRequest> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl CognitionBackend for FakeCognitionBackend {
    fn id(&self) -> &str {
        &self.id
    }

    async fn complete(
        &self,
        request: &CognitionRequest,
    ) -> Result<CognitionResponse, CognitionError> {
        self.calls.lock().unwrap().push(request.clone());
        match self.queue.lock().unwrap().pop_front() {
            Some(r) => Ok(r),
            None => Err(CognitionError(format!(
                "FakeCognitionBackend({}) queue empty — test underspecified the LLM responses",
                self.id
            ))),
        }
    }
}
