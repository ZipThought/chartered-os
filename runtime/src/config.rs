//! `.chartered/` deployment-config loading. Spec §The Runtime.
//!
//! One config schema; one code path. Test deployments differ from
//! production deployments only in the `backend` value of the per-role
//! tables (`fake` vs `openai`/`anthropic`/...). See feedback memory:
//! "Single code path across test and production — at every level."

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chartered_core::{
    build_charter, build_role_context, CharterDef, CharterLoadError, GovernanceMode,
    RoleContextDef, Skill,
};
use serde::Deserialize;

use crate::charter_loader::{load_charter_def, load_role_context_def, load_skills};

#[derive(Debug, Clone)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl From<CharterLoadError> for ConfigError {
    fn from(e: CharterLoadError) -> Self {
        ConfigError(e.0)
    }
}

/// Per-role CognitionBackend selector. Strong-typed so unknown values
/// are rejected at TOML-parse time. Adding a new backend (Anthropic,
/// vLLM, …) is one variant here + one arm in `runtime::run::build_backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Fake,
    #[serde(rename = "openai")]
    OpenAi,
    Gemini,
}

/// Per-deployment runtime configuration. The governance mode is the
/// 2x2 of (grounding, evaluation) per spec §The Runtime > Governance
/// mode. Defaults to FULL (both on) when the field is absent.
#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub governance: GovernanceConfig,
}

/// Toggle pair for the four named modes: full, grounding-only,
/// evaluation-only, neither.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct GovernanceConfig {
    #[serde(default = "default_true")]
    pub grounding: bool,
    #[serde(default = "default_true")]
    pub evaluation: bool,
}

fn default_true() -> bool {
    true
}

impl Default for GovernanceConfig {
    fn default() -> Self {
        Self {
            grounding: true,
            evaluation: true,
        }
    }
}

