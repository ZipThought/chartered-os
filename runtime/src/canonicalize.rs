//! Shared adapter utility: convert a raw assistant content string into
//! an `Option<ActionHint>`.
//!
//! Why here, not in the kernel: the kernel's contract is strong-typed
//! (`ActionHint`). The kernel does NOT parse JSON or scan for code
//! fences — that's the adapter's job. Adapters that need the same
//! generic canonicalization (universal LLM idioms: markdown fences
//! around JSON, reasoning prose followed by JSON) share this utility.
//! Vendor-specific envelopes (gpt-oss harmony, OpenAI native
//! `tool_calls` field, Gemini `functionCall`) stay in the adapter that
//! owns them.
//!
//! Spec correspondence: `SPECIFICATION.md §Cognition Layer` ("the
//! interface handles multiple response formats so that any cognition
//! implementation satisfies the kernel's contract") + the
//! kernel-doesn't-parse-JSON principle.

use chartered_core::{ActionHint, Decision, DecisionLine, JudgeOutput};

/// Try to extract an `ActionHint` from the assistant's raw content.
/// Strips markdown code fences, then scans for the first balanced
/// `{...}` JSON object and interprets it as the canonical Action
/// envelope: `{"halt": true}` → `Halt`; `{"tool": "...", "params":
/// {...}}` → `Propose`. Returns `None` for pure-reasoning responses
/// (no JSON, or JSON without the expected fields) — the Actor's inner
/// loop continues.
pub fn canonicalize_action_hint(raw: &str) -> Option<ActionHint> {
    let stripped = strip_code_fences(raw);
    let json_text = extract_first_json_object(stripped).unwrap_or(stripped);
    let value: serde_json::Value = serde_json::from_str(json_text.trim()).ok()?;
    parse_action_value(&value)
}

/// Parse a `serde_json::Value` as the canonical Action envelope. Public
/// because adapters that have already deserialized vendor-native
/// tool-call JSON (e.g. OpenAI native `tool_calls`, Gemini
/// `functionCall`) can call this directly to wrap into `ActionHint`.
pub fn parse_action_value(value: &serde_json::Value) -> Option<ActionHint> {
    if value.get("halt").and_then(serde_json::Value::as_bool) == Some(true) {
        return Some(ActionHint::Halt);
    }
    let tool = value.get("tool").and_then(serde_json::Value::as_str)?;
    let params = value
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Some(ActionHint::Propose {
        tool: tool.to_string(),
        params,
    })
}

/// Strip a single layer of leading/trailing markdown code fence markers
/// (```json, ```JSON, or plain ```). Whitespace-tolerant.
pub fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```JSON"))
        .or_else(|| s.strip_prefix("```"))
        .map(str::trim)
        .unwrap_or(s);
    s.trim_end_matches("```").trim()
}

/// Scan the raw assistant content for Evaluator decision lines. Each
/// recognized line yields one `DecisionLine` (`{decision, observation}`).
/// Empty vec when nothing matches.
///
/// The scan is lenient — it locates the first `ALLOW` / `DENY` /
/// `ESCALATE` / `DEFER` token (uppercase, word-boundary-aware)
/// anywhere in a line, including after reasoning prose, and reads
/// the text after the next `:` as the observation. Lines that
/// contain the legacy `DECISION: <KW>[, REASON: ...]` paraphrase are
/// also handled. Lines without a recognized token are dropped.
pub fn canonicalize_verdict_lines(raw: &str) -> Vec<DecisionLine> {
    raw.lines()
        .filter_map(canonicalize_one_decision_line)
        .collect()
}

