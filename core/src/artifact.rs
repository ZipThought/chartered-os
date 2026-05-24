//! Artifact substrate.
//!
//! An **Artifact** is a kind-typed handle to an addressable entity exposing
//! a uniform operation set. Implementations bridge to substrate through
//! the `ArtifactBackend` trait. `ArtifactStore` registers Backends and
//! dispatches Tool calls to the Backend that owns each ArtifactId.
//!
//! See `docs/SPECIFICATION.md` §Vocabulary (Artifact) and §Tools.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::receipt::ReceiptId;
use crate::snapshot::SnapshotId;
use crate::steward::StewardId;
use crate::task::TaskId;
use crate::tool::{ToolExecutor, ToolId, ToolParams, ToolResult};

// ============================================================
// Identifiers
// ============================================================

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArtifactId(pub String);

impl ArtifactId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArtifactKindId(pub String);

impl ArtifactKindId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for ArtifactKindId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Reference kind: an in-memory or filesystem text artifact with byte-range
/// selectors and replace-range edits.
pub fn kind_text() -> ArtifactKindId {
    ArtifactKindId::new("text")
}

/// Reference kind: an append-only store of structured records. The
/// kernel injects runtime provenance (`_task_id`, `_steward_id`,
/// `_snapshot_id`, `_receipt_id`) into the Edit; the content fields
/// under `edit.append` are opaque to the kernel — Charters define
/// whatever shape suits the deployment.
pub fn kind_record_store() -> ArtifactKindId {
    ArtifactKindId::new("record-store")
}

// ============================================================
// Handle and structured types
// ============================================================

/// A kind-typed handle to an addressable entity. Carries no content; reads
/// and modifications dispatch through `ArtifactStore` to the owning Backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: ArtifactId,
    pub kind: ArtifactKindId,
}

/// Kind-discriminated Selector. Each Backend deserializes against its own
/// schema. For `kind=text`, contains an optional `range` field naming a
/// byte-coordinate substring. For `kind=record-store`, contains an
/// optional `filter` object whose entries are field=value equality
/// checks against the persisted record (any field; metadata or content).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selector(pub serde_json::Value);

impl Selector {
    pub fn empty() -> Self {
        Self(serde_json::Value::Null)
    }

    pub fn from_value(v: serde_json::Value) -> Self {
        Self(v)
    }
}

/// Kind-discriminated Edit. For `kind=text`, names a `range` and a
/// `replacement` substring. For `kind=record-store`, names an `append`
/// object whose fields are opaque content (the Charter defines the
/// shape); runtime provenance flows in via `_*`-prefixed metadata at
/// the Edit top level (injected by the kernel at `ModifyArtifact`
/// dispatch).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Edit(pub serde_json::Value);

impl Edit {
    pub fn from_value(v: serde_json::Value) -> Self {
        Self(v)
    }
}

/// Kind-shaped read result. The wrapping is uniform; the inner JSON is
/// kind-specific.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Projection(pub serde_json::Value);

/// Text-coordinate range. Used by `kind=text` Backends and by the Selection
/// trigger machinery (which addresses substring selections in user
/// documents).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRange {
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub end_line: usize,
}

/// A record persisted by a `kind=record-store` Backend. Carries the
/// kernel-injected runtime provenance plus the Steward-supplied content
/// (opaque to the kernel — the Charter defines the shape). Serializes
/// flat: provenance fields plus every content field at top level.
#[derive(Debug, Clone, Serialize)]
pub struct Record {
    pub id: String,
    pub task_id: TaskId,
    pub steward_id: StewardId,
    pub snapshot_id: SnapshotId,
    pub receipt_id: ReceiptId,
    /// Opaque content fields. Serialized into the persisted record at
    /// top level (flattened) so query filters can match either
    /// provenance or content uniformly.
    #[serde(flatten)]
    pub content: serde_json::Map<String, serde_json::Value>,
}

// ============================================================
// Backend trait
// ============================================================

