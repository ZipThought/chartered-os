use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolId(pub String);

impl ToolId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for ToolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A specific Tool invocation — the only form of proposal in v1.
/// Spec §Vocabulary > Tool call.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    pub tool: ToolId,
    pub params: ToolParams,
    #[serde(serialize_with = "ser_arc_str")]
    pub context_id: Arc<str>,
    #[serde(serialize_with = "ser_arc_str")]
    pub source_id: Arc<str>,
}

fn ser_arc_str<S: Serializer>(v: &Arc<str>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(v)
}

/// Tool parameters, opaquely typed per Tool. JSON in v1.
#[derive(Debug, Clone, Serialize)]
pub struct ToolParams(pub serde_json::Value);

impl ToolParams {
    /// Read a required string field. Returns `Err("missing required
    /// field: {name}")` when absent or non-string. Every ToolExecutor
    /// that consumes a string param uses this — keeps error wording
    /// uniform across kernel and dispatch.
    pub fn require_str(&self, field: &str) -> Result<&str, String> {
        self.0
            .get(field)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing required field: {field}"))
    }

    /// Read an optional `Vec<String>` field. Returns an empty vec when
    /// the field is absent; returns `Err` when present but not an array
    /// of strings.
    pub fn optional_string_array(&self, field: &str) -> Result<Vec<String>, String> {
        let Some(v) = self.0.get(field) else {
            return Ok(Vec::new());
        };
        let Some(arr) = v.as_array() else {
            return Err(format!("field `{field}` must be an array of strings"));
        };
        let mut out = Vec::with_capacity(arr.len());
        for (i, item) in arr.iter().enumerate() {
            match item.as_str() {
                Some(s) => out.push(s.to_string()),
                None => return Err(format!("field `{field}`[{i}] must be a string")),
            }
        }
        Ok(out)
    }

    pub fn with_runtime_metadata(
        &self,
        receipt_id: &str,
        task_id: &str,
        attempt_id: Option<&str>,
        steward_id: &str,
        snapshot_id: &str,
    ) -> Self {
        let mut value = self.0.clone();
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "_receipt_id".into(),
                serde_json::Value::String(receipt_id.into()),
            );
            obj.insert(
                "_task_id".into(),
                serde_json::Value::String(task_id.into()),
            );
            if let Some(attempt_id) = attempt_id {
                obj.insert(
                    "_attempt_id".into(),
                    serde_json::Value::String(attempt_id.into()),
                );
            }
            obj.insert(
                "_steward_id".into(),
                serde_json::Value::String(steward_id.into()),
            );
            obj.insert(
                "_snapshot_id".into(),
                serde_json::Value::String(snapshot_id.into()),
            );
        }
        Self(value)
    }
}

/// What a Tool execution returns. `Ok(value)` carries the success
/// payload (an arbitrary JSON document); `Err(reason)` carries an
/// operator-readable failure string. Aliased to `Result<_, _>` so
/// executors can use `?` on `ToolParams::require_str` and similar.
/// Wire shape under serde is `{"Ok": value}` / `{"Err": reason}`.
pub type ToolResult = Result<serde_json::Value, String>;

/// The executor side of a Tool. Spec §Tools.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn id(&self) -> &ToolId;
    async fn execute(&self, params: &ToolParams) -> ToolResult;
}

/// Registry mapping Tool identifiers to their executors.
/// CHECKLIST §Tool Registry Is the Only Path.
#[derive(Default)]
pub struct ToolRegistry {
    executors: HashMap<ToolId, Arc<dyn ToolExecutor>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            executors: HashMap::new(),
        }
    }

    pub fn register(&mut self, executor: Arc<dyn ToolExecutor>) {
        let id = executor.id().clone();
        self.executors.insert(id, executor);
    }

    pub fn get(&self, id: &ToolId) -> Option<Arc<dyn ToolExecutor>> {
        self.executors.get(id).cloned()
    }

    pub fn contains(&self, id: &ToolId) -> bool {
        self.executors.contains_key(id)
    }
}
