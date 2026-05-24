//! Verification harness: iterate a checked-in corpus of scenario
//! briefs, run each through one Agent, compare the categorical outcome
//! against the corpus's expected outcome, aggregate per-technique and
//! per-failure-class pass/fail counts. Emits paper-ready JSON to
//! stdout.
//!
//! The harness reuses one in-process Agent across the corpus — the
//! Agent's stateless-across-calls property holds. Subprocess-per-
//! scenario was rejected because per-call process-startup cost
//! dominates the per-call LLM cost at small corpus sizes.
//!
//! The corpus shape is one JSON object per line in `corpus.jsonl`,
//! with fields documented on `ScenarioEntry`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::{Agent, AgentBuildError, AgentOutcome, Brief};

/// One scenario in `corpus.jsonl`. Required fields: `id`, `brief`,
/// `expected_outcome`. Optional fields scope the per-cell aggregation
/// the harness emits.
#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioEntry {
    pub id: String,
    pub brief: String,
    pub expected_outcome: ExpectedOutcome,
    /// Technique label used for per-technique aggregation. Required.
    pub technique: String,
    /// One of `persuasive_prefix` | `adversarial_input` |
    /// `honest_error` | `restraint_warranted`. Required.
    pub failure_class: String,
    /// Optional explanatory note carried into the report row.
    #[serde(default)]
    pub note: Option<String>,
}

/// Categorical expectation; matches the four `AgentOutcome` variants
/// the Agent can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedOutcome {
    Externalized,
    Quiet,
    Escalated,
    Failed,
}

impl ExpectedOutcome {
    fn matches(self, actual: &AgentOutcome) -> bool {
        matches!(
            (self, actual),
            (ExpectedOutcome::Externalized, AgentOutcome::Externalized)
                | (ExpectedOutcome::Quiet, AgentOutcome::Quiet)
                | (ExpectedOutcome::Escalated, AgentOutcome::Escalated { .. })
                | (ExpectedOutcome::Failed, AgentOutcome::Failed { .. })
        )
    }

    fn as_label(self) -> &'static str {
        match self {
            ExpectedOutcome::Externalized => "externalized",
            ExpectedOutcome::Quiet => "quiet",
            ExpectedOutcome::Escalated => "escalated",
            ExpectedOutcome::Failed => "failed",
        }
    }
}

fn actual_label(outcome: &AgentOutcome) -> &'static str {
    match outcome {
        AgentOutcome::Externalized => "externalized",
        AgentOutcome::Quiet => "quiet",
        AgentOutcome::Escalated { .. } => "escalated",
        AgentOutcome::Failed { .. } => "failed",
    }
}

/// Per-scenario row in the report. Always carries the actual outcome
/// alongside the expected so a reviewer can confirm the decision
/// without re-running.
#[derive(Debug, Serialize)]
pub struct ScenarioReport {
    pub id: String,
    pub technique: String,
    pub failure_class: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
    pub run_id: String,
    pub receipts_path: PathBuf,
    pub cognition_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_error: Option<String>,
}

/// Aggregate counts for one cell (one technique × one failure class).
#[derive(Debug, Default, Serialize, Clone)]
pub struct CellAggregate {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

impl CellAggregate {
    fn record(&mut self, passed: bool) {
        self.total += 1;
        if passed {
            self.passed += 1;
        } else {
            self.failed += 1;
        }
    }
}

/// Top-level harness output. The per-scenario `scenarios` field lets
/// a reviewer inspect any individual decision; `by_technique` and
/// `by_failure_class` are the aggregations the paper consumes.
#[derive(Debug, Serialize)]
pub struct SuiteReport {
    pub corpus_dir: PathBuf,
    pub chartered_dir: PathBuf,
    pub scenarios: Vec<ScenarioReport>,
    pub totals: CellAggregate,
    pub by_technique: BTreeMap<String, CellAggregate>,
    pub by_failure_class: BTreeMap<String, CellAggregate>,
    pub by_cell: BTreeMap<String, CellAggregate>,
}

/// Errors that arise before any scenario runs (path resolution,
/// corpus parsing).
#[derive(Debug)]
pub struct SuiteError(pub String);

impl std::fmt::Display for SuiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SuiteError {}

impl From<AgentBuildError> for SuiteError {
    fn from(e: AgentBuildError) -> Self {
        SuiteError(e.0)
    }
}

/// Iterate the corpus once against one Agent. Per-scenario errors are
/// captured into the report (one row per scenario, `passed=false`)
/// rather than aborting the suite — a single backend hiccup does not
/// invalidate the entire run.
pub async fn run_suite(
    chartered_dir: &Path,
    workspace_root: Option<PathBuf>,
    corpus_dir: &Path,
) -> Result<SuiteReport, SuiteError> {
    let corpus_path = corpus_dir.join("corpus.jsonl");
    let raw = std::fs::read_to_string(&corpus_path)
        .map_err(|e| SuiteError(format!("read {}: {e}", corpus_path.display())))?;

    let mut entries: Vec<ScenarioEntry> = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let entry: ScenarioEntry = serde_json::from_str(trimmed)
            .map_err(|e| SuiteError(format!("parse {} line {}: {e}", corpus_path.display(), idx + 1)))?;
        entries.push(entry);
    }

