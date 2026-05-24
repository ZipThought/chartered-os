//! Minimal in-process consumer of `chartered_runtime::Agent`.
//!
//! Usage:
//!
//!     cargo run --example agent_embed -- <chartered_dir> [<prompt>]
//!
//! Construct an Agent from a `.chartered/` directory, run one prompt
//! through the governed loop, print the categorical outcome and a
//! brief summary. The example pattern-matches on every `AgentOutcome`
//! variant so a reader sees the full surface.
//!
//! This program holds no agent-conversation state across calls. The
//! Agent caches per-deployment config and HTTP clients; each `run` is
//! atomic and writes a fresh `<chartered_dir>/runs/<run_id>/`.

use std::path::PathBuf;
use std::process::ExitCode;

use chartered_runtime::{Agent, AgentOutcome, Brief};

#[tokio::main]
async fn main() -> ExitCode {
    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: agent_embed <chartered_dir> [<prompt>]");
        return ExitCode::from(64);
    }
    let chartered_dir = PathBuf::from(&args[1]);
    let prompt = args.get(2).cloned().unwrap_or_else(|| {
        "Halt immediately; this run only verifies the embedding API.".into()
    });

    let agent = match Agent::from_chartered_dir(&chartered_dir, None).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("agent_embed: build failed: {e}");
            return ExitCode::from(1);
        }
    };

    let result = match agent.run(Brief::Prompt(prompt)).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("agent_embed: run failed: {e}");
            return ExitCode::from(1);
        }
    };

    println!("run_id={}", result.paths.run_id);
    println!(
        "run_dir={} receipts={}",
        result.paths.run_dir.display(),
        result.artifacts.receipts.len()
    );
    match &result.outcome {
        AgentOutcome::Externalized => println!("outcome: externalized (loop produced a visible effect)"),
        AgentOutcome::Quiet => println!("outcome: quiet (loop completed without externalizing)"),
        AgentOutcome::Escalated { cause } => println!("outcome: escalated ({cause:?})"),
        AgentOutcome::Failed { reason } => println!("outcome: failed ({reason})"),
    }
    ExitCode::SUCCESS
}
