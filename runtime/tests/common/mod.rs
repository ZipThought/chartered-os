//! Shared E2E fixtures: spin a `.chartered/` deployment in a tempdir
//! and run the chartered-runtime binary against it. Each integration
//! test binary loads this module via `mod common;`.
//!
//! The deployments are real production deployments — same loader, same
//! runtime path, same OS-touching dispatch tools. Only the per-role
//! `backend` value in `steward.toml` differs between fake-mode CI and
//! real-LLM tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

/// Path to the binary under test, materialized by Cargo.
pub const BIN: &str = env!("CARGO_BIN_EXE_chartered-runtime");

/// One isolated deployment in a tempdir. Drop deletes it.
pub struct TestDeployment {
    _tmp: TempDir,
    pub workspace_root: PathBuf,
    pub chartered_dir: PathBuf,
    pub charter_dir: PathBuf,
}

impl TestDeployment {
    pub fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let workspace_root = tmp.path().to_path_buf();
        let chartered_dir = workspace_root.join(".chartered");
        std::fs::create_dir_all(&chartered_dir).unwrap();
        std::fs::create_dir_all(chartered_dir.join("tools")).unwrap();
        let charter_dir = workspace_root.join("charter");
        std::fs::create_dir_all(&charter_dir).unwrap();
        Self {
            _tmp: tmp,
            workspace_root,
            chartered_dir,
            charter_dir,
        }
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.chartered_dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    pub fn write_charter(&self, frames_toml: &str, scopes_md: &str) {
        std::fs::write(self.charter_dir.join("frames.toml"), frames_toml).unwrap();
        std::fs::write(self.charter_dir.join("scopes.md"), scopes_md).unwrap();
        // Charter loader requires behavioral_spec.md per spec §The
        // Charter. Tests that exercise behavioral content override
        // this; tests that only exercise the kernel mechanism use the
        // empty default.
        std::fs::write(
            self.charter_dir.join("behavioral_spec.md"),
            "Test Steward. JSON Action objects only.\n",
        )
        .unwrap();
    }

    /// Like `write_charter` but lets the test name a specific
    /// behavioral_spec body. Use for tests that assert on the assembled
    /// Actor system prompt.
    pub fn write_charter_with_spec(
        &self,
        frames_toml: &str,
        scopes_md: &str,
        behavioral_spec: &str,
    ) {
        std::fs::write(self.charter_dir.join("frames.toml"), frames_toml).unwrap();
        std::fs::write(self.charter_dir.join("scopes.md"), scopes_md).unwrap();
        std::fs::write(self.charter_dir.join("behavioral_spec.md"), behavioral_spec).unwrap();
    }

    pub fn write_charter_ref(&self, version: u64) {
        let path_str = self.charter_dir.to_string_lossy().to_string();
        let toml = format!("path = \"{path_str}\"\nversion = {version}\n");
        self.write("charter.toml", &toml);
    }

    pub fn write_chartered_toml(&self) {
        self.write(
            "chartered.toml",
            "[governance]\ngrounding = true\nevaluation = true\n",
        );
    }

    pub fn write_role_context_md(&self) {
        self.write("role_context.md", "# Role context\n\n(empty)\n");
    }

    pub fn run_with_user_message(&self, msg: &str) -> Output {
        Command::new(BIN)
            .arg("--chartered-dir")
            .arg(&self.chartered_dir)
            .arg("--workspace-root")
            .arg(&self.workspace_root)
            .arg("--user-message")
            .arg(msg)
            .output()
            .expect("binary executes")
    }

    pub fn run_no_user_message(&self) -> Output {
        Command::new(BIN)
            .arg("--chartered-dir")
            .arg(&self.chartered_dir)
            .arg("--workspace-root")
            .arg(&self.workspace_root)
            .output()
            .expect("binary executes")
    }

    pub fn workspace_file(&self, rel: &str) -> PathBuf {
        self.workspace_root.join(rel)
    }
}

pub fn parse_stdout_json(out: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout not JSON: {e}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

pub fn assert_success(out: &Output) {
    assert!(
        out.status.success(),
        "binary exited nonzero: {}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn write_tool(dep: &TestDeployment, name: &str, executor: &str, tool_id: &str) {
    dep.write(
        &format!("tools/{name}.toml"),
        &format!("id = \"{tool_id}\"\nexecutor = \"{executor}\"\n"),
    );
}

pub fn list_files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    visit(root, &mut out);
    out
}

fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            visit(&p, out);
        } else {
            out.push(p);
        }
    }
}
