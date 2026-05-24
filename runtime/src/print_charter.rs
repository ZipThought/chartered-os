//! `chartered-runtime --print-charter` — emit the deployment's parsed
//! Charter as JSON to stdout. The dashboard consumes this to render the
//! left-rail Charter tree; no parallel parser in the harness.
//!
//! Single source of truth: this module reuses the runtime's existing
//! Charter and config loaders (the same code paths `run::run` walks at
//! the start of every Selection trigger), then serializes the resulting
//! shape.

use std::path::PathBuf;

use chartered_core::FrameDef;
use serde::Serialize;

use crate::config::{self, ConfigError};

#[derive(Debug, Serialize)]
pub struct PrintedCharter {
    pub charter_ref: CharterRefView,
    pub charter_dir: String,
    pub behavioral_spec: String,
    pub scopes: Vec<NamedSection>,
    pub frames: Vec<FrameView>,
    pub stewards: Vec<StewardView>,
    pub actions: Vec<ActionView>,
    pub role_context_present: bool,
    pub governance_mode: GovernanceModeView,
}

#[derive(Debug, Serialize)]
pub struct CharterRefView {
    pub path: String,
    pub version: u64,
}

#[derive(Debug, Serialize)]
pub struct NamedSection {
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameView {
    pub id: String,
    pub concern: String,
    pub applies_to_tools: Vec<String>,
    pub declared_scopes: Vec<DeclaredScopeView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeclaredScopeView {
    pub name: String,
    pub kind: &'static str,
}

#[derive(Debug, Serialize)]
pub struct StewardView {
    pub id: String,
    pub display_name: String,
    pub frames: Vec<FrameView>,
}

#[derive(Debug, Serialize)]
pub struct ActionView {
    pub name: String,
    pub kind: String,
    pub prompt: String,
}

#[derive(Debug, Serialize)]
pub struct GovernanceModeView {
    pub grounding: bool,
    pub evaluation: bool,
}

pub fn print(chartered_dir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir().map_err(|e| ConfigError(format!("cwd: {e}")))?;
    let chartered_dir = match chartered_dir {
        Some(d) => d,
        None => config::find_chartered_dir(&cwd)
            .ok_or_else(|| ConfigError("no .chartered/ directory found by walk-up search".into()))?,
    };
    let cfg = config::load(&chartered_dir)?;

    let charter_dir = resolve_charter_dir(&cfg)?;
    let frames: Vec<FrameView> = cfg.charter_def.frames.iter().map(frame_view).collect();

    let stewards = vec![StewardView {
        id: "sut".into(),
        display_name: "sut".into(),
        frames: frames.clone(),
    }];

    let actions = parse_actions_from_behavioral_spec(&cfg.charter_def.behavioral_spec);

    let scopes: Vec<NamedSection> = cfg
        .charter_def
        .charter_scopes
        .iter()
        .map(|(name, text)| NamedSection {
            name: name.clone(),
            text: text.clone(),
        })
        .collect();

    let role_context_present = cfg.role_context_def.is_some();
    let governance_mode = GovernanceModeView {
        grounding: cfg.runtime.governance.grounding,
        evaluation: cfg.runtime.governance.evaluation,
    };

    let printed = PrintedCharter {
        charter_ref: CharterRefView {
            path: cfg.charter_ref.path.clone(),
            version: cfg.charter_ref.version,
        },
        charter_dir: charter_dir.display().to_string(),
        behavioral_spec: extract_non_action_body(&cfg.charter_def.behavioral_spec),
        scopes,
        frames,
        stewards,
        actions,
        role_context_present,
        governance_mode,
    };

    let json = serde_json::to_string_pretty(&printed)?;
    println!("{json}");
    Ok(())
}

fn resolve_charter_dir(cfg: &config::DeploymentConfig) -> Result<PathBuf, ConfigError> {
    let raw = std::path::Path::new(&cfg.charter_ref.path);
    let resolved = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cfg.chartered_dir.join(raw)
    };
    resolved
        .canonicalize()
        .map_err(|e| ConfigError(format!("resolving charter dir {}: {e}", resolved.display())))
}

fn frame_view(fd: &FrameDef) -> FrameView {
    FrameView {
        id: fd.id.0.clone(),
        concern: fd.concern.clone(),
        applies_to_tools: fd
            .applies_to_tools
            .iter()
            .map(|t| t.0.clone())
            .collect(),
        declared_scopes: fd
            .declared_scopes
            .iter()
            .map(|ds| DeclaredScopeView {
                name: ds.name.clone(),
                kind: match ds.kind {
                    chartered_core::ScopeKind::Charter => "Charter",
                    chartered_core::ScopeKind::RoleContext => "RoleContext",
                },
            })
            .collect(),
    }
}

// `behavioral_spec.md` may declare structured Actions under a top-level
// `# Actions` section. Each `## <Name>` subsection carries `Type:` and
// `Prompt:` lines that drive the dashboard's selection-trigger button
// bar. The Steward's behavioral prose lives outside that section and
// goes verbatim into the Actor's system prompt.
//
// `parse_actions_from_behavioral_spec` extracts the structured Actions;
// `extract_non_action_body` returns the prose body with the Actions
// section stripped so it can be shown to operators as conduct prose
// distinct from the Action declarations.

fn parse_actions_from_behavioral_spec(behavioral_spec: &str) -> Vec<ActionView> {
    let actions_block = match find_top_level_section(behavioral_spec, "Actions") {
        Some(block) => block,
        None => return Vec::new(),
    };
    let subsections = parse_sub_sections(actions_block);
    subsections
        .into_iter()
        .filter_map(|sub| {
            let mut kind = "generative".to_string();
            let mut prompt_lines: Vec<&str> = Vec::new();
            let mut in_prompt = false;
            for line in sub.text.lines() {
                if let Some(rest) = strip_label_prefix(line, "Type:") {
                    kind = rest.trim().to_lowercase();
                    in_prompt = false;
                } else if let Some(rest) = strip_label_prefix(line, "Prompt:") {
                    prompt_lines.clear();
                    prompt_lines.push(rest.trim_start());
                    in_prompt = true;
                } else if in_prompt {
                    prompt_lines.push(line);
                }
            }
            let prompt = prompt_lines.join("\n").trim().to_string();
            if prompt.is_empty() && kind == "generative" {
                return None;
            }
            Some(ActionView {
                name: sub.name,
                kind,
                prompt,
            })
        })
        .collect()
}

fn extract_non_action_body(behavioral_spec: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    let mut skipping_depth: usize = 0;
    for line in behavioral_spec.lines() {
        if let Some(heading) = parse_heading(line) {
            if heading.depth == 1 {
                skipping = matches!(heading.title.trim(), "Actions" | "Reviewers");
                skipping_depth = if skipping { 1 } else { 0 };
            } else if skipping && heading.depth > skipping_depth {
                // still inside the skipped section
            } else if heading.depth <= skipping_depth {
                skipping = false;
            }
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

struct Heading<'a> {
    depth: usize,
    title: &'a str,
}

fn parse_heading(line: &str) -> Option<Heading<'_>> {
    let trimmed = line.trim_start();
    let depth = trimmed.chars().take_while(|&c| c == '#').count();
    if depth == 0 {
        return None;
    }
    let rest = &trimmed[depth..];
    if !rest.starts_with(' ') {
        return None;
    }
    Some(Heading {
        depth,
        title: rest.trim(),
    })
}

fn find_top_level_section<'a>(text: &'a str, title: &str) -> Option<&'a str> {
    let lines: Vec<&str> = text.lines().collect();
    let mut start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(h) = parse_heading(line)
            && h.depth == 1
            && h.title == title
        {
            start = Some(i + 1);
            break;
        }
    }
    let start = start?;
    let mut end = lines.len();
    for (offset, line) in lines[start..].iter().enumerate() {
        if let Some(h) = parse_heading(line)
            && h.depth == 1
        {
            end = start + offset;
            break;
        }
    }
    let byte_start = byte_offset_of_line(text, start);
    let byte_end = byte_offset_of_line(text, end);
    Some(&text[byte_start..byte_end])
}