/// Per-substrate implementation of one ArtifactKind. The kernel maintains
/// **one Backend per kind** in the `ArtifactStore`; dispatch is by kind,
/// not by guessing ownership of an artifact_id.
///
/// `read` / `modify` validate the artifact_id within the Backend's
/// namespace and return specific errors (escapes-workspace-root,
/// artifact-not-found, malformed-selector). There is no cross-Backend
/// fall-through and no shape-based ownership heuristic; an unknown kind
/// is a specific kernel-level error before any Backend runs.
///
/// Additional operations (`query`, `subscribe`, `cite`, `attest`) will
/// land as the OS Contract Plan moves them into the trait.
#[async_trait]
pub trait ArtifactBackend: Send + Sync {
    fn kind(&self) -> &ArtifactKindId;
    fn list(&self) -> Vec<Artifact>;
    async fn read(
        &self,
        artifact_id: &ArtifactId,
        selector: &Selector,
    ) -> Result<Projection, String>;
    async fn modify(&self, artifact_id: &ArtifactId, edit: &Edit) -> Result<Projection, String>;
}

// ============================================================
// ArtifactStore — registry of Backends, kind-dispatching
// ============================================================

/// Registers `ArtifactBackend` instances keyed by `ArtifactKindId` and
/// dispatches Tool calls to the Backend serving the requested kind.
/// **One Backend per kind**; second registration of the same kind
/// returns `BackendConflict`. No fall-through: an unknown kind is a
/// specific error before any Backend runs.
#[derive(Default)]
pub struct ArtifactStore {
    backends: std::collections::BTreeMap<ArtifactKindId, std::sync::Arc<dyn ArtifactBackend>>,
}

#[derive(Debug, Clone)]
pub struct BackendConflict {
    pub kind: ArtifactKindId,
}

impl std::fmt::Display for BackendConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ArtifactBackend already registered for kind `{}`",
            self.kind
        )
    }
}

impl std::error::Error for BackendConflict {}

impl ArtifactStore {
    pub fn new() -> Self {
        Self {
            backends: std::collections::BTreeMap::new(),
        }
    }

    /// Builder-style registration; panics on conflict so misconfiguration
    /// surfaces at startup. Use `register` for fallible registration.
    pub fn with_backend(mut self, backend: std::sync::Arc<dyn ArtifactBackend>) -> Self {
        self.register(backend)
            .expect("duplicate ArtifactBackend kind");
        self
    }

    /// Register a Backend. Returns `BackendConflict` if a Backend for
    /// this kind is already registered (one Backend per kind invariant).
    pub fn register(
        &mut self,
        backend: std::sync::Arc<dyn ArtifactBackend>,
    ) -> Result<(), BackendConflict> {
        let kind = backend.kind().clone();
        if self.backends.contains_key(&kind) {
            return Err(BackendConflict { kind });
        }
        self.backends.insert(kind, backend);
        Ok(())
    }

    pub fn list(&self) -> Vec<Artifact> {
        self.backends.values().flat_map(|b| b.list()).collect()
    }

    pub fn registered_kinds(&self) -> Vec<ArtifactKindId> {
        self.backends.keys().cloned().collect()
    }

    pub async fn read(
        &self,
        kind: &ArtifactKindId,
        artifact_id: &ArtifactId,
        selector: &Selector,
    ) -> Result<Projection, String> {
        let backend = self.backend_of_kind(kind)?;
        backend.read(artifact_id, selector).await
    }

    pub async fn modify(
        &self,
        kind: &ArtifactKindId,
        artifact_id: &ArtifactId,
        edit: &Edit,
    ) -> Result<Projection, String> {
        let backend = self.backend_of_kind(kind)?;
        backend.modify(artifact_id, edit).await
    }

    fn backend_of_kind(
        &self,
        kind: &ArtifactKindId,
    ) -> Result<std::sync::Arc<dyn ArtifactBackend>, String> {
        self.backends
            .get(kind)
            .cloned()
            .ok_or_else(|| format!("ArtifactKind `{kind}` is not registered"))
    }
}

// ============================================================
// Reference Backends (in-memory)
// ============================================================

/// `kind=text` Backend backed by an in-memory `BTreeMap`. Used by kernel
/// tests and by the dashboard's in-process workspace mode.
pub struct InMemoryTextBackend {
    kind: ArtifactKindId,
    artifacts: Mutex<BTreeMap<ArtifactId, String>>,
}

impl InMemoryTextBackend {
    pub fn new<I>(seed: I) -> Self
    where
        I: IntoIterator<Item = (ArtifactId, String)>,
    {
        let mut map = BTreeMap::new();
        for (id, content) in seed {
            map.insert(id, content);
        }
        Self {
            kind: kind_text(),
            artifacts: Mutex::new(map),
        }
    }

