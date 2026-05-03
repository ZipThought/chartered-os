use std::sync::Arc;

use async_trait::async_trait;

use crate::cognition::{CognitionBackend, CognitionRequest, CognitionResponse, Message, ToolCallHint};
use crate::receipt::RefinementSignal;
use crate::tool::{ToolCall, ToolId, ToolParams, ToolResult};

/// What the Actor proposes. Spec §The Loop: every external effect is a
/// Tool call; Halt signals the Task is complete.
///
/// `Fail` is the structural failure signal: the Actor's cognition could
/// not produce a valid Action (parse error, backend error, malformed
/// output). The LoopRunner emits a Receipt with `intercept_complete=false`
/// and Outcome::Escalated so the operator sees the failure in the
/// Receipt trail. CHECKLIST §Risk Register > Silent Failure: every
/// partial-coverage condition (cognitive failure prevents the Steward's
/// output from reaching the Gate) flips `intercept_complete=false`;
/// silent halt would leave operators unable to distinguish "task
/// complete" from "Actor failed." The LoopRunner sources the
/// identifiers (workspace_id, actor.id) when it forges the failure
/// Receipt — the Actor only contributes the reason.
#[derive(Debug, Clone)]
pub enum Action {
    Propose(ToolCall),
    Halt,
    Fail { reason: String },
}

/// What the Actor observes after a loop step. On rejection the Actor
/// receives the Refinement signal; on acceptance the proposal was
/// dispatched and the Tool's result is returned. Spec §The Loop.
#[derive(Debug, Clone)]
pub enum Observation {
    Accepted(ToolResult),
    Rejected(RefinementSignal),
}

/// The plant in the negative-feedback loop. Spec §Cognition Layer.
#[async_trait]
pub trait Actor: Send + Sync {
    fn id(&self) -> &str;
    async fn step(&mut self, observation: Option<Observation>) -> Action;
}

/// The canonical Actor implementation: maintains a per-turn
/// conversation history with the CognitionBackend, parses the LLM's
/// canonical response into an Action.
///
/// Two response shapes the Actor consumes from the backend:
///   1. `tool_call_hint` is set — the adapter has already extracted
///      `(tool, params)` from a structured tool-use wire format
///      (gpt-oss harmony, OpenAI native tool_calls, etc.). The Actor
///      builds a `ToolCall` from the hint without re-parsing `text`.
///   2. `tool_call_hint` is `None` — the Actor parses `text` as JSON:
///      `{"tool":"<id>", "params":{...}}` → `Action::Propose`,
///      `{"halt": true}` → `Action::Halt`. Anything else →
///      `Action::Fail` (operator-visible, recorded in trail with
///      `intercept_complete=false`, never silent).
pub struct LlmActor {
    id: String,
    backend: Arc<dyn CognitionBackend>,
    system_prompt: String,
    history: Vec<Message>,
    context_id: Arc<str>,
    source_id: Arc<str>,
}

impl LlmActor {
    pub fn new(
        id: impl Into<String>,
        backend: Arc<dyn CognitionBackend>,
        system_prompt: impl Into<String>,
        context_id: impl Into<Arc<str>>,
    ) -> Self {
        let id = id.into();
        let source_id: Arc<str> = Arc::from(id.as_str());
        Self {
            id,
            backend,
            system_prompt: system_prompt.into(),
            history: Vec::new(),
            context_id: context_id.into(),
            source_id,
        }
    }

    pub fn with_initial_user_message(mut self, msg: impl Into<String>) -> Self {
        self.history.push(Message::user(msg));
        self
    }

    fn append_observation(&mut self, observation: Observation) {
        let formatted = match observation {
            Observation::Accepted(Ok(v)) => {
                format!("[GATE: ALLOWED, dispatched]\nresult: {v}")
            }
            Observation::Accepted(Err(e)) => {
                format!("[GATE: ALLOWED, dispatch failed]\nerror: {e}")
            }
            Observation::Rejected(signal) => {
                let mut s = String::from("[GATE: DENIED, refine]\n");
                for (frame_id, reason) in &signal.entries {
                    s.push_str(&format!("- frame={frame_id}: {reason}\n"));
                }
                s
            }
        };
        self.history.push(Message::user(formatted));
    }

