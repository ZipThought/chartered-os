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

/// Reference kind: an append-only collection of `Finding` records, queried
/// through `ArtifactBackend::read` and appended through
/// `ArtifactBackend::modify`.
pub fn kind_findings_store() -> ArtifactKindId {
    ArtifactKindId::new("findings-store")
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
/// byte-coordinate substring. For `kind=findings-store`, contains an
/// optional `filter` field naming a query (severity, frame, steward, etc.).
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
/// `replacement` substring. For `kind=findings-store`, names an `append`
/// of one Finding record.
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

/// A structured record stored in a `kind=findings-store` artifact.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub id: String,
    pub task_id: TaskId,
    pub author_steward_id: StewardId,
    pub snapshot_id: SnapshotId,
    pub artifact_id: ArtifactId,
    pub range: ArtifactRange,
    pub concern: String,
    pub severity: String,
    pub detail: String,
    pub admitting_receipt_id: ReceiptId,
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

/// `kind=findings-store` Backend backed by an in-memory `Vec<Finding>`.
/// Exposes one artifact (typically `findings`); `read` returns the filtered
/// list, `modify` appends one Finding.
pub struct InMemoryFindingsBackend {
    kind: ArtifactKindId,
    artifact_id: ArtifactId,
    findings: Mutex<Vec<Finding>>,
    counter: AtomicU64,
}

impl InMemoryFindingsBackend {
    pub fn new(artifact_id: ArtifactId) -> Self {
        Self {
            kind: kind_findings_store(),
            artifact_id,
            findings: Mutex::new(Vec::new()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn findings(&self) -> Vec<Finding> {
        self.findings.lock().unwrap().clone()
    }
}

#[async_trait]
impl ArtifactBackend for InMemoryFindingsBackend {
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
                "findings-store has no artifact `{artifact_id}` (only `{}` exists)",
                self.artifact_id
            ));
        }
        let filter = parse_findings_filter(selector)?;
        let findings = self.findings.lock().unwrap();
        let filtered: Vec<&Finding> = findings.iter().filter(|f| filter.matches(f)).collect();
        Ok(Projection(serde_json::json!({
            "findings": filtered,
        })))
    }

    async fn modify(&self, artifact_id: &ArtifactId, edit: &Edit) -> Result<Projection, String> {
        if artifact_id != &self.artifact_id {
            return Err(format!(
                "findings-store has no artifact `{artifact_id}` (only `{}` exists)",
                self.artifact_id
            ));
        }
        let append = parse_findings_append(edit)?;
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let finding = Finding {
            id: format!("F-{n:03}"),
            task_id: append.task_id,
            author_steward_id: append.author_steward_id,
            snapshot_id: append.snapshot_id,
            artifact_id: append.artifact_id,
            range: append.range,
            concern: append.concern,
            severity: append.severity,
            detail: append.detail,
            admitting_receipt_id: append.receipt_id,
        };
        let id = finding.id.clone();
        self.findings.lock().unwrap().push(finding);
        Ok(Projection(serde_json::json!({ "finding_id": id })))
    }
}

// ============================================================
// Backward-compat façade: InMemoryArtifactStore
// ============================================================

/// Façade that bundles an in-memory text Backend + an in-memory
/// findings-store Backend behind one ArtifactStore. Preserves the legacy
/// API for tests and the dashboard's in-process mode while the kernel
/// dispatches through the OS-contract substrate beneath.
pub struct InMemoryArtifactStore {
    pub store: std::sync::Arc<ArtifactStore>,
    text: std::sync::Arc<InMemoryTextBackend>,
    findings: std::sync::Arc<InMemoryFindingsBackend>,
}

impl InMemoryArtifactStore {
    pub fn new<I>(artifacts: I) -> Self
    where
        I: IntoIterator<Item = LegacyArtifact>,
    {
        let entries = artifacts.into_iter().map(|a| (a.id, a.content));
        let text = std::sync::Arc::new(InMemoryTextBackend::new(entries));
        let findings =
            std::sync::Arc::new(InMemoryFindingsBackend::new(ArtifactId::new("findings")));
        // One Backend per kind; the ArtifactStore enforces this on
        // registration. Order is not load-bearing.
        let store = std::sync::Arc::new(
            ArtifactStore::new()
                .with_backend(findings.clone())
                .with_backend(text.clone()),
        );
        Self {
            store,
            text,
            findings,
        }
    }