    pub fn entries(&self) -> Vec<(ArtifactId, String)> {
        self.artifacts
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn content(&self, artifact_id: &ArtifactId) -> Option<String> {
        self.artifacts.lock().unwrap().get(artifact_id).cloned()
    }
}

#[async_trait]
impl ArtifactBackend for InMemoryTextBackend {
    fn kind(&self) -> &ArtifactKindId {
        &self.kind
    }

    fn list(&self) -> Vec<Artifact> {
        let kind = self.kind.clone();
        self.artifacts
            .lock()
            .unwrap()
            .keys()
            .map(|id| Artifact {
                id: id.clone(),
                kind: kind.clone(),
            })
            .collect()
    }

    async fn read(
        &self,
        artifact_id: &ArtifactId,
        selector: &Selector,
    ) -> Result<Projection, String> {
        let artifacts = self.artifacts.lock().unwrap();
        let content = artifacts
            .get(artifact_id)
            .ok_or_else(|| format!("artifact `{artifact_id}` not found"))?;
        let range = parse_text_range_from_selector(selector)?;
        let body = match range {
            Some(r) => slice_range(content, r)?.to_string(),
            None => content.clone(),
        };
        Ok(Projection(serde_json::json!({
            "artifact_id": artifact_id,
            "content": body,
            "range": range,
        })))
    }

