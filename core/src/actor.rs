use std::sync::Arc;

use async_trait::async_trait;

use crate::cognition::{ActionHint, CognitionBackend, CognitionRequest, CognitionResponse, Message};
use crate::receipt::RefinementSignal;
use crate::tool::{ToolCall, ToolId, ToolParams, ToolResult};

/// What the Actor proposes. Spec §The Loop: every external effect is a
/// Tool call; Halt signals the Task is complete.
///
/// `Fail` is the structural failure signal: the Actor's cognition could
/// not produce a valid Action (the adapter never delivered an action_hint
/// within the inner step budget). The LoopRunner emits a Receipt with
/// `intercept_complete=false` and Outcome::Escalated so the operator
/// sees the failure in the Receipt trail. CHECKLIST §Risk Register >
/// Silent Failure: every partial-coverage condition (cognitive failure
/// prevents the Steward's output from reaching the Gate) flips
/// `intercept_complete=false`; silent halt would leave operators unable
/// to distinguish "task complete" from "Actor failed." The LoopRunner
/// sources the identifiers (workspace_id, actor.id) when it forges the
/// failure Receipt — the Actor only contributes the reason.
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

/// The canonical Actor implementation. Maintains a per-turn
/// conversation history with the CognitionBackend; commits on the
/// strong-typed `action_hint` the adapter produces. The kernel does
/// not parse JSON — the adapter converts whatever wire shape the
/// vendor used (harmony envelope, plain prose with embedded JSON,
/// OpenAI native `tool_calls`, Gemini `functionCall`, etc.) into an
/// `ActionHint`. When the response has no `action_hint`, the Actor
/// treats it as a reasoning step and continues the inner loop until
/// either an action commits or the step budget exhausts.
pub struct LlmActor {
    id: String,
    backend: Arc<dyn CognitionBackend>,
    system_prompt: String,
    history: Vec<Message>,
    context_id: Arc<str>,
    source_id: Arc<str>,
    inner_step_budget: usize,
}

/// Default ceiling on LLM calls inside one `Actor::step` invocation.
/// Each pure-reasoning response burns one step; structured (tool-call
/// or halt) responses commit immediately. Exhaustion → `Action::Fail`
/// with operator-visible diagnostic.
pub const DEFAULT_INNER_STEP_BUDGET: usize = 8;

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
            inner_step_budget: DEFAULT_INNER_STEP_BUDGET,
        }
    }

    pub fn with_initial_user_message(mut self, msg: impl Into<String>) -> Self {
        self.history.push(Message::user(msg));
        self
    }

    /// Cap the LLM calls inside one outer step. The Actor's
    /// agentic inner loop walks up to `budget` LLM exchanges before
    /// returning `Action::Fail`. Spec §The Loop and §Cognition Layer:
    /// every externally-observable tool call still crosses the Gate;
    /// the inner loop is bounded planning, not bounded effect.
    pub fn with_inner_step_budget(mut self, budget: usize) -> Self {
        self.inner_step_budget = budget;
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

    fn action_from_hint(&self, hint: ActionHint) -> Action {
        match hint {
            ActionHint::Halt => Action::Halt,
            ActionHint::Propose { tool, params } => Action::Propose(ToolCall {
                tool: ToolId::new(tool),
                params: ToolParams(params),
                context_id: self.context_id.clone(),
                source_id: self.source_id.clone(),
            }),
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

        // Agentic inner loop. Each iteration either commits (action_hint
        // present → returned to the outer LoopRunner, which crosses the
        // Gate) or treats the response as reasoning and continues.
        // Bounded by `inner_step_budget` so a non-committing model can't
        // spin indefinitely.
        for _ in 0..self.inner_step_budget {
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

            self.history.push(Message::assistant(response.content.clone()));
            if let Some(hint) = response.action_hint {
                return self.action_from_hint(hint);
            }
            // No action_hint → adapter classified the response as pure
            // reasoning. Keep it in history (already pushed above) and
            // continue the inner loop.
        }
        self.fail(format!(
            "inner step budget exhausted ({} steps) without the adapter producing an action_hint",
            self.inner_step_budget
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cognition::FakeCognitionBackend;

    fn propose(tool: &str, params: serde_json::Value) -> ActionHint {
        ActionHint::Propose {
            tool: tool.to_string(),
            params,
        }
    }

    #[tokio::test]
    async fn commits_on_first_action_hint() {
        let backend = Arc::new(FakeCognitionBackend::new("a1"));
        backend.enqueue_action(propose(
            "write_file",
            serde_json::json!({"path": "out.md"}),
        ));
        let mut actor = LlmActor::new("a", backend, "sys", "ctx");

        match actor.step(None).await {
            Action::Propose(tc) => {
                assert_eq!(tc.tool.0, "write_file");
                assert_eq!(tc.params.0["path"].as_str(), Some("out.md"));
            }
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn returns_halt_on_halt_action_hint() {
        let backend = Arc::new(FakeCognitionBackend::new("a2"));
        backend.enqueue_action(ActionHint::Halt);
        let mut actor = LlmActor::new("a", backend, "sys", "ctx");
        match actor.step(None).await {
            Action::Halt => {}
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reasoning_responses_continue_inner_loop_until_commit() {
        let backend = Arc::new(FakeCognitionBackend::new("a3"));
        backend
            .enqueue("First I reason about the request.")
            .enqueue("Now I plan: I'll write the file.")
            .enqueue_action(propose(
                "write_file",
                serde_json::json!({"path": "out.md"}),
            ));
        let mut actor = LlmActor::new("a", backend, "sys", "ctx");

        match actor.step(None).await {
            Action::Propose(tc) => {
                assert_eq!(tc.tool.0, "write_file");
                assert_eq!(tc.params.0["path"].as_str(), Some("out.md"));
            }
            other => panic!("expected Propose after reasoning, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn budget_exhaustion_fails_with_diagnostic() {
        let backend = Arc::new(FakeCognitionBackend::new("a4"));
        for _ in 0..DEFAULT_INNER_STEP_BUDGET {
            backend.enqueue("more reasoning, no commit");
        }
        let mut actor = LlmActor::new("a", backend, "sys", "ctx");

        match actor.step(None).await {
            Action::Fail { reason } => {
                assert!(
                    reason.contains("inner step budget exhausted"),
                    "reason: {reason}"
                );
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn custom_budget_is_respected() {
        let backend = Arc::new(FakeCognitionBackend::new("a5"));
        backend.enqueue("just reasoning"); // one response only
        let mut actor =
            LlmActor::new("a", backend, "sys", "ctx").with_inner_step_budget(1);
        match actor.step(None).await {
            Action::Fail { reason } => {
                assert!(reason.contains("inner step budget exhausted (1 steps)"));
            }
            other => panic!("expected Fail with budget=1, got {other:?}"),
        }
    }
}