    fn fail(&self, reason: impl Into<String>) -> Action {
        Action::Fail {
            reason: reason.into(),
        }
    }

    fn action_from_hint(&self, hint: ToolCallHint) -> Action {
        Action::Propose(ToolCall {
            tool: ToolId::new(hint.tool),
            params: ToolParams(hint.params),
            context_id: self.context_id.clone(),
            source_id: self.source_id.clone(),
        })
    }

    fn action_from_text(&self, text: &str) -> Action {
        match parse_canonical_action(text) {
            ParsedAction::Propose { tool, params } => Action::Propose(ToolCall {
                tool: ToolId::new(tool),
                params: ToolParams(params),
                context_id: self.context_id.clone(),
                source_id: self.source_id.clone(),
            }),
            ParsedAction::Halt => Action::Halt,
            ParsedAction::Unparseable(reason) => self.fail(reason),
        }
    }
}

#[async_trait]
impl Actor for LlmActor {
    fn id(&self) -> &str {
        &self.id
    }

    async fn step(&mut self, observation: Option<Observation>) -> Action {
        if let Some(obs) = observation {
            self.append_observation(obs);
        }

        let mut messages = Vec::with_capacity(self.history.len() + 1);
        messages.push(Message::system(self.system_prompt.clone()));
        messages.extend(self.history.iter().cloned());

        let request = CognitionRequest {
            messages,
            max_output_tokens: Some(1024),
        };
        let response: CognitionResponse = match self.backend.complete(&request).await {
            Ok(r) => r,
            Err(e) => return self.fail(format!("backend error: {e}")),
        };

        self.history.push(Message::assistant(response.text.clone()));
        if let Some(hint) = response.tool_call_hint {
            return self.action_from_hint(hint);
        }
        self.action_from_text(&response.text)
    }
}

#[derive(Debug)]
enum ParsedAction {
    Propose {
        tool: String,
        params: serde_json::Value,
    },
    Halt,
    Unparseable(String),
}

/// Parse the canonical Actor response: a JSON object of shape
/// `{"tool": "...", "params": {...}}` or `{"halt": true}`.
/// Adapters strip vendor envelopes (markdown fences, harmony, etc.)
/// and populate `tool_call_hint` for tool-use formats; this parser
/// only sees canonical JSON text.
fn parse_canonical_action(text: &str) -> ParsedAction {
    let value: serde_json::Value = match serde_json::from_str(text.trim()) {
        Ok(v) => v,
        Err(e) => {
            return ParsedAction::Unparseable(format!(
                "response is not a JSON object: {e}"
            ));
        }
    };
    if value.get("halt").and_then(serde_json::Value::as_bool) == Some(true) {
        return ParsedAction::Halt;
    }
    let tool = match value.get("tool").and_then(serde_json::Value::as_str) {
        Some(t) => t.to_string(),
        None => {
            return ParsedAction::Unparseable(
                "response JSON has no `tool` field and no `halt: true`".into(),
            );
        }
    };
    let params = value
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    ParsedAction::Propose { tool, params }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_json_propose() {
        match parse_canonical_action(r#"{"tool":"write_file","params":{"path":"x"}}"#) {
            ParsedAction::Propose { tool, .. } => assert_eq!(tool, "write_file"),
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn parses_raw_json_halt() {
        match parse_canonical_action(r#"{"halt": true}"#) {
            ParsedAction::Halt => {}
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_garbage_is_reported() {
        match parse_canonical_action("just some prose with no json") {
            ParsedAction::Unparseable(_) => {}
            other => panic!("expected Unparseable, got {other:?}"),
        }
    }

    #[test]
    fn propose_with_no_params_field_yields_null_params() {
        match parse_canonical_action(r#"{"tool":"halt_check"}"#) {
            ParsedAction::Propose { tool, params } => {
                assert_eq!(tool, "halt_check");
                assert_eq!(params, serde_json::Value::Null);
            }
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn missing_tool_and_no_halt_is_unparseable() {
        match parse_canonical_action(r#"{"params":{"x":1}}"#) {
            ParsedAction::Unparseable(_) => {}
            other => panic!("expected Unparseable, got {other:?}"),
        }
    }
}