    async fn modify(&self, artifact_id: &ArtifactId, edit: &Edit) -> Result<Projection, String> {
        let mut artifacts = self.artifacts.lock().unwrap();
        let content = artifacts
            .get_mut(artifact_id)
            .ok_or_else(|| format!("artifact `{artifact_id}` not found"))?;
        let (range, replacement) = parse_text_edit(edit)?;
        *content = apply_text_edit(content, range, &replacement)?;
        Ok(Projection(serde_json::json!({
            "artifact_id": artifact_id,
            "range": range,
            "applied_text": replacement,
        })))
    }
}

/// `kind=record-store` Backend backed by an in-memory `Vec<Record>`.
/// Exposes one artifact whose id the constructor takes; `read` returns
/// the filtered list, `modify` appends one Record.
pub struct InMemoryRecordStore {
    kind: ArtifactKindId,
    artifact_id: ArtifactId,
    records: Mutex<Vec<Record>>,
    counter: AtomicU64,
}

impl InMemoryRecordStore {
    pub fn new(artifact_id: ArtifactId) -> Self {
        Self {
            kind: kind_record_store(),
            artifact_id,
            records: Mutex::new(Vec::new()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn records(&self) -> Vec<Record> {
        self.records.lock().unwrap().clone()
    }
}

#[async_trait]
impl ArtifactBackend for InMemoryRecordStore {
    fn kind(&self) -> &ArtifactKindId {
        &self.kind
    }

    fn list(&self) -> Vec<Artifact> {
        vec![Artifact {
            id: self.artifact_id.clone(),
            kind: self.kind.clone(),
        }]
    }

    async fn read(
        &self,
        artifact_id: &ArtifactId,
        selector: &Selector,
    ) -> Result<Projection, String> {
        if artifact_id != &self.artifact_id {
            return Err(format!(
                "record-store has no artifact `{artifact_id}` (only `{}` exists)",
                self.artifact_id
            ));
        }
        let filter = parse_record_filter(selector)?;
        let records = self.records.lock().unwrap();
        let filtered: Vec<&Record> = records
            .iter()
            .filter(|r| {
                let v = serde_json::to_value(r).unwrap_or(serde_json::Value::Null);
                filter.matches_value(&v)
            })
            .collect();
        Ok(Projection(serde_json::json!({
            "records": filtered,
        })))
    }

    async fn modify(&self, artifact_id: &ArtifactId, edit: &Edit) -> Result<Projection, String> {
        if artifact_id != &self.artifact_id {
            return Err(format!(
                "record-store has no artifact `{artifact_id}` (only `{}` exists)",
                self.artifact_id
            ));
        }
        let append = parse_record_append(edit)?;
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let record = Record {
            id: format!("record-{n:03}"),
            task_id: append.task_id,
            steward_id: append.steward_id,
            snapshot_id: append.snapshot_id,
            receipt_id: append.receipt_id,
            content: append.content,
        };
        let id = record.id.clone();
        self.records.lock().unwrap().push(record);
        Ok(Projection(serde_json::json!({ "record_id": id })))
    }
}

// ============================================================
// In-memory façade: InMemoryArtifactStore
// ============================================================

/// Façade that bundles an in-memory text Backend + an in-memory
/// record-store Backend behind one ArtifactStore. Used by kernel tests
/// and by the dashboard's in-process mode.
pub struct InMemoryArtifactStore {
    pub store: std::sync::Arc<ArtifactStore>,
    text: std::sync::Arc<InMemoryTextBackend>,
    records: std::sync::Arc<InMemoryRecordStore>,
}

impl InMemoryArtifactStore {
    pub fn new<I>(artifacts: I) -> Self
    where
        I: IntoIterator<Item = TextArtifactSeed>,
    {
        let entries = artifacts.into_iter().map(|a| (a.id, a.content));
        let text = std::sync::Arc::new(InMemoryTextBackend::new(entries));
        let records =
            std::sync::Arc::new(InMemoryRecordStore::new(ArtifactId::new("records")));
        // One Backend per kind; the ArtifactStore enforces this on
        // registration. Order is not load-bearing.
        let store = std::sync::Arc::new(
            ArtifactStore::new()
                .with_backend(records.clone())
                .with_backend(text.clone()),
        );
        Self {
            store,
            text,
            records,
        }
    }

    /// Accessor returning text artifacts as `TextArtifactSeed { id, content }`.
    pub fn artifacts(&self) -> Vec<TextArtifactSeed> {
        self.text
            .entries()
            .into_iter()
            .map(|(id, content)| TextArtifactSeed { id, content })
            .collect()
    }

    /// Accessor returning the records collection.
    pub fn records(&self) -> Vec<Record> {
        self.records.records()
    }

    pub fn artifact_store(&self) -> std::sync::Arc<ArtifactStore> {
        self.store.clone()
    }
}

/// Test-seed shape: a `(id, content)` pair preserved for tests and the in-memory
/// seeding API. New code uses `Artifact` (the kind-typed handle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextArtifactSeed {
    pub id: ArtifactId,
    pub content: String,
}

// ============================================================
// Helpers — Selector and Edit parsing per kind
// ============================================================

/// Parse the `range` field from a Selector's top-level object, if any.
/// Returns `None` if the Selector is null or carries no `range` key.
pub fn parse_text_range_from_selector(
    selector: &Selector,
) -> Result<Option<ArtifactRange>, String> {
    if selector.0.is_null() {
        return Ok(None);
    }
    let range_value = match &selector.0 {
        serde_json::Value::Object(map) => map.get("range"),
        _ => None,
    };
    let Some(range_value) = range_value else {
        return Ok(None);
    };
    parse_artifact_range(range_value).map(Some)
}

/// Parse a text-shaped Edit into `(range, replacement)`.
pub fn parse_text_edit(edit: &Edit) -> Result<(ArtifactRange, String), String> {
    let obj = edit
        .0
        .as_object()
        .ok_or_else(|| "text edit must be a JSON object".to_string())?;
    let range_value = obj
        .get("range")
        .ok_or_else(|| "text edit missing required field: range".to_string())?;
    let range = parse_artifact_range(range_value)?;
    let replacement = obj
        .get("replacement")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "text edit missing required field: replacement".to_string())?
        .to_string();
    Ok((range, replacement))
}

pub fn parse_artifact_range(value: &serde_json::Value) -> Result<ArtifactRange, String> {
    let get = |name: &str| -> Result<usize, String> {
        value
            .get(name)
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| format!("range missing required field: {name}"))
    };
    Ok(ArtifactRange {
        start: get("start")?,
        end: get("end")?,
        start_line: get("start_line")?,
        end_line: get("end_line")?,
    })
}

pub fn validate_range(content: &str, range: ArtifactRange) -> Result<(), String> {
    if range.start > range.end || range.end > content.len() {
        return Err("artifact range is outside content bounds".into());
    }
    if !content.is_char_boundary(range.start) || !content.is_char_boundary(range.end) {
        return Err("artifact range does not align with UTF-8 boundaries".into());
    }
    Ok(())
}

pub fn slice_range(content: &str, range: ArtifactRange) -> Result<&str, String> {
    validate_range(content, range)?;
    Ok(&content[range.start..range.end])
}

/// Splice `replacement` into `content` at `range`. Validates the range
/// (bounds + UTF-8 boundaries) and returns the resulting string.
/// Single source of truth for text-edit splicing — both
/// `InMemoryTextBackend` and `FilesystemTextBackend` route through it.
pub fn apply_text_edit(
    content: &str,
    range: ArtifactRange,
    replacement: &str,
) -> Result<String, String> {
    validate_range(content, range)?;
    let mut next =
        String::with_capacity(content.len() - (range.end - range.start) + replacement.len());
    next.push_str(&content[..range.start]);
    next.push_str(replacement);
    next.push_str(&content[range.end..]);
    Ok(next)
}

/// Selector shape for `kind=record-store` artifacts. Generic
/// field-equality filter: every string-valued entry in `selector.filter`
/// matches a top-level field of the persisted record (metadata or
/// content). Records that don't carry a queried field, or carry a
/// different value, fall out of the projection.
#[derive(Debug, Default)]
pub struct RecordFilter {
    pub fields: std::collections::BTreeMap<String, String>,
}

impl RecordFilter {
    /// Match against any JSON value with top-level fields — both the
    /// in-memory `Record` (after serializing) and disk-backed JSONL
    /// lines parsed back into a `serde_json::Value` use this path.
    pub fn matches_value(&self, v: &serde_json::Value) -> bool {
        for (name, expected) in &self.fields {
            let actual = v.get(name).and_then(|x| x.as_str());
            if actual != Some(expected.as_str()) {
                return false;
            }
        }
        true
    }
}

pub fn parse_record_filter(selector: &Selector) -> Result<RecordFilter, String> {
    if selector.0.is_null() {
        return Ok(RecordFilter::default());
    }
    let obj = selector
        .0
        .as_object()
        .ok_or_else(|| "record selector must be a JSON object".to_string())?;
    let filter = obj.get("filter").and_then(|v| v.as_object());
    let mut fields = std::collections::BTreeMap::new();
    if let Some(filter) = filter {
        for (k, val) in filter {
            if let Some(s) = val.as_str() {
                fields.insert(k.clone(), s.to_string());
            }
        }
    }
    Ok(RecordFilter { fields })
}

/// Append shape for `kind=record-store`. Runtime provenance comes from
/// `_*` fields the kernel injected into the Edit at `ModifyArtifact`
/// dispatch; the Steward-supplied content lives under `edit.append` and
/// is opaque to the kernel — every field there flows through into the
/// persisted record as-is, with the kernel adding `id`, `task_id`,
/// `steward_id`, `snapshot_id`, `receipt_id` at the top level.
pub struct RecordAppend {
    pub task_id: TaskId,
    pub steward_id: StewardId,
    pub snapshot_id: SnapshotId,
    pub receipt_id: ReceiptId,
    pub content: serde_json::Map<String, serde_json::Value>,
}

/// Read a kernel-injected `_*` metadata field at the top of the Edit
/// object. The runtime injects `_receipt_id`, `_task_id`,
/// `_attempt_id`, `_steward_id`, `_snapshot_id` into the Edit at the
/// `ModifyArtifact` dispatch boundary (`edit_from_params`); Backends
/// that need provenance read them from here. Steward-supplied payload
/// lives under the kind-specific key (e.g. `append`); the kernel
/// metadata namespace (`_`-prefix) does not collide with it.
fn runtime_metadata_field<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<&'a str, String> {
    obj.get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("record edit missing runtime metadata field: {name}"))
}

