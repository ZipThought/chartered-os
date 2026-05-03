//! File-backed Receipt store and CognitionBackend logger.
//!
//! Both write JSON Lines (one record per line) so operators can grep
//! the trail and the LLM call log directly. Per-run isolation: each
//! binary invocation writes to its own directory under
//! `<chartered_dir>/runs/<run_id>/`.
//!
//! The store keeps an in-memory cache alongside the file so that
//! `ReceiptStore::query` (used by Frames with `prior_receipt_queries`)
//! does not have to deserialize off disk on every call. The on-disk
//! file is the durable record; the cache is a derived index built
//! within the same process.
//!
//! Concurrency: every write goes through a `Mutex`-guarded `File`
//! handle. Producers may share the handle across threads (Arc).
//!
//! Effect-surface boundary: this module touches `std::fs`, which is
//! the reason it lives in `runtime/` rather than `core/`. The kernel
//! `core::ReceiptStore` and `core::CognitionBackend` traits stay
//! in-memory only.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chartered_core::{
    CognitionBackend, CognitionError, CognitionRequest, CognitionResponse, FrameId, Receipt,
    ReceiptStore, ReceiptStoreError, receipt_matches_query,
};
use serde::Serialize;

/// JSON-Lines append sink: serializes one record per line, fsyncs each
/// write so an operator-visible trail survives process death. Mutex
/// serializes concurrent writers — log lines never interleave.
///
/// Serialization failure is reported via `eprintln!` and the line is
/// dropped rather than silently rewritten to `"{}"`. AGENTS.md
/// §Error Discipline §Semantic Integrity Under Failure: a dropped
/// record with operator-visible diagnostic beats a fake-shape record
/// that masks the failure.
pub struct JsonlSink {
    path: PathBuf,
    file: Mutex<File>,
}