fn byte_offset_of_line(text: &str, line_index: usize) -> usize {
    if line_index == 0 {
        return 0;
    }
    let mut current_line = 0;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            current_line += 1;
            if current_line == line_index {
                return i + 1;
            }
        }
    }
    text.len()
}

struct SubSection {
    name: String,
    text: String,
}

fn parse_sub_sections(block: &str) -> Vec<SubSection> {
    let mut out = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_text = String::new();
    for line in block.lines() {
        if let Some(h) = parse_heading(line)
            && h.depth == 2
        {
            if let Some(name) = current_name.take() {
                out.push(SubSection {
                    name,
                    text: current_text.trim().to_string(),
                });
                current_text.clear();
            }
            current_name = Some(h.title.to_string());
        } else if current_name.is_some() {
            current_text.push_str(line);
            current_text.push('\n');
        }
    }
    if let Some(name) = current_name {
        out.push(SubSection {
            name,
            text: current_text.trim().to_string(),
        });
    }
    out
}

fn strip_label_prefix<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let trimmed = line.trim_start();
    let l_label = label.to_lowercase();
    let l_line = trimmed.to_lowercase();
    if l_line.starts_with(&l_label) {
        Some(&trimmed[label.len()..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actions_with_type_and_prompt() {
        let body = "# Actions\n\n## Refine\nType: generative\nPrompt: Tighten the selected text.\n\n## Review\nType: evaluative\nPrompt: Surface a concrete issue.\n";
        let actions = parse_actions_from_behavioral_spec(body);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].name, "Refine");
        assert_eq!(actions[0].kind, "generative");
        assert!(actions[0].prompt.contains("Tighten"));
        assert_eq!(actions[1].name, "Review");
        assert_eq!(actions[1].kind, "evaluative");
    }

    #[test]
    fn non_action_body_strips_actions_and_reviewers() {
        let body = "Conduct prose here.\n\n# Actions\n## A\nType: generative\nPrompt: x\n\n# Reviewers\n## R\nConcern: y\n\n# Tail\nmore prose\n";
        let stripped = extract_non_action_body(body);
        assert!(stripped.contains("Conduct prose here"));
        assert!(stripped.contains("# Tail"));
        assert!(!stripped.contains("Actions"));
        assert!(!stripped.contains("Reviewers"));
    }

    #[test]
    fn empty_actions_section_yields_empty_list() {
        let actions = parse_actions_from_behavioral_spec("Pure conduct prose, no sections.");
        assert!(actions.is_empty());
    }
}