    /// Legacy accessor returning text artifacts as `LegacyArtifact { id, content }`.
    pub fn artifacts(&self) -> Vec<LegacyArtifact> {
        self.text
            .entries()
            .into_iter()
            .map(|(id, content)| LegacyArtifact { id, content })
            .collect()
    }

    /// Legacy accessor returning the findings collection.
    pub fn findings(&self) -> Vec<Finding> {
        self.findings.findings()
    }

    pub fn artifact_store(&self) -> std::sync::Arc<ArtifactStore> {
        self.store.clone()
    }
}

/// Legacy text-only Artifact shape preserved for tests and the in-memory
/// seeding API. New code uses `Artifact` (the kind-typed handle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyArtifact {
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

/// Selector shape for `kind=findings-store` artifacts. Single source of
/// truth — both `InMemoryFindingsBackend` (kernel typed records) and
/// `FilesystemFindingsBackend` (JSONL records on disk) parse through
/// `parse_findings_filter` and match through this struct.
#[derive(Debug, Default)]
pub struct FindingsFilter {
    pub severity: Option<String>,
    pub artifact_id: Option<String>,
    pub task_id: Option<String>,
    pub author_steward_id: Option<String>,
}

impl FindingsFilter {
    pub fn matches(&self, f: &Finding) -> bool {
        if let Some(s) = &self.severity
            && &f.severity != s
        {
            return false;
        }
        if let Some(a) = &self.artifact_id
            && &f.artifact_id.0 != a
        {
            return false;
        }
        if let Some(task_id) = &self.task_id
            && &f.task_id.0 != task_id
        {
            return false;
        }
        if let Some(s) = &self.author_steward_id
            && &f.author_steward_id.0 != s
        {
            return false;
        }
        true
    }

    /// Match against a JSON record (used by the disk-backed Findings
    /// Backend, where each line is a `serde_json::Value` parsed lazily
    /// from JSONL). Mirrors `matches(&Finding)` field-for-field on the
    /// same kernel-defined Selector schema.
    pub fn matches_value(&self, v: &serde_json::Value) -> bool {
        let str_field = |name: &str| v.get(name).and_then(|x| x.as_str());
        if let Some(s) = &self.severity
            && str_field("severity") != Some(s)
        {
            return false;
        }
        if let Some(a) = &self.artifact_id
            && str_field("artifact_id") != Some(a)
        {
            return false;
        }
        if let Some(task_id) = &self.task_id
            && str_field("task_id") != Some(task_id)
        {
            return false;
        }
        if let Some(s) = &self.author_steward_id
            && str_field("author_steward_id") != Some(s)
        {
            return false;
        }
        true
    }
}

