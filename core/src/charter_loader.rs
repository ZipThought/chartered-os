//! Load Charter and Role context from filesystem artifacts. Spec
//! §The Charter, §Role Context, §Reference Charters.
//!
//! Inputs:
//! - `<charter_dir>/frames.toml` — `permitted_tools` + `[[frames]]`
//!   tables with `id`, `concern`, `applies_to_tools`, typed
//!   `declared_scopes`, optional `prior_receipt_queries`.
//! - `<charter_dir>/scopes.md` — Charter Scopes; `## Heading` lines
//!   become scope names via slugify (lowercase, non-alphanumeric runs
//!   collapsed to `_`); content between headings becomes scope text.
//! - `<deployment>/role_context.md` — Role context Scopes; same
//!   markdown format as scopes.md but parsed by `load_role_context_def`
//!   into `RoleContextDef`.
//!
//! Two-stage construction: parser → `CharterDef`/`RoleContextDef`
//! (data only, no Evaluator instances), then `build_charter` /
//! `build_role_context` materialize the runtime Charter/RoleContext
//! by attaching an Evaluator (caller chooses backend) and a version
//! number (caller supplies from `charter.toml` / `role_context.md`
//! deployment metadata).

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::charter::{Charter, DeclaredScope, RoleContext, ScopeKind};
use crate::frame::{Frame, FrameId, PriorReceiptQuery};
use crate::tool::ToolId;
use crate::verdict::Evaluator;

#[derive(Debug, Clone)]
pub struct CharterLoadError(pub String);

impl std::fmt::Display for CharterLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CharterLoadError {}

/// Parsed Charter shape, lacking Evaluator instances. The caller
/// materializes a runtime Charter via `build_charter`.
#[derive(Debug, Clone)]
pub struct CharterDef {
    pub permitted_tools: Vec<ToolId>,
    pub charter_scopes: Vec<(String, String)>,
    pub behavioral_spec: String,
    pub frames: Vec<FrameDef>,
    pub charter_content_hash: String,
}

#[derive(Debug, Clone)]
pub struct FrameDef {
    pub id: FrameId,
    pub concern: String,
    pub applies_to_tools: Vec<ToolId>,
    pub declared_scopes: Vec<DeclaredScope>,
    pub prior_receipt_queries: Vec<PriorReceiptQuery>,
}

/// Parsed Role context shape, lacking version.
#[derive(Debug, Clone)]
pub struct RoleContextDef {
    pub scopes: Vec<(String, String)>,
    pub role_context_content_hash: String,
}

/// Load a Charter from a directory containing `frames.toml`,
/// `scopes.md`, and `behavioral_spec.md`. Returns `CharterDef` (data
/// only); use `build_charter` to attach Evaluators.
///
/// `behavioral_spec.md` is the conduct prose the Runtime composes into
/// the Actor's system prompt. Per spec §The Charter, this lives on the
/// Charter (per-Steward), not in deployment-side configuration.
pub fn load_charter_def(charter_dir: &Path) -> Result<CharterDef, CharterLoadError> {
    let frames_path = charter_dir.join("frames.toml");
    let scopes_path = charter_dir.join("scopes.md");
    let behavioral_spec_path = charter_dir.join("behavioral_spec.md");

    let frames_text = std::fs::read_to_string(&frames_path).map_err(|e| {
        CharterLoadError(format!("reading {}: {e}", frames_path.display()))
    })?;
    let scopes_text = std::fs::read_to_string(&scopes_path).map_err(|e| {
        CharterLoadError(format!("reading {}: {e}", scopes_path.display()))
    })?;
    let behavioral_spec = std::fs::read_to_string(&behavioral_spec_path).map_err(|e| {
        CharterLoadError(format!("reading {}: {e}", behavioral_spec_path.display()))
    })?;

    let doc: FramesDocToml = toml::from_str(&frames_text).map_err(|e| {
        CharterLoadError(format!("parsing {}: {e}", frames_path.display()))
    })?;

    let charter_scopes = parse_named_sections(&scopes_text);

    let mut frames = Vec::with_capacity(doc.frames.len());
    for ft in doc.frames {
        let mut declared_scopes = Vec::with_capacity(ft.declared_scopes.len());
        for ds in ft.declared_scopes {
            let kind = match ds.kind.as_str() {
                "Charter" => ScopeKind::Charter,
                "RoleContext" => ScopeKind::RoleContext,
                other => {
                    return Err(CharterLoadError(format!(
                        "unknown ScopeKind `{other}` in frame `{}` of {}",
                        ft.id,
                        frames_path.display()
                    )));
                }
            };
            declared_scopes.push(DeclaredScope { name: ds.name, kind });
        }
        frames.push(FrameDef {
            id: FrameId::new(ft.id),
            concern: ft.concern,
            applies_to_tools: ft.applies_to_tools.into_iter().map(ToolId::new).collect(),
            declared_scopes,
            prior_receipt_queries: ft
                .prior_receipt_queries
                .into_iter()
                .map(|q| PriorReceiptQuery {
                    frame_id_filter: q.frame_id_filter.map(FrameId::new),
                    limit: q.limit,
                })
                .collect(),
        });
    }

    let mut hasher = Sha256::new();
    hasher.update(frames_text.as_bytes());
    hasher.update(b":");
    hasher.update(scopes_text.as_bytes());
    hasher.update(b":");
    hasher.update(behavioral_spec.as_bytes());
    let charter_content_hash = hex::encode(hasher.finalize());

    Ok(CharterDef {
        permitted_tools: doc.permitted_tools.into_iter().map(ToolId::new).collect(),
        charter_scopes,
        behavioral_spec,
        frames,
        charter_content_hash,
    })
}

