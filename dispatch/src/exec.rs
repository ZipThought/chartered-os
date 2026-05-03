//! Native subprocess ToolExecutor.
//!
//! Spec §Tools: "exec_command(cmd, args, env) — dispatch a subprocess.
//! The Gate evaluates the proposed command, args, and env at the
//! tool-call boundary; what the subprocess does after dispatch is
//! outside the Gate." Operators wanting post-dispatch syscall
//! observability deploy a tracer (companion `tracer/` binary or
//! Docker / gVisor / strace).

use std::path::PathBuf;

use async_trait::async_trait;
use chartered_core::{ToolExecutor, ToolId, ToolParams, ToolResult};

/// `exec_command(cmd, args)` — spawns a subprocess in the workspace
/// root, captures stdout/stderr, returns the exit code.
pub struct NativeExec {
    id: ToolId,
    workspace_root: PathBuf,
}

impl NativeExec {
    pub fn new(id: ToolId, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            id,
            workspace_root: workspace_root.into(),
        }
    }
}

#[async_trait]
impl ToolExecutor for NativeExec {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        let cmd = params.require_str("cmd")?.to_string();
        let args = params.optional_string_array("args")?;

        let output = tokio::process::Command::new(&cmd)
            .args(&args)
            .current_dir(&self.workspace_root)
            .output()
            .await
            .map_err(|e| format!("spawn {cmd} failed: {e}"))?;

        Ok(serde_json::json!({
            "exit_code": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
        }))
    }
}
