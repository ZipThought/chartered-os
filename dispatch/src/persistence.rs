//! Unified persistence primitive for every durable append-only stream.
//!
//! Spec §Persistence: "One async append-only primitive serves every
//! durable stream — Receipt log, Cognition log, Findings. One
//! open/append/fsync discipline. One serialization-failure contract."
//!
//! Lives in `dispatch/` (not `core/`) because it touches `tokio::fs`;
//! the kernel stays in-memory only. Runtime and dispatch both reach for
//! it via `chartered_dispatch::JsonlSink`.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// JSON-Lines append sink: serializes one record per line, fsyncs each
/// write so an operator-visible trail survives process death. Mutex
/// serializes concurrent writers — log lines never interleave.
///
/// Serialization failure is reported via `eprintln!` and returned as
/// `ErrorKind::InvalidData`; the line is dropped rather than silently
/// rewritten to `"{}"`. AGENTS.md §Error Discipline §Semantic Integrity
/// Under Failure: a dropped record with operator-visible diagnostic
/// beats a fake-shape record that masks the failure.
pub struct JsonlSink {
    path: PathBuf,
    file: Mutex<File>,
}

impl JsonlSink {
    pub async fn create(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .await?;
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
    pub async fn append<T: Serialize>(&self, value: &T) -> std::io::Result<()> {
        let mut line = serde_json::to_string(value).map_err(|e| {
            eprintln!(
                "JsonlSink({}): serialize failure: {e}",
                self.path.display()
            );
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        line.push('\n');
        let mut file = self.file.lock().await;
        file.write_all(line.as_bytes()).await?;
        file.sync_data().await?;
        Ok(())
    }
}
