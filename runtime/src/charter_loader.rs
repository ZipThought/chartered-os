//! Deployment-side Charter and Role context loading from disk.
//!
//! Reads files from the deployment's Charter directory and the
//! deployment's `.chartered/` directory, then hands the text to the
//! kernel's pure parsers (`chartered_core::parse_charter_def`,
//! `parse_role_context_def`). The kernel has no filesystem dependency;
//! substrate choice (local disk here, future daemon-distributed
//! Charter delivery elsewhere) is owned at this layer.
//!
//! Spec §The Charter, §Role Context, §Reference Charters.

use std::path::Path;

use chartered_core::{
    CharterDef, CharterLoadError, RoleContextDef, Skill, parse_charter_def, parse_role_context_def,
};

/// Load a Charter from a directory containing `frames.toml`,
/// `scopes.md`, and `behavioral_spec.md`. Returns `CharterDef` (data
/// only); use `chartered_core::build_charter` to attach Evaluators.
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

    parse_charter_def(
        &frames_text,
        &scopes_text,
        &behavioral_spec,
        &frames_path.display().to_string(),
    )
}

/// Load Role context from a markdown file (same `## Heading` →
/// slugified-name + content convention as `scopes.md`).
pub fn load_role_context_def(path: &Path) -> Result<RoleContextDef, CharterLoadError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CharterLoadError(format!("reading {}: {e}", path.display())))?;
    Ok(parse_role_context_def(&text))
}

/// Load Skills from `<charter_dir>/skills/*.md`. Each `.md` file is one
/// Skill: id = filename stem, content = file body. Missing directory
/// returns an empty Vec (Skills are optional — the rest of the Charter
/// loads regardless). Spec §Skills: Actor-side cognition instrumentation.
pub fn load_skills(charter_dir: &Path) -> Result<Vec<Skill>, CharterLoadError> {
    let skills_dir = charter_dir.join("skills");
    if !skills_dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(&skills_dir).map_err(|e| {
        CharterLoadError(format!("reading {}: {e}", skills_dir.display()))
    })?;
    let mut skills = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            CharterLoadError(format!("read_dir entry in {}: {e}", skills_dir.display()))
        })?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let content = std::fs::read_to_string(&path).map_err(|e| {
            CharterLoadError(format!("reading {}: {e}", path.display()))
        })?;
        skills.push(Skill::new(id, content));
    }
    // Stable order across loads — sort by id so skills_content_hash is
    // deterministic without relying on filesystem iteration order.
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
}
