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

/// Adapter-canonicalized assistant response. Distinct fields by
/// ontology, not duplication:
///   - `content` is the verbatim assistant output (whatever the model
///     emitted as its message). Kept for `cognition.jsonl` so operators
///     see what the model actually said, and appended to the Actor's
///     history so the LLM sees its own prior reasoning on the next
///     inner step. The kernel does NOT parse it.
///   - `action_hint`, `verdict_lines`, `judge_output` are the role-
///     specific strong types the adapter extracted from whichever wire
///     shape the vendor used (harmony envelope, plain prose, vendor-
///     native tool-call fields, etc.). Each role-consumer (Actor,
///     Evaluator, Judge) reads its relevant field directly — the
///     kernel performs no text or JSON parsing.
///
/// Adapters populate the fields they have evidence for; consumers that
/// don't see their field treat it as "the response was content-only
/// for this role" (Actor: continue inner loop; Evaluator: no verdict
/// produced — Gate's empty-trace fallback applies; Judge: same).
#[derive(Debug, Clone, Serialize)]
pub struct CognitionResponse {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_hit_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_hint: Option<ActionHint>,
    /// Strong-typed Evaluator output the adapter extracted from
    /// content. One entry per decision the model emitted. Empty when
    /// the adapter found none (pure prose, no recognizable decision).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub verdict_lines: Vec<DecisionLine>,
    /// Strong-typed Judge output the adapter extracted from content.
    /// `None` when the adapter found no parseable Judge output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_output: Option<crate::scenario::JudgeOutput>,
}

/// One adapter-extracted Evaluator decision line. The adapter produces
/// these without knowing which Evaluator role is consuming them; the
/// LlmEvaluator stamps its own `evaluator_id` when wrapping these
/// into `EvaluatorEntry` for the Gate.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionLine {
    pub decision: crate::verdict::Decision,
    pub observation: String,
}

/// Strong-typed Action surface that the adapter produces from a vendor
/// response. The Actor consumes this directly — there is no JSON-from-
/// string fallback in the kernel.
///
/// `Propose.params` is `serde_json::Value` because params are dynamic
/// per Tool (the kernel doesn't know all Tools' schemas — the executor
/// deserializes into its specific shape). Everything else is typed.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionHint {
    Propose {
        tool: String,
        params: serde_json::Value,
    },
    Halt,
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

    /// Enqueue a pure-reasoning response — `action_hint` is `None`.
    /// The Actor treats this as an inner-loop reasoning step and
    /// continues without committing an Action.
    pub fn enqueue(&self, content: impl Into<String>) -> &Self {
        self.queue.lock().unwrap().push_back(content_only(content.into()));
        self
    }

    /// Enqueue a structured Action response — what the adapter would
    /// have canonicalized from a vendor wire shape. The Actor commits
    /// immediately on this. `content` is auto-derived for
    /// `cognition.jsonl` visibility (and so the Actor's history shows
    /// the action the model emitted).
    pub fn enqueue_action(&self, action: ActionHint) -> &Self {
        let content = match &action {
            ActionHint::Halt => r#"{"halt":true}"#.to_string(),
            ActionHint::Propose { tool, params } => {
                serde_json::json!({ "tool": tool, "params": params }).to_string()
            }
        };
        let mut r = content_only(content);
        r.action_hint = Some(action);
        self.queue.lock().unwrap().push_back(r);
        self
    }

    /// Enqueue an explicit (content, action_hint) pair. Used by the
    /// runtime when configuring fake backends from TOML
    /// `fake_responses` — the strings stay JSON-shaped for operator
    /// ergonomics, and the runtime canonicalizes each into an
    /// `ActionHint` before enqueueing.
    pub fn enqueue_with_action(
        &self,
        content: impl Into<String>,
        action_hint: Option<ActionHint>,
    ) -> &Self {
        let mut r = content_only(content.into());
        r.action_hint = action_hint;
        self.queue.lock().unwrap().push_back(r);
        self
    }

    /// Enqueue a strong-typed Evaluator response — the adapter
    /// equivalent for fake-LLM tests. `content` auto-derived so the
    /// cognition.jsonl record reflects what a real model would have
    /// emitted (line-shaped DECISION text).
    pub fn enqueue_verdict_lines(&self, lines: Vec<DecisionLine>) -> &Self {
        let content = lines
            .iter()
            .map(|l| format!("{}: {}", decision_to_keyword(l.decision), l.observation))
            .collect::<Vec<_>>()
            .join("\n");
        let mut r = content_only(content);
        r.verdict_lines = lines;
        self.queue.lock().unwrap().push_back(r);
        self
    }

    /// Enqueue a strong-typed Judge response.
    pub fn enqueue_judge_output(&self, output: crate::scenario::JudgeOutput) -> &Self {
        let content = serde_json::to_string(&output).unwrap_or_default();
        let mut r = content_only(content);
        r.judge_output = Some(output);
        self.queue.lock().unwrap().push_back(r);
        self
    }

    pub fn enqueue_with_tokens(
        &self,
        content: impl Into<String>,
        input_tokens: u32,
        output_tokens: u32,
        cache_hit_tokens: u32,
    ) -> &Self {
        let mut r = content_only(content.into());
        r.input_tokens = input_tokens;
        r.output_tokens = output_tokens;
        r.cache_hit_tokens = cache_hit_tokens;
        self.queue.lock().unwrap().push_back(r);
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

/// Construct a `CognitionResponse` that carries only verbatim content
/// — no role-specific strong types. The Actor's inner loop treats this
/// as a reasoning step; the Evaluator's empty-trace path applies; the
/// Judge sees no parseable output. Used by every `enqueue_*` variant
/// as the base before stamping the relevant role's strong type.
fn content_only(content: String) -> CognitionResponse {
    CognitionResponse {
        content,
        input_tokens: 0,
        output_tokens: 0,
        cache_hit_tokens: 0,
        action_hint: None,
        verdict_lines: Vec::new(),
        judge_output: None,
    }
}

fn decision_to_keyword(d: crate::verdict::Decision) -> &'static str {
    match d {
        crate::verdict::Decision::Allow => "ALLOW",
        crate::verdict::Decision::Deny => "DENY",
        crate::verdict::Decision::Escalate => "ESCALATE",
        crate::verdict::Decision::Defer => "DEFER",
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
