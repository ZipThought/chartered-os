//! Shared test fixtures for kernel integration tests.
//!
//! In-memory ToolExecutors used to demonstrate the dispatch path and
//! the reconciliation invariant without touching the OS. NOT production
//! Tools — production effects live in `chartered-dispatch`. Loaded by
//! integration tests via `mod common;`.
//!
//! Each integration test binary compiles `common/mod.rs` independently
//! and may use only a subset of these helpers; `#![allow(dead_code)]`
//! suppresses the per-binary unused warnings that would otherwise
//! result.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use chartered_core::{
    ActionHint, Charter, Decision, DecisionLine, FakeCognitionBackend, Frame, FrameId,
    GovernanceMode, LlmActor, LlmEvaluator, RoleContext, Snapshot, Steward, StewardId,
    ToolExecutor, ToolId, ToolParams, ToolRegistry, ToolResult, Workspace, WorkspaceId,
};

/// Build a Frame backed by an `LlmEvaluator` and a `FakeCognitionBackend`.
/// Test inputs are line-shaped Evaluator responses like `"ALLOW: ok"` —
/// this helper translates each into a `Vec<DecisionLine>` (mimicking
/// what a real adapter does on the LLM output) and enqueues via the
/// fake backend's strong-typed `enqueue_verdict_lines`. The kernel
/// never parses these lines (see `AGENTS.md §Verification`); this
/// helper is the test-fixture analogue of a production adapter.
///
/// Strings that contain no recognized decision keyword are enqueued as
/// pure content (no `verdict_lines`) — used by tests that want to
/// exercise the Gate's empty-trace fallback (UNGROUNDED).
pub fn make_frame(id: &str, applies: &[&str], responses: &[&str]) -> Frame {
    let backend = Arc::new(FakeCognitionBackend::new(format!("eval-{id}")));
    for r in responses {
        let lines = verdict_lines_from_test_string(r);
        if lines.is_empty() {
            backend.enqueue(r.to_string());
        } else {
            backend.enqueue_verdict_lines(lines);
        }
    }
    let evaluator = Arc::new(LlmEvaluator::new(
        format!("llm-eval-{id}"),
        backend,
        FrameId::new(id),
        format!("test concern {id}"),
    ));
    Frame {
        id: FrameId::new(id),
        concern: format!("frame {id}"),
        declared_scopes: vec![],
        applies_to_tools: applies.iter().map(|s| ToolId::new(*s)).collect(),
        evaluator,
        prior_receipt_queries: vec![],
    }
}

/// Parse Evaluator-shaped test fixture strings into `DecisionLine`s.
/// One line per line of input that starts with a recognized keyword.
/// Lines without a recognized keyword produce no DecisionLine (the
/// caller can decide whether to fall back to plain content).
fn verdict_lines_from_test_string(text: &str) -> Vec<DecisionLine> {
    text.lines()
        .filter_map(|line| {
            let (head, tail) = line.split_once(':')?;
            let decision = match head.trim().to_uppercase().as_str() {
                "ALLOW" => Decision::Allow,
                "DENY" => Decision::Deny,
                "ESCALATE" => Decision::Escalate,
                "DEFER" => Decision::Defer,
                _ => return None,
            };
            Some(DecisionLine {
                decision,
                observation: tail.trim().to_string(),
            })
        })
        .collect()
}

/// Build an `LlmActor` whose backend dequeues one Action per `step()`.
/// Test inputs are `serde_json::Value` shaped as the canonical Action
/// envelope (`{"tool":"...","params":{...}}` or `{"halt":true}`) — this
/// helper translates each into an `ActionHint` and enqueues it via the
/// fake backend's strong-typed surface. The kernel never parses JSON
/// (spec §Cognition Layer: kernel consumes the strong-typed
/// `action_hint`); this helper is the test-fixture analogue of what
/// real adapters do in production.
pub fn make_llm_actor(id: &str, responses: Vec<serde_json::Value>) -> LlmActor {
    let backend = Arc::new(FakeCognitionBackend::new(format!("actor-{id}")));
    for r in responses {
        let action = action_hint_from_value(&r).unwrap_or_else(|| {
            panic!(
                "make_llm_actor: response value is not a canonical Action envelope: {r}"
            )
        });
        backend.enqueue_action(action);
    }
    LlmActor::new(id, backend, "test actor", "ctx-test")
}

fn action_hint_from_value(v: &serde_json::Value) -> Option<ActionHint> {
    if v.get("halt").and_then(serde_json::Value::as_bool) == Some(true) {
        return Some(ActionHint::Halt);
    }
    let tool = v.get("tool").and_then(serde_json::Value::as_str)?;
    let params = v
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Some(ActionHint::Propose {
        tool: tool.to_string(),
        params,
    })
}