impl JsonlSink {
    pub fn create(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().append(true).create(true).open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serialize `value` to one JSON line, write+fsync. Returns the IO
    /// error verbatim on write/fsync failure; serializer failure is
    /// reported to stderr and returned as `ErrorKind::InvalidData`.
    /// Callers MUST honor the result — silently writing to disk while
    /// updating an in-memory mirror diverges the two surfaces.
    pub fn append<T: Serialize>(&self, value: &T) -> std::io::Result<()> {
        let line = serde_json::to_string(value).map_err(|e| {
            eprintln!(
                "JsonlSink({}): serialize failure: {e}",
                self.path.display()
            );
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        let mut file = self.file.lock().unwrap();
        writeln!(file, "{line}")?;
        file.sync_data()?;
        Ok(())
    }
}

/// Append-only file-backed Receipt store. Each Receipt is serialized
/// to one JSON line. Reads (`query`, `all`) are served from an
/// in-memory mirror so Frames with `prior_receipt_queries` do not pay
/// disk I/O on every Gate evaluation.
pub struct AppendOnlyFileReceiptStore {
    sink: JsonlSink,
    in_memory: Mutex<Vec<Receipt>>,
}

impl AppendOnlyFileReceiptStore {
    pub fn create(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        Ok(Self {
            sink: JsonlSink::create(path)?,
            in_memory: Mutex::new(Vec::new()),
        })
    }

    pub fn path(&self) -> &Path {
        self.sink.path()
    }
}

impl ReceiptStore for AppendOnlyFileReceiptStore {
    fn append(&self, receipt: &Receipt) -> Result<(), ReceiptStoreError> {
        // Disk-first: only mirror the Receipt in-memory if the durable
        // write succeeded. Otherwise the on-disk trail and the in-memory
        // index would diverge — query() callers (Frame
        // prior_receipt_queries) would see Receipts that aren't in the
        // operator-visible JSONL.
        match self.sink.append(receipt) {
            Ok(()) => {
                self.in_memory.lock().unwrap().push(receipt.clone());
                Ok(())
            }
            Err(e) => Err(ReceiptStoreError(format!(
                "AppendOnlyFileReceiptStore({}): write failure for Receipt {}: {e}",
                self.sink.path().display(),
                receipt.receipt_id
            ))),
        }
    }

    fn query(
        &self,
        context_id: &str,
        steward_id: &chartered_core::StewardId,
        frame_id: Option<&FrameId>,
        limit: usize,
    ) -> Vec<Receipt> {
        self.in_memory
            .lock()
            .unwrap()
            .iter()
            .filter(|r| receipt_matches_query(r, context_id, steward_id, frame_id))
            .take(limit)
            .cloned()
            .collect()
    }

    fn all(&self) -> Vec<Receipt> {
        self.in_memory.lock().unwrap().clone()
    }
}

/// CognitionBackend wrapper that records every (request, response)
/// pair to a shared `JsonlSink`. The wrapped backend's `id()` passes
/// through and is recorded on every log line so operators can grep by
/// role (`actor`, `eval-<frame_id>`, `tester`, `judge`). Multiple
/// `LoggingBackend` instances share one `Arc<JsonlSink>` so log lines
/// never interleave.
pub struct LoggingBackend {
    inner: Arc<dyn CognitionBackend>,
    log: Arc<JsonlSink>,
}

impl LoggingBackend {
    pub fn new(inner: Arc<dyn CognitionBackend>, log: Arc<JsonlSink>) -> Self {
        Self { inner, log }
    }
}

#[async_trait]
impl CognitionBackend for LoggingBackend {
    fn id(&self) -> &str {
        self.inner.id()
    }

    async fn complete(
        &self,
        request: &CognitionRequest,
    ) -> Result<CognitionResponse, CognitionError> {
        let started_ns = unix_nanos();
        let result = self.inner.complete(request).await;
        let finished_ns = unix_nanos();
        let entry = serde_json::json!({
            "started_ns": started_ns,
            "finished_ns": finished_ns,
            "backend_id": self.inner.id(),
            "request": request,
            "response": result.as_ref().ok(),
            "error": result.as_ref().err().map(|e| e.to_string()),
        });
        if let Err(e) = self.log.append(&entry) {
            eprintln!(
                "LoggingBackend({}): cognition log write failure: {e}",
                self.inner.id()
            );
        }
        result
    }
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Build the per-run directory path: `<chartered_dir>/runs/<run_id>/`.
pub fn run_dir(chartered_dir: &Path, run_id: &str) -> PathBuf {
    chartered_dir.join("runs").join(run_id)
}

/// Generate a per-invocation run identifier. Sortable lexicographically
/// (timestamp prefix); collision-resistant within one host (PID
/// suffix). Format: `r-<unix_nanos>-<pid>`.
pub fn make_run_id() -> String {
    let nanos = unix_nanos();
    let pid = std::process::id();
    format!("r-{nanos}-{pid}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chartered_core::FakeCognitionBackend;

    #[tokio::test]
    async fn receipt_store_writes_jsonl_and_caches_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipts.jsonl");
        let store = AppendOnlyFileReceiptStore::create(&path).unwrap();

        let receipt = mk_receipt("ctx-1");
        store.append(&receipt).unwrap();

        // In-memory mirror reflects the write immediately.
        assert_eq!(store.all().len(), 1);

        // The file on disk is one valid JSON line.
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1);
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["tool_call"]["context_id"].as_str(), Some("ctx-1"));
    }

    #[tokio::test]
    async fn cognition_log_records_request_and_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognition.jsonl");
        let log: Arc<JsonlSink> = Arc::new(JsonlSink::create(&path).unwrap());

        let inner = Arc::new(FakeCognitionBackend::new("eval-test"));
        inner.enqueue("ALLOW: ok");
        let wrapped = LoggingBackend::new(inner, log.clone());

        let req = CognitionRequest {
            messages: vec![chartered_core::Message::user("hello")],
            max_output_tokens: Some(64),
        };
        let resp = wrapped.complete(&req).await.unwrap();
        assert_eq!(resp.text, "ALLOW: ok");

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1);
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["backend_id"].as_str(), Some("eval-test"));
        assert_eq!(v["response"]["text"].as_str(), Some("ALLOW: ok"));
        assert!(
            v["request"]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("hello")
        );
    }

    #[tokio::test]
    async fn cognition_log_records_backend_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognition.jsonl");
        let log: Arc<JsonlSink> = Arc::new(JsonlSink::create(&path).unwrap());

        // Empty queue → error on first call.
        let inner = Arc::new(FakeCognitionBackend::new("evaluator"));
        let wrapped = LoggingBackend::new(inner, log.clone());

        let req = CognitionRequest {
            messages: vec![chartered_core::Message::user("x")],
            max_output_tokens: None,
        };
        let result = wrapped.complete(&req).await;
        assert!(result.is_err());

        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert!(v["response"].is_null());
        assert!(v["error"].as_str().unwrap().contains("queue empty"));
    }

    fn mk_receipt(context_id: &str) -> Receipt {
        use chartered_core::*;
        Receipt {
            receipt_id: ReceiptId("test-id".into()),
            task_id: TaskId::new("task-test"),
            attempt_id: Some(AttemptId::new("attempt-test")),
            steward_id: StewardId::new("test-steward"),
            governance_mode: GovernanceMode::FULL,
            tool_call: ToolCall {
                tool: ToolId::new("noop"),
                params: ToolParams(serde_json::json!({})),
                context_id: Arc::from(context_id),
                source_id: Arc::from("steward"),
            },
            verdicts: vec![],
            outcome: Outcome::Allowed,
            timestamp: std::time::SystemTime::now(),
            intercept_complete: true,
            charter_version: 1,
            role_context_version: 1,
            snapshot_id: SnapshotId("snap-test".into()),
        }
    }
}