/// Materialize a runtime Charter from a CharterDef. The factory is
/// called once per Frame and supplies the Evaluator instance (any
/// `CognitionBackend` impl).
pub fn build_charter<F>(
    def: CharterDef,
    charter_version: u64,
    mut evaluator_factory: F,
) -> Charter
where
    F: FnMut(&FrameDef) -> Arc<dyn Evaluator>,
{
    let frames = def
        .frames
        .iter()
        .map(|fd| Frame {
            id: fd.id.clone(),
            concern: fd.concern.clone(),
            applies_to_tools: fd.applies_to_tools.clone(),
            declared_scopes: fd.declared_scopes.clone(),
            evaluator: evaluator_factory(fd),
            prior_receipt_queries: fd.prior_receipt_queries.clone(),
        })
        .collect();

    Charter {
        frames,
        permitted_tools: def.permitted_tools,
        charter_scopes: def.charter_scopes,
        behavioral_spec: def.behavioral_spec,
        charter_version,
        charter_content_hash: def.charter_content_hash,
    }
}

/// Load Role context from a markdown file using the same `## Heading`
/// → slugified-name + content convention as `scopes.md`.
pub fn load_role_context_def(path: &Path) -> Result<RoleContextDef, CharterLoadError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CharterLoadError(format!("reading {}: {e}", path.display())))?;
    let scopes = parse_named_sections(&text);
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let role_context_content_hash = hex::encode(hasher.finalize());
    Ok(RoleContextDef {
        scopes,
        role_context_content_hash,
    })
}

pub fn build_role_context(def: RoleContextDef, role_context_version: u64) -> RoleContext {
    RoleContext {
        scopes: def.scopes,
        role_context_version,
        role_context_content_hash: def.role_context_content_hash,
    }
}

#[derive(Deserialize)]
struct FramesDocToml {
    permitted_tools: Vec<String>,
    frames: Vec<FrameToml>,
}

#[derive(Deserialize)]
struct FrameToml {
    id: String,
    concern: String,
    applies_to_tools: Vec<String>,
    #[serde(default)]
    declared_scopes: Vec<DeclaredScopeToml>,
    #[serde(default)]
    prior_receipt_queries: Vec<PriorReceiptQueryToml>,
}

#[derive(Deserialize)]
struct DeclaredScopeToml {
    name: String,
    kind: String,
}

#[derive(Deserialize)]
struct PriorReceiptQueryToml {
    #[serde(default)]
    frame_id_filter: Option<String>,
    limit: usize,
}

fn parse_named_sections(md: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_content = String::new();
    for line in md.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(name) = current_name.take() {
                sections.push((name, current_content.trim().to_string()));
                current_content.clear();
            }
            current_name = Some(slugify(heading.trim()));
        } else if current_name.is_some() {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }
    if let Some(name) = current_name {
        sections.push((name, current_content.trim().to_string()));
    }
    sections
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_was_underscore = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for cc in c.to_lowercase() {
                out.push(cc);
            }
            last_was_underscore = false;
        } else if !last_was_underscore && !out.is_empty() {
            out.push('_');
            last_was_underscore = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_spaces_and_punctuation() {
        assert_eq!(slugify("File System Access"), "file_system_access");
        assert_eq!(slugify("Returns & Warranty Policy"), "returns_warranty_policy");
        assert_eq!(slugify("Product Pricing & Fees"), "product_pricing_fees");
        assert_eq!(slugify("  Lots   of   spaces  "), "lots_of_spaces");
    }

    #[test]
    fn parse_sections_yields_slug_keyed_pairs() {
        let md = "# Title\n\npreamble ignored\n\n## First Section\n\nfirst content\nmore first\n\n## Second Section\n\nsecond content\n";
        let sections = parse_named_sections(md);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "first_section");
        assert_eq!(sections[0].1, "first content\nmore first");
        assert_eq!(sections[1].0, "second_section");
        assert_eq!(sections[1].1, "second content");
    }

    #[test]
    fn parse_sections_handles_empty_md() {
        assert!(parse_named_sections("").is_empty());
        assert!(parse_named_sections("# only h1\nbody\n").is_empty());
    }
}