fn canonicalize_one_decision_line(line: &str) -> Option<DecisionLine> {
    // Locate the latest keyword occurrence in the line, then read the
    // observation from after the next `:`. Latest, not earliest, so
    // prose containing the word "allow" earlier in the line doesn't
    // shadow the actual ALLOW: token at the end (the LLM's emitted
    // decision is typically near the end after the reasoning prose).
    const KEYWORDS: &[(&str, Decision)] = &[
        ("ALLOW", Decision::Allow),
        ("DENY", Decision::Deny),
        ("ESCALATE", Decision::Escalate),
        ("DEFER", Decision::Defer),
    ];

    let upper = line.to_uppercase();
    let mut best: Option<(usize, Decision, usize)> = None;
    for (kw, dec) in KEYWORDS {
        let mut search_from = 0usize;
        while let Some(rel) = upper[search_from..].find(kw) {
            let abs = search_from + rel;
            let before_ok = abs == 0
                || !upper.as_bytes()[abs - 1].is_ascii_alphanumeric();
            let after = abs + kw.len();
            let after_ok = after >= upper.len()
                || !upper.as_bytes()[after].is_ascii_alphanumeric();
            if before_ok && after_ok && best.is_none_or(|(pos, _, _)| abs > pos) {
                best = Some((abs, *dec, kw.len()));
            }
            search_from = abs + kw.len();
        }
    }
    let (kw_pos, decision, kw_len) = best?;

    // Two shapes: `DECISION: <KW>, ...` (the keyword appears in the
    // tail of a DECISION:-prefixed line) and `<KW>: reason` (the
    // keyword is directly followed by `:`). Both produce the same
    // DecisionLine; we differ only on where to read the observation.
    let after_kw = &line[kw_pos + kw_len..];
    let observation = if let Some((_, tail)) = after_kw.split_once(':') {
        tail.trim().trim_start_matches(',').trim().to_string()
    } else {
        // `DECISION: ALLOW` with no reason — degenerate paraphrase.
        // Drop a leading comma + optional "REASON:" prefix.
        let mut rest = after_kw.trim().trim_start_matches(',').trim();
        if let Some(stripped) = rest
            .strip_prefix("REASON:")
            .or_else(|| rest.strip_prefix("Reason:"))
            .or_else(|| rest.strip_prefix("reason:"))
        {
            rest = stripped.trim();
        }
        rest.to_string()
    };

    Some(DecisionLine {
        decision,
        observation,
    })
}

/// Try to extract the strong-typed Judge output from raw assistant
/// content. Strips fences, extracts the first balanced JSON object,
/// deserializes via serde. Returns `None` when nothing parses — the
/// Judge layer surfaces this as a `JudgeError` ("adapter produced no
/// judge_output").
pub fn canonicalize_judge_output(raw: &str) -> Option<JudgeOutput> {
    let stripped = strip_code_fences(raw);
    let json_text = extract_first_json_object(stripped).unwrap_or(stripped);
    serde_json::from_str::<JudgeOutput>(json_text.trim()).ok()
}

/// Walk `text` and return the first balanced top-level JSON object
/// substring. String/escape state is tracked so braces inside JSON
/// strings do not confuse depth tracking. Returns None when no
/// balanced object is present.
pub fn extract_first_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'"' => in_string = true,
            b'}' => {
                depth -= 1;
                if depth == 0
                    && let Some(s) = start
                {
                    return Some(&text[s..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_prose_yields_none() {
        assert!(canonicalize_action_hint("just thinking out loud").is_none());
    }

    #[test]
    fn raw_propose_json_yields_propose() {
        let h = canonicalize_action_hint(r#"{"tool":"write_file","params":{"path":"x"}}"#).unwrap();
        match h {
            ActionHint::Propose { tool, params } => {
                assert_eq!(tool, "write_file");
                assert_eq!(params["path"].as_str(), Some("x"));
            }
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn raw_halt_json_yields_halt() {
        let h = canonicalize_action_hint(r#"{"halt": true}"#).unwrap();
        assert!(matches!(h, ActionHint::Halt));
    }

    #[test]
    fn propose_with_no_params_field_yields_null_params() {
        let h = canonicalize_action_hint(r#"{"tool":"halt_check"}"#).unwrap();
        match h {
            ActionHint::Propose { tool, params } => {
                assert_eq!(tool, "halt_check");
                assert_eq!(params, serde_json::Value::Null);
            }
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn missing_tool_and_no_halt_yields_none() {
        assert!(canonicalize_action_hint(r#"{"params":{"x":1}}"#).is_none());
    }

    #[test]
    fn markdown_fenced_json_is_unwrapped() {
        let h = canonicalize_action_hint("```json\n{\"halt\":true}\n```").unwrap();
        assert!(matches!(h, ActionHint::Halt));
    }

    #[test]
    fn prose_prefix_before_json_is_extracted() {
        // Gemini-shaped output: reasoning prose followed by the action.
        let s = "I'll write a file. {\"tool\":\"write_file\",\"params\":{\"path\":\"x\"}}";
        let h = canonicalize_action_hint(s).unwrap();
        match h {
            ActionHint::Propose { tool, .. } => assert_eq!(tool, "write_file"),
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn braces_inside_strings_do_not_terminate_extraction() {
        let s = r#"prose {"k":"a }{ b","n":1}"#;
        assert_eq!(
            extract_first_json_object(s),
            Some(r#"{"k":"a }{ b","n":1}"#)
        );
    }

    #[test]
    fn escaped_quotes_inside_strings_dont_break_state() {
        let s = r#"prefix {"k":"with \"quote\"","n":1}"#;
        assert_eq!(
            extract_first_json_object(s),
            Some(r#"{"k":"with \"quote\"","n":1}"#)
        );
    }
}