pub fn parse_record_append(edit: &Edit) -> Result<RecordAppend, String> {
    let obj = edit
        .0
        .as_object()
        .ok_or_else(|| "record edit must be a JSON object".to_string())?;
    let append = obj
        .get("append")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "record edit missing required field: append".to_string())?;
    Ok(RecordAppend {
        task_id: TaskId::new(runtime_metadata_field(obj, "_task_id")?),
        steward_id: StewardId::new(runtime_metadata_field(obj, "_steward_id")?),
        snapshot_id: SnapshotId(runtime_metadata_field(obj, "_snapshot_id")?.to_string()),
        receipt_id: ReceiptId(runtime_metadata_field(obj, "_receipt_id")?.to_string()),
        content: append.clone(),
    })
}

// ============================================================
// Tool ABI — substrate-blind, dispatches through ArtifactStore
// ============================================================

/// Extract the `kind` parameter; absence is a specific error, not a default.
fn kind_from_params(params: &ToolParams) -> Result<ArtifactKindId, String> {
    Ok(ArtifactKindId::new(params.require_str("kind")?))
}

/// Selector ABI: the `selector` field carries any JSON Value (kind-shaped).
/// For convenience, a top-level `range` field on the Tool params (without
/// `selector`) is promoted to `{ "range": <value> }` as a Selector — this
/// is the standard text-kind shape used by Selection triggers, not a
/// fall-back across kinds.
fn selector_from_params(kind: &ArtifactKindId, params: &ToolParams) -> Selector {
    if let Some(sel) = params.0.get("selector") {
        return Selector::from_value(sel.clone());
    }
    if kind == &kind_text()
        && let Some(range) = params.0.get("range")
    {
        return Selector::from_value(serde_json::json!({ "range": range }));
    }
    Selector::empty()
}