/// Build a Snapshot from frames + permitted tool ids.
pub fn make_snapshot(frames: Vec<Frame>, permitted: &[&str]) -> Arc<Snapshot> {
    let charter = Charter {
        frames,
        permitted_tools: permitted.iter().map(|s| ToolId::new(*s)).collect(),
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "test-charter".into(),
        behavioral_spec: String::new(),
    };
    Snapshot::new(charter, RoleContext::empty(), Vec::new())
}

/// Build a Steward with the given id, snapshot, and registry. Wraps the
/// shape every kernel test wants when constructing a single-Steward
/// Workspace.
pub fn make_steward(id: &str, snap: Arc<Snapshot>, registry: ToolRegistry) -> Steward {
    Steward::new(StewardId::new(id), snap, Arc::new(registry))
}

/// Steward configured for passthrough enforcement (Gate writes Receipts
/// but never denies). Replaces the old `LoopRunner::passthrough` shape.
pub fn make_steward_passthrough(
    id: &str,
    snap: Arc<Snapshot>,
    registry: ToolRegistry,
) -> Steward {
    make_steward(id, snap, registry).with_governance_mode(GovernanceMode::GROUNDING_ONLY)
}

/// Wrap a Snapshot + ToolRegistry in a Workspace hosting a single
/// Steward (id = "sut"). The kernel tests that don't care about
/// multi-Steward composition use this — equivalent to the v1
/// single-Steward shape.
pub fn make_workspace(snap: Arc<Snapshot>, registry: ToolRegistry) -> Arc<Workspace> {
    let steward = make_steward("sut", snap, registry);
    Arc::new(
        Workspace::single(WorkspaceId::new("test-workspace"), steward)
            .expect("workspace validates"),
    )
}

/// The unique Steward Arc inside a single-Steward Workspace constructed
/// via `make_workspace`. Tests that need to pass the Steward to
/// `LoopRunner::new` or `ScenarioRunner::new` use this.
pub fn sole_steward(ws: &Arc<Workspace>) -> Arc<Steward> {
    ws.sole_steward().clone()
}

/// Tool that returns `Ok({})` for every call. Used by gate/loop tests
/// that only care about dispatch happening, not what the tool does.
pub struct NopTool {
    id: ToolId,
}

impl NopTool {
    pub fn new(id: &str) -> Self {
        Self {
            id: ToolId::new(id),
        }
    }
}

#[async_trait]
impl ToolExecutor for NopTool {
    fn id(&self) -> &ToolId {
        &self.id
    }
    async fn execute(&self, _: &ToolParams) -> ToolResult {
        Ok(json!({}))
    }
}

/// `ToolRegistry` populated with `NopTool`s for each id in `tools`.
pub fn registry_with_nops(tools: &[&str]) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    for t in tools {
        reg.register(Arc::new(NopTool::new(t)));
    }
    reg
}

/// In-memory message log shared with `SendMessageTool`. The
/// reconciliation invariant (CHECKLIST §Receipt System > Reconciliation)
/// is verifiable by inspecting the log after a Receipt: ALLOWED →
/// message present; DENIED → message absent.
#[derive(Debug, Clone, Default)]
pub struct MessageLog {
    inner: Arc<Mutex<Vec<LoggedMessage>>>,
}

#[derive(Debug, Clone)]
pub struct LoggedMessage {
    pub channel: String,
    pub content: String,
}

impl MessageLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<LoggedMessage> {
        self.inner.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
}

/// `send_message(channel, content)` — appends to the shared MessageLog.
pub struct SendMessageTool {
    id: ToolId,
    log: MessageLog,
}

impl SendMessageTool {
    pub fn new(log: MessageLog) -> Self {
        Self {
            id: ToolId::new("send_message"),
            log,
        }
    }
}

#[async_trait]
impl ToolExecutor for SendMessageTool {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        let channel = params.require_str("channel")?.to_string();
        let content = params.require_str("content")?.to_string();
        self.log
            .inner
            .lock()
            .unwrap()
            .push(LoggedMessage { channel, content });
        Ok(json!({ "delivered": true }))
    }
}

/// In-memory file system shared with `WriteSpecTool`.
#[derive(Debug, Clone, Default)]
pub struct InMemoryFs {
    inner: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

impl InMemoryFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&self, path: &str) -> Option<String> {
        self.inner.lock().unwrap().get(path).cloned()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.inner.lock().unwrap().contains_key(path)
    }

    pub fn snapshot(&self) -> std::collections::HashMap<String, String> {
        self.inner.lock().unwrap().clone()
    }
}

/// `write_spec(path, content)` — writes to the in-memory FS.
pub struct WriteSpecTool {
    id: ToolId,
    fs: InMemoryFs,
}

impl WriteSpecTool {
    pub fn new(fs: InMemoryFs) -> Self {
        Self {
            id: ToolId::new("write_spec"),
            fs,
        }
    }
}

#[async_trait]
impl ToolExecutor for WriteSpecTool {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        let path = params.require_str("path")?.to_string();
        let content = params.require_str("content")?.to_string();
        self.fs.inner.lock().unwrap().insert(path.clone(), content);
        Ok(json!({ "wrote": path }))
    }
}
