//! Binary entry path: parses CLI Options, constructs an Agent,
//! converts Options into a Brief, runs once, prints the JSON shape the
//! dashboard and integration tests consume. The library API
//! (`chartered_runtime::Agent`) is the load-bearing surface; this
//! module is a thin translation layer between argv and that surface.

use std::path::PathBuf;

use chartered_core::{ArtifactId, ArtifactRange, SelectionAction};
use serde::Serialize;

use crate::agent::{Agent, AgentOutcome, Brief, RunArtifacts, RunPaths};

#[derive(Debug, Default)]
pub struct Options {
    pub chartered_dir: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
    pub user_message: Option<String>,
    pub selection_trigger: Option<SelectionTriggerOptions>,
    pub refinement_budget: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SelectionTriggerOptions {
    pub artifact_id: ArtifactId,
    pub range: ArtifactRange,
    pub action: SelectionAction,
}

/// Stdout shape produced by the binary. Mirrors the prior format so
/// existing dashboard parsers and integration tests continue to work.
#[derive(Debug, Serialize)]
struct BinaryStdout {
    workspace_id: String,
    run_id: String,
    run_dir: String,
    receipts_log: String,
    cognition_log: String,
    tasks: Vec<chartered_core::TaskRecord>,
    attempts: Vec<chartered_core::AttemptRecord>,
    receipts: Vec<chartered_core::Receipt>,
    judge: chartered_core::JudgeReport,
    turns: usize,
    terminated_by_budget: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tester_failure: Option<String>,
    outcome: AgentOutcome,
}

pub async fn run(opts: Options) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let chartered_dir = match opts.chartered_dir {
        Some(d) => d,
        None => crate::config::find_chartered_dir(&cwd).ok_or_else(|| {
            crate::agent::RunError("no .chartered/ directory found by walk-up search".into())
        })?,
    };

    let mut agent = Agent::from_chartered_dir(&chartered_dir, opts.workspace_root).await?;
    if let Some(b) = opts.refinement_budget {
        agent = agent.with_refinement_budget(b);
    }

    let brief = brief_from_options(
        agent.workspace_root().to_path_buf(),
        opts.user_message,
        opts.selection_trigger,
        agent.has_configured_tester(),
    )?;

    let result = agent.run(brief).await?;

    let RunArtifacts {
        tasks,
        attempts,
        receipts,
        judge,
        turns,
        terminated_by_budget,
        tester_failure,
    } = result.artifacts;
    let RunPaths {
        workspace_id,
        run_id,
        run_dir,
        receipts_log,
        cognition_log,
    } = result.paths;

    let stdout = BinaryStdout {
        workspace_id,
        run_id,
        run_dir: run_dir.display().to_string(),
        receipts_log: receipts_log.display().to_string(),
        cognition_log: cognition_log.display().to_string(),
        tasks,
        attempts,
        receipts,
        judge,
        turns,
        terminated_by_budget,
        tester_failure,
        outcome: result.outcome,
    };
    println!("{}", serde_json::to_string_pretty(&stdout)?);
    Ok(())
}

/// Translate the binary's CLI Options into one Brief variant. The
/// binary's interface is a convenience over the Agent's; future CLI
/// flags map to new Brief variants behind the same boundary.
fn brief_from_options(
    workspace_root: PathBuf,
    user_message: Option<String>,
    selection: Option<SelectionTriggerOptions>,
    has_configured_tester: bool,
) -> Result<Brief, crate::agent::RunError> {
    if let (Some(_), Some(_)) = (&user_message, &selection) {
        return Err(crate::agent::RunError(
            "--user-message and --selection-* are both set; use one trigger".into(),
        ));
    }
    if has_configured_tester {
        if user_message.is_some() || selection.is_some() {
            return Err(crate::agent::RunError(
                "[tester] in steward.toml and a singleton trigger are both set; use one".into(),
            ));
        }
        return Ok(Brief::TesterDriven);
    }
    if let Some(text) = user_message {
        return Ok(Brief::Prompt(text));
    }
    if let Some(s) = selection {
        let _ = workspace_root; // workspace_root carried for symmetry; Agent reads it.
        return Ok(Brief::Selection {
            artifact_id: s.artifact_id,
            range: s.range,
            action: s.action,
        });
    }
    Err(crate::agent::RunError(
        "neither [tester] in steward.toml nor --user-message / --selection-* is set".into(),
    ))
}