/// Edit ABI: the `edit` field carries any JSON Value (kind-shaped). For
/// convenience, top-level `range` + `replacement` fields on the Tool
/// params (without `edit`) are promoted to a text Edit — only when
/// `kind=text`. Runtime-injected `_*` metadata at the top of params is
/// propagated into the Edit at the same level, so Backends that need
/// provenance (record-store and any future tamper-evident kind) read
/// it from the Edit they are handed. Backends that don't care ignore it.
fn edit_from_params(kind: &ArtifactKindId, params: &ToolParams) -> Result<Edit, String> {
    let mut value = if let Some(edit) = params.0.get("edit") {
        edit.clone()
    } else if kind == &kind_text() {
        let range = params
            .0
            .get("range")
            .ok_or_else(|| "text edit missing required field: range".to_string())?
            .clone();
        let replacement = params
            .0
            .get("replacement")
            .ok_or_else(|| "text edit missing required field: replacement".to_string())?
            .clone();
        serde_json::json!({ "range": range, "replacement": replacement })
    } else {
        return Err(format!(
            "modify_artifact for kind `{kind}` requires an `edit` field"
        ));
    };

    if let (Some(edit_obj), Some(params_obj)) = (value.as_object_mut(), params.0.as_object()) {
        for key in [
            "_receipt_id",
            "_task_id",
            "_attempt_id",
            "_steward_id",
            "_snapshot_id",
        ] {
            if let Some(v) = params_obj.get(key) {
                edit_obj.entry(key.to_string()).or_insert_with(|| v.clone());
            }
        }
    }

    Ok(Edit::from_value(value))
}

pub struct ReadArtifact {
    id: ToolId,
    store: std::sync::Arc<ArtifactStore>,
}

impl ReadArtifact {
    pub fn new(id: ToolId, store: std::sync::Arc<ArtifactStore>) -> Self {
        Self { id, store }
    }
}

#[async_trait]
impl ToolExecutor for ReadArtifact {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        let kind = kind_from_params(params)?;
        let artifact_id = ArtifactId::new(params.require_str("artifact_id")?);
        let selector = selector_from_params(&kind, params);
        let projection = self.store.read(&kind, &artifact_id, &selector).await?;
        Ok(projection.0)
    }
}

pub struct ModifyArtifact {
    id: ToolId,
    store: std::sync::Arc<ArtifactStore>,
}

impl ModifyArtifact {
    pub fn new(id: ToolId, store: std::sync::Arc<ArtifactStore>) -> Self {
        Self { id, store }
    }
}

#[async_trait]
impl ToolExecutor for ModifyArtifact {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        let kind = kind_from_params(params)?;
        let artifact_id = ArtifactId::new(params.require_str("artifact_id")?);
        let edit = edit_from_params(&kind, params)?;
        let projection = self.store.modify(&kind, &artifact_id, &edit).await?;
        Ok(projection.0)
    }
}

pub struct ListArtifacts {
    id: ToolId,
    store: std::sync::Arc<ArtifactStore>,
}

impl ListArtifacts {
    pub fn new(id: ToolId, store: std::sync::Arc<ArtifactStore>) -> Self {
        Self { id, store }
    }
}

#[async_trait]
impl ToolExecutor for ListArtifacts {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, _params: &ToolParams) -> ToolResult {
        let listed: Vec<_> = self
            .store
            .list()
            .into_iter()
            .map(|a| serde_json::json!({ "artifact_id": a.id, "kind": a.kind }))
            .collect();
        Ok(serde_json::json!({ "artifacts": listed }))
    }
}

