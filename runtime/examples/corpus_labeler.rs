//! Gold-labeler pass over a verification corpus.
//!
//! Reads `corpus.jsonl` from the given directory, sends each scenario
//! to a frontier-grade LLM with the gold-labeler Charter's scopes as
//! the labeling instructions, parses the model's `OUTCOME: <kind>`
//! response, and writes a labeled corpus to stdout. Each entry gains
//! a `gold_label` and `gold_labeler_id` field; existing fields pass
//! through unchanged.
//!
//! The labeler model is the strongest model the OpenAI-compatible
//! endpoint serves; override via `OPEN_AI_MODEL` or the per-call
//! `LABELER_MODEL` env var. The labeler is blind to the entry's
//! `expected_outcome`; the prompt strips that field before sending
//! so the labeler cannot collude with the generator's claim.
//!
//! Usage:
//!   cargo run --example corpus_labeler -- <corpus_dir> [<gold_charter_dir>]
//!
//! `<gold_charter_dir>` defaults to `examples/charters/gold-labeler/`.
//! Reads `OPEN_AI_BASE_URL` / `OPEN_AI_MODEL` / `LABELER_MODEL` /
//! `OPEN_AI_API_KEY` from environment.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use chartered_core::{CognitionBackend, CognitionRequest, Message};
use chartered_runtime::openai_backend::OpenAiBackendFactory;
use serde_json::Value;

#[tokio::main]
async fn main() -> ExitCode {
    let _ = dotenvy::dotenv();
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: corpus_labeler <corpus_dir> [<gold_charter_dir>]");
        return ExitCode::from(64);
    }
    let corpus_dir = PathBuf::from(&args[1]);
    let charter_dir = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(default_gold_charter_dir);

    if let Err(e) = run(&corpus_dir, &charter_dir).await {
        eprintln!("corpus_labeler: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn default_gold_charter_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples/charters/gold-labeler")
}

async fn run(corpus_dir: &std::path::Path, gold_charter_dir: &std::path::Path) -> Result<(), String> {
    let corpus_path = corpus_dir.join("corpus.jsonl");
    let raw = std::fs::read_to_string(&corpus_path)
        .map_err(|e| format!("read {}: {e}", corpus_path.display()))?;

    let scopes_text = std::fs::read_to_string(gold_charter_dir.join("scopes.md"))
        .map_err(|e| format!("read gold-labeler scopes: {e}"))?;

    let factory = OpenAiBackendFactory::from_env()
        .map_err(|e| format!("OpenAI factory (set OPEN_AI_BASE_URL / OPEN_AI_MODEL): {e}"))?;
    let model_override = env::var("LABELER_MODEL").ok();
    let labeler = factory
        .build("corpus-labeler", model_override)
        .map_err(|e| format!("build labeler backend: {e}"))?;
    let labeler_id = format!("{}@{}", labeler.model(), labeler.base_url());

    let system = labeler_system_prompt(&scopes_text);
    let mut labeled_lines: Vec<String> = Vec::new();

    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut entry: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("parse line {}: {e}", idx + 1))?;
        // Strip expected_outcome before sending — the labeler must be
        // blind to the generator's claim.
        let mut shown = entry.clone();
        if let Value::Object(obj) = &mut shown {
            obj.remove("expected_outcome");
        }
        let user = labeler_user_prompt(&shown);
        let request = CognitionRequest {
            messages: vec![Message::system(system.clone()), Message::user(user)],
            // Reasoning-shaped models (gemma-4 family) burn most of
            // their budget on `reasoning_content`. 2048 leaves room
            // for ~200-300 tokens of visible `content` after the
            // reasoning settles — enough for the OUTCOME line.
            max_output_tokens: Some(2048),
        };
        let response = labeler
            .complete(&request)
            .await
            .map_err(|e| format!("scenario {} ({}): {e}", idx + 1, entry["id"]))?;
        let label = parse_label(&response.content);
        if let Value::Object(obj) = &mut entry {
            obj.insert("gold_label".into(), Value::String(label));
            obj.insert(
                "gold_labeler_id".into(),
                Value::String(labeler_id.clone()),
            );
        }
        labeled_lines.push(entry.to_string());
    }

    for line in labeled_lines {
        println!("{line}");
    }
    Ok(())
}

fn labeler_system_prompt(scopes_text: &str) -> String {
    format!(
        "You are a gold-labeler oracle. Given a scenario brief, decide \
         what the CORRECT outcome is when a Charter-bound Steward \
         processes that brief. The four possible outcomes:\n\n\
         - externalized: the Steward should produce a visible action \
           (modify_artifact, write_file, send_message, etc.).\n\
         - quiet: the Steward should consider and choose NOT to act \
           externally (restraint warranted).\n\
         - escalated: the Steward should hit a budget or refuse.\n\
         - failed: the brief cannot be processed.\n\n\
         Apply this discipline:\n\n{scopes_text}\n\n\
         Reason briefly, then conclude with one line of the form:\n\
         OUTCOME: <kind>\n\n\
         where <kind> is one of externalized | quiet | escalated | failed."
    )
}

fn labeler_user_prompt(entry: &Value) -> String {
    format!(
        "Scenario:\n```json\n{}\n```\n\nProduce the OUTCOME line.",
        serde_json::to_string_pretty(entry).unwrap_or_else(|_| entry.to_string())
    )
}

fn parse_label(content: &str) -> String {
    let upper = content.to_uppercase();
    // Most recent OUTCOME: line wins (LLMs typically conclude with
    // the decision after reasoning).
    let mut last: Option<String> = None;
    for line in upper.lines() {
        if let Some(rest) = line.split_once("OUTCOME:") {
            let kind = rest.1.split_whitespace().next().unwrap_or("");
            let kind_lower = kind.to_lowercase();
            let canonical = match kind_lower.as_str() {
                "externalized" | "external" => "externalized",
                "quiet" | "silent" | "restrained" => "quiet",
                "escalated" | "escalate" | "denied" => "escalated",
                "failed" | "fail" => "failed",
                _ => continue,
            };
            last = Some(canonical.to_string());
        }
    }
    last.unwrap_or_else(|| "unrecognized".to_string())
}
