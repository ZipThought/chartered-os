//! CharteredOS Runtime binary. Per-deployment process from spec §The
//! Runtime. ONE invocation, ONE code path.
//!
//!     chartered-runtime [--chartered-dir <dir>]
//!                       [--workspace-root <dir>]
//!                       [--user-message <text>]
//!                       [--refinement-budget <n>]
//!
//! `.chartered/` walk-up resolves config; `steward.toml` selects per-role
//! backends (fake or real). A test deployment differs from a production
//! deployment only in the `backend` value — every code path the binary
//! runs is identical for both.

use std::path::PathBuf;

use chartered_core::{ArtifactId, ArtifactRange, SelectionAction, SelectionActionKind};
use chartered_runtime::run;

#[tokio::main]
async fn main() {
    // Process env wins; `.env` fills gaps. Walked from CWD upward.
    let _ = dotenvy::dotenv();

    let args: Vec<String> = std::env::args().collect();
    if let Err(e) = dispatch(&args).await {
        eprintln!("chartered-runtime: {e}");
        std::process::exit(1);
    }
}

async fn dispatch(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = run::Options::default();
    let mut selection = SelectionArgs::default();
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        let value = || -> Result<&String, Box<dyn std::error::Error>> {
            args.get(i + 1)
                .ok_or_else(|| format!("missing value for {arg}").into())
        };
        match arg.as_str() {
            "--user-message" => {
                opts.user_message = Some(value()?.clone());
                i += 2;
            }
            "--chartered-dir" => {
                opts.chartered_dir = Some(PathBuf::from(value()?));
                i += 2;
            }
            "--workspace-root" => {
                opts.workspace_root = Some(PathBuf::from(value()?));
                i += 2;
            }
            "--refinement-budget" => {
                opts.refinement_budget = Some(
                    value()?
                        .parse::<usize>()
                        .map_err(|e| format!("--refinement-budget: {e}"))?,
                );
                i += 2;
            }
            "--selection-artifact" => {
                selection.artifact_id = Some(value()?.clone());
                i += 2;
            }
            "--selection-start" => {
                selection.start = Some(
                    value()?
                        .parse::<usize>()
                        .map_err(|e| format!("--selection-start: {e}"))?,
                );
                i += 2;
            }
            "--selection-end" => {
                selection.end = Some(
                    value()?
                        .parse::<usize>()
                        .map_err(|e| format!("--selection-end: {e}"))?,
                );
                i += 2;
            }
            "--selection-start-line" => {
                selection.start_line = Some(
                    value()?
                        .parse::<usize>()
                        .map_err(|e| format!("--selection-start-line: {e}"))?,
                );
                i += 2;
            }
            "--selection-end-line" => {
                selection.end_line = Some(
                    value()?
                        .parse::<usize>()
                        .map_err(|e| format!("--selection-end-line: {e}"))?,
                );
                i += 2;
            }
            "--selection-action" => {
                selection.action = Some(value()?.clone());
                i += 2;
            }
            "--selection-kind" => {
                selection.kind = Some(parse_selection_kind(value()?)?);
                i += 2;
            }
            "--help" | "-h" => {
                usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    opts.selection_trigger = selection.finish()?;
    run::run(opts).await
}

#[derive(Default)]
struct SelectionArgs {
    artifact_id: Option<String>,
    start: Option<usize>,
    end: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    action: Option<String>,
    kind: Option<SelectionActionKind>,
}

impl SelectionArgs {
    fn finish(self) -> Result<Option<run::SelectionTriggerOptions>, Box<dyn std::error::Error>> {
        let any = self.artifact_id.is_some()
            || self.start.is_some()
            || self.end.is_some()
            || self.start_line.is_some()
            || self.end_line.is_some()
            || self.action.is_some()
            || self.kind.is_some();
        if !any {
            return Ok(None);
        }
        Ok(Some(run::SelectionTriggerOptions {
            artifact_id: ArtifactId::new(self.artifact_id.ok_or("missing --selection-artifact")?),
            range: ArtifactRange {
                start: self.start.ok_or("missing --selection-start")?,
                end: self.end.ok_or("missing --selection-end")?,
                start_line: self.start_line.ok_or("missing --selection-start-line")?,
                end_line: self.end_line.ok_or("missing --selection-end-line")?,
            },
            action: SelectionAction {
                name: self.action.ok_or("missing --selection-action")?,
                kind: self.kind.ok_or("missing --selection-kind")?,
            },
        }))
    }
}

fn parse_selection_kind(s: &str) -> Result<SelectionActionKind, Box<dyn std::error::Error>> {
    match s {
        "generative" => Ok(SelectionActionKind::Generative),
        "evaluative" => Ok(SelectionActionKind::Evaluative),
        other => {
            Err(format!("--selection-kind must be generative or evaluative, got `{other}`").into())
        }
    }
}

fn usage() {
    eprintln!("usage: chartered-runtime [opts]");
    eprintln!();
    eprintln!("opts:");
    eprintln!("  --chartered-dir <dir>          override walk-up search for .chartered/");
    eprintln!("  --workspace-root <dir>         override default (parent of .chartered/)");
    eprintln!(
        "  --user-message <text>          single-task input; required unless [tester] in steward.toml"
    );
    eprintln!("  --selection-artifact <id>      artifact path for selection trigger");
    eprintln!("  --selection-start <n>          selection start byte offset");
    eprintln!("  --selection-end <n>            selection end byte offset");
    eprintln!("  --selection-start-line <n>     selection start line");
    eprintln!("  --selection-end-line <n>       selection end line");
    eprintln!("  --selection-action <name>      action label, e.g. Refine or Review");
    eprintln!("  --selection-kind <kind>        generative or evaluative");
    eprintln!("  --refinement-budget <n>        default: 3");
}
