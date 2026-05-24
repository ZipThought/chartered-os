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

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use chartered_core::{
    CognitionBackend, CognitionError, CognitionRequest, CognitionResponse, FrameId, Receipt,
    ReceiptStore, ReceiptStoreError, receipt_matches_query,
};
use chartered_dispatch::JsonlSink;

/// Append-only file-backed Receipt store. Each Receipt is serialized
/// to one JSON line via the shared `JsonlSink` primitive. Reads
/// (`query`, `all`) are served from an in-memory mirror so Frames with
/// `prior_receipt_queries` do not pay disk I/O on every Gate
/// evaluation.
pub struct AppendOnlyFileReceiptStore {
    sink: JsonlSink,
    in_memory: StdMutex<Vec<Receipt>>,
}

impl AppendOnlyFileReceiptStore {
    pub async fn create(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        Ok(Self {
            sink: JsonlSink::create(path).await?,
            in_memory: StdMutex::new(Vec::new()),
        })
    }

    pub fn path(&self) -> &Path {
        self.sink.path()
    }
}

#[async_trait]
impl ReceiptStore for AppendOnlyFileReceiptStore {
    async fn append(&self, receipt: &Receipt) -> Result<(), ReceiptStoreError> {
        // Disk-first: only mirror the Receipt in-memory if the durable
        // write succeeded. Otherwise the on-disk trail and the in-memory
        // index would diverge — query() callers (Frame
        // prior_receipt_queries) would see Receipts that aren't in the
        // operator-visible JSONL.
        match self.sink.append(receipt).await {
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
        if let Err(e) = self.log.append(&entry).await {
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