    if entries.is_empty() {
        return Err(SuiteError(format!(
            "corpus {} is empty (no non-comment lines)",
            corpus_path.display()
        )));
    }

    let agent = Agent::from_chartered_dir(chartered_dir, workspace_root).await?;

    let mut scenarios: Vec<ScenarioReport> = Vec::with_capacity(entries.len());
    let mut totals = CellAggregate::default();
    let mut by_technique: BTreeMap<String, CellAggregate> = BTreeMap::new();
    let mut by_failure_class: BTreeMap<String, CellAggregate> = BTreeMap::new();
    let mut by_cell: BTreeMap<String, CellAggregate> = BTreeMap::new();

    for entry in entries {
        let cell_key = format!("{}|{}", entry.technique, entry.failure_class);
        let row = match agent.run(Brief::Prompt(entry.brief.clone())).await {
            Ok(result) => {
                let passed = entry.expected_outcome.matches(&result.outcome);
                ScenarioReport {
                    id: entry.id.clone(),
                    technique: entry.technique.clone(),
                    failure_class: entry.failure_class.clone(),
                    expected: entry.expected_outcome.as_label().to_string(),
                    actual: actual_label(&result.outcome).to_string(),
                    passed,
                    run_id: result.paths.run_id,
                    receipts_path: result.paths.receipts_log,
                    cognition_path: result.paths.cognition_log,
                    note: entry.note.clone(),
                    agent_error: None,
                }
            }
            Err(e) => ScenarioReport {
                id: entry.id.clone(),
                technique: entry.technique.clone(),
                failure_class: entry.failure_class.clone(),
                expected: entry.expected_outcome.as_label().to_string(),
                actual: "agent_error".to_string(),
                passed: false,
                run_id: String::new(),
                receipts_path: PathBuf::new(),
                cognition_path: PathBuf::new(),
                note: entry.note.clone(),
                agent_error: Some(e.to_string()),
            },
        };
        totals.record(row.passed);
        by_technique
            .entry(entry.technique.clone())
            .or_default()
            .record(row.passed);
        by_failure_class
            .entry(entry.failure_class.clone())
            .or_default()
            .record(row.passed);
        by_cell.entry(cell_key).or_default().record(row.passed);
        scenarios.push(row);
    }

    Ok(SuiteReport {
        corpus_dir: corpus_dir.to_path_buf(),
        chartered_dir: chartered_dir.to_path_buf(),
        scenarios,
        totals,
        by_technique,
        by_failure_class,
        by_cell,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_outcome_matches_each_variant() {
        assert!(ExpectedOutcome::Quiet.matches(&AgentOutcome::Quiet));
        assert!(ExpectedOutcome::Externalized.matches(&AgentOutcome::Externalized));
        assert!(
            ExpectedOutcome::Escalated.matches(&AgentOutcome::Escalated {
                cause: crate::agent::EscalationCause::InnerStepBudget,
            })
        );
        assert!(
            ExpectedOutcome::Failed.matches(&AgentOutcome::Failed {
                reason: "any".into(),
            })
        );
        assert!(!ExpectedOutcome::Quiet.matches(&AgentOutcome::Externalized));
    }
}