impl From<GovernanceConfig> for GovernanceMode {
    fn from(cfg: GovernanceConfig) -> Self {
        GovernanceMode::new(cfg.grounding, cfg.evaluation)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StewardConfig {
    pub actor: ActorConfig,
    pub evaluator: EvaluatorConfig,
    #[serde(default)]
    pub tester: Option<TesterConfig>,
    #[serde(default)]
    pub judge: Option<JudgeConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActorConfig {
    pub backend: BackendKind,
    #[serde(default)]
    pub model: Option<String>,
    /// Required when `backend = "fake"`. One LLM response per loop step.
    #[serde(default)]
    pub fake_responses: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvaluatorConfig {
    pub backend: BackendKind,
    #[serde(default)]
    pub model: Option<String>,
    /// Required when `backend = "fake"`. Per-Frame response queue,
    /// keyed by Frame id.
    #[serde(default)]
    pub fake_responses: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TesterConfig {
    pub backend: BackendKind,
    pub brief: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Required when `backend = "fake"`. One LLM response per turn.
    #[serde(default)]
    pub fake_responses: Vec<String>,
    /// Number of Tester turns to run. Required when [tester] is
    /// configured (otherwise --user-message + max_turns=1).
    pub max_turns: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JudgeConfig {
    pub backend: BackendKind,
    pub criteria: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Required when `backend = "fake"`. Single response.
    #[serde(default)]
    pub fake_response: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CharterRef {
    pub path: String,
    pub version: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolRegistration {
    pub id: String,
    pub executor: String,
}

pub struct DeploymentConfig {
    pub chartered_dir: PathBuf,
    pub runtime: RuntimeConfig,
    pub steward: StewardConfig,
    pub charter_ref: CharterRef,
    pub charter_def: CharterDef,
    pub role_context_def: Option<RoleContextDef>,
    pub tools: Vec<ToolRegistration>,
    pub role_context_version: u64,
    /// Skills loaded from `<charter_dir>/skills/*.md`. Empty when the
    /// directory is absent. Spec §Skills.
    pub skills: Vec<Skill>,
}

impl DeploymentConfig {
    /// Build Charter, Role context, and the Skills bound to the Snapshot.
    /// Returns a tuple so the caller can compose a `Snapshot::new`.
    /// Takes `&self` because the Agent reuses the same DeploymentConfig
    /// across many run() calls; the underlying defs are Clone-able and
    /// inexpensive to rebuild per run.
    pub fn build_charter<F>(
        &self,
        evaluator_factory: F,
    ) -> (
        chartered_core::Charter,
        chartered_core::RoleContext,
        Vec<Skill>,
    )
    where
        F: FnMut(&chartered_core::FrameDef) -> std::sync::Arc<dyn chartered_core::Evaluator>,
    {
        let charter = build_charter(
            self.charter_def.clone(),
            self.charter_ref.version,
            evaluator_factory,
        );
        let role_context = match &self.role_context_def {
            Some(def) => build_role_context(def.clone(), self.role_context_version),
            None => chartered_core::RoleContext::empty(),
        };
        (charter, role_context, self.skills.clone())
    }
}

pub fn find_chartered_dir(start: &Path) -> Option<PathBuf> {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut dir = start;
    loop {
        let candidate = dir.join(".chartered");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = PathBuf::from(home).join(".chartered");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

pub fn load(chartered_dir: &Path) -> Result<DeploymentConfig, ConfigError> {
    if !chartered_dir.is_dir() {
        return Err(ConfigError(format!(
            "{} is not a directory",
            chartered_dir.display()
        )));
    }

    let runtime: RuntimeConfig = load_toml(&chartered_dir.join("chartered.toml"))?;
    let steward: StewardConfig = load_toml(&chartered_dir.join("steward.toml"))?;
    let charter_ref: CharterRef = load_toml(&chartered_dir.join("charter.toml"))?;

    let charter_dir = resolve_charter_path(chartered_dir, &charter_ref.path)?;
    let charter_def = load_charter_def(&charter_dir)?;
    let skills = load_skills(&charter_dir).map_err(|e| ConfigError(e.to_string()))?;

    let role_context_path = chartered_dir.join("role_context.md");
    let role_context_def = if role_context_path.is_file() {
        Some(load_role_context_def(&role_context_path)?)
    } else {
        None
    };
    // RoleContext version source isn't yet wired (no TOML field, no env);
    // every loaded role context gets version 1 until a source is defined.
    let role_context_version: u64 = 1;

    let tools = load_tools_dir(&chartered_dir.join("tools"))?;

    Ok(DeploymentConfig {
        chartered_dir: chartered_dir.to_path_buf(),
        runtime,
        steward,
        charter_ref,
        charter_def,
        role_context_def,
        tools,
        role_context_version,
        skills,
    })
}

fn resolve_charter_path(chartered_dir: &Path, raw: &str) -> Result<PathBuf, ConfigError> {
    let p = Path::new(raw);
    let resolved = if p.is_absolute() {
        p.to_path_buf()
    } else {
        chartered_dir.join(p)
    };
    resolved.canonicalize().map_err(|e| {
        ConfigError(format!(
            "resolving Charter path {}: {e}",
            resolved.display()
        ))
    })
}

fn load_toml<T: for<'de> serde::Deserialize<'de>>(path: &Path) -> Result<T, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("reading {}: {e}", path.display())))?;
    toml::from_str(&text).map_err(|e| ConfigError(format!("parsing {}: {e}", path.display())))
}

fn load_tools_dir(tools_dir: &Path) -> Result<Vec<ToolRegistration>, ConfigError> {
    if !tools_dir.is_dir() {
        return Ok(vec![]);
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(tools_dir)
        .map_err(|e| ConfigError(format!("reading {}: {e}", tools_dir.display())))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    entries.sort();
    let mut tools = Vec::with_capacity(entries.len());
    for path in entries {
        let reg: ToolRegistration = load_toml(&path)?;
        tools.push(reg);
    }
    Ok(tools)
}