pub fn parse_findings_filter(selector: &Selector) -> Result<FindingsFilter, String> {
    if selector.0.is_null() {
        return Ok(FindingsFilter::default());
    }
    let obj = selector
        .0
        .as_object()
        .ok_or_else(|| "findings selector must be a JSON object".to_string())?;
    let filter = obj.get("filter").and_then(|v| v.as_object());
    let mut out = FindingsFilter::default();
    if let Some(filter) = filter {
        out.severity = filter
            .get("severity")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        out.artifact_id = filter
            .get("artifact_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        out.task_id = filter
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        out.author_steward_id = filter
            .get("author_steward_id")
            .or_else(|| filter.get("steward_id"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
    }
    Ok(out)
}

/// Edit shape for `kind=findings-store` `append`. Single source of
/// truth — both `InMemoryFindingsBackend` and the disk-backed
/// `FilesystemFindingsBackend` parse through `parse_findings_append`.
/// Each backend chooses its own `Finding::id` scheme (sequential for
/// in-memory test fixtures, receipt-derived for the durable disk
/// backend); only the parse contract is shared.
pub struct FindingsAppend {
    pub task_id: TaskId,
    pub author_steward_id: StewardId,
    pub snapshot_id: SnapshotId,
    pub artifact_id: ArtifactId,
    pub range: ArtifactRange,
    pub concern: String,
    pub severity: String,
    pub detail: String,
    pub receipt_id: ReceiptId,
}

pub fn parse_findings_append(edit: &Edit) -> Result<FindingsAppend, String> {
    let obj = edit
        .0
        .as_object()
        .ok_or_else(|| "findings edit must be a JSON object".to_string())?;
    let append = obj
        .get("append")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "findings edit missing required field: append".to_string())?;
    let str_field = |name: &str| -> Result<String, String> {
        append
            .get(name)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| format!("findings append missing required field: {name}"))
    };
    let range_value = append
        .get("range")
        .ok_or_else(|| "findings append missing required field: range".to_string())?;
    Ok(FindingsAppend {
        task_id: TaskId::new(str_field("task_id")?),
        author_steward_id: StewardId::new(str_field("author_steward_id")?),
        snapshot_id: SnapshotId(str_field("snapshot_id")?),
        artifact_id: ArtifactId::new(str_field("artifact_id")?),
        range: parse_artifact_range(range_value)?,
        concern: str_field("concern")?,
        severity: str_field("severity")?,
        detail: str_field("detail")?,
        receipt_id: ReceiptId(str_field("receipt_id")?),
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
/// convenience, top-level `range` + `replacement` fields on the Tool params
/// (without `edit`) are promoted to a text Edit — only when `kind=text`.
fn edit_from_params(kind: &ArtifactKindId, params: &ToolParams) -> Result<Edit, String> {
    if let Some(edit) = params.0.get("edit") {
        return Ok(Edit::from_value(edit.clone()));
    }
    if kind == &kind_text() {
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
        return Ok(Edit::from_value(serde_json::json!({
            "range": range,
            "replacement": replacement,
        })));
    }
    Err(format!(
        "modify_artifact for kind `{kind}` requires an `edit` field"
    ))
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

/// Sugar over `modify_artifact` against the workspace's `kind=findings-store`
/// Backend. Preserves the legacy Tool name so existing Charters that grant
/// `record_finding` continue to work; semantically, this Tool resolves the
/// findings-store artifact and applies an `Edit::Append`.
pub struct RecordFinding {
    id: ToolId,
    store: std::sync::Arc<ArtifactStore>,
    findings_artifact_id: ArtifactId,
}

impl RecordFinding {
    pub fn new(id: ToolId, store: std::sync::Arc<ArtifactStore>) -> Self {
        Self::with_artifact_id(id, store, ArtifactId::new("findings"))
    }

    pub fn with_artifact_id(
        id: ToolId,
        store: std::sync::Arc<ArtifactStore>,
        findings_artifact_id: ArtifactId,
    ) -> Self {
        Self {
            id,
            store,
            findings_artifact_id,
        }
    }
}

#[async_trait]
impl ToolExecutor for RecordFinding {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        // Sugar over modify_artifact against kind=findings-store. Kind and
        // findings-store artifact_id are fixed by the Tool's identity;
        // the params shape carries the finding's content fields.
        let p = &params.0;
        let str_field = |name: &str| -> Result<&str, String> {
            p.get(name)
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("missing required field: {name}"))
        };
        let range_value = p
            .get("range")
            .ok_or_else(|| "missing required field: range".to_string())?;
        let append = serde_json::json!({
            "append": {
                "task_id": str_field("_task_id")?,
                "author_steward_id": str_field("_steward_id")?,
                "snapshot_id": str_field("_snapshot_id")?,
                "receipt_id": str_field("_receipt_id")?,
                "artifact_id": str_field("artifact_id")?,
                "range": range_value,
                "concern": str_field("concern")?,
                "severity": str_field("severity")?,
                "detail": str_field("detail")?,
            }
        });
        let edit = Edit::from_value(append);
        let projection = self
            .store
            .modify(&kind_findings_store(), &self.findings_artifact_id, &edit)
            .await?;
        Ok(projection.0)
    }
}
