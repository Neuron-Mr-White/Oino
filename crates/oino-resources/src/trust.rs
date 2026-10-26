//! Project trust (pi `project-trust.ts`, `trust-manager.ts`, runtime §9.7).

use std::path::{Path, PathBuf};

use oino_settings::paths::{CONFIG_DIR_NAME, TRUST_FILE, canonicalize_or_self, lexical_normalize};
use oino_settings::storage::{FileLock, write_atomic};
use oino_settings::{JsonObject, ProjectTrustDefault};
use serde_json::Value;

/// Project-local entries under `.oino/` that require trust (pi
/// `TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES`, without `mcp.json`: no MCP in core).
pub const PROTECTED_PROJECT_RESOURCES: [&str; 7] = [
    "settings.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
];

/// A saved trust decision: `Some(true|false)`, or `None` for "no decision".
pub type ProjectTrustDecision = Option<bool>;

/// A saved decision and the (canonical) directory it was saved for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustEntry {
    /// Canonical directory key in `trust.json`.
    pub path: PathBuf,
    /// Trusted or not.
    pub trusted: bool,
}

/// One edit of `trust.json` (`decision: None` deletes the key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustUpdate {
    /// Directory (canonicalised on write).
    pub path: PathBuf,
    /// New decision.
    pub decision: ProjectTrustDecision,
}

/// A selectable trust answer (pi `ProjectTrustOption`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustOption {
    /// Label shown to the user.
    pub label: String,
    /// Whether choosing it trusts the project for this session.
    pub trusted: bool,
    /// Edits to save when chosen (empty = this session only).
    pub updates: Vec<TrustUpdate>,
    /// Directory whose decision is saved, if any.
    pub saved_path: Option<PathBuf>,
}

/// `trust.json` failure (pi messages).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TrustStoreError {
    /// Unreadable or invalid JSON.
    #[error("Failed to read trust store {path}: {message}")]
    Read {
        /// Store path.
        path: PathBuf,
        /// Cause.
        message: String,
    },
    /// Top-level value is not an object.
    #[error("Invalid trust store {path}: expected an object")]
    NotAnObject {
        /// Store path.
        path: PathBuf,
    },
    /// A value is not `true`, `false` or `null`.
    #[error("Invalid trust store {path}: value for {key} must be true, false, or null")]
    InvalidValue {
        /// Store path.
        path: PathBuf,
        /// JSON-quoted key.
        key: String,
    },
    /// Lock or write failure.
    #[error("Failed to write trust store {path}: {message}")]
    Write {
        /// Store path.
        path: PathBuf,
        /// Cause.
        message: String,
    },
}

/// `<agent>/trust.json` (pi `ProjectTrustStore`): canonical directory → decision, locked access,
/// nearest-ancestor lookup.
#[derive(Clone, Debug)]
pub struct TrustStore {
    path: PathBuf,
}

fn normalize_dir(dir: &Path) -> PathBuf {
    let absolute =
        std::path::absolute(dir).map_or_else(|_| dir.to_path_buf(), |p| lexical_normalize(&p));
    canonicalize_or_self(&absolute)
}

impl TrustStore {
    /// The store in `agent_dir`.
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            path: agent_dir.join(TRUST_FILE),
        }
    }

    /// Store file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Nearest saved decision for `cwd` or an ancestor (pi `get`).
    ///
    /// # Errors
    ///
    /// [`TrustStoreError`] for an unreadable or invalid store.
    pub fn get(&self, cwd: &Path) -> Result<ProjectTrustDecision, TrustStoreError> {
        Ok(self.get_entry(cwd)?.map(|entry| entry.trusted))
    }

    /// Nearest saved entry (pi `getEntry`).
    ///
    /// # Errors
    ///
    /// See [`Self::get`].
    pub fn get_entry(&self, cwd: &Path) -> Result<Option<TrustEntry>, TrustStoreError> {
        let data = self.locked(|| self.read())?;
        let start = normalize_dir(cwd);
        Ok(start
            .ancestors()
            .find_map(|dir| match data.get(dir.to_string_lossy().as_ref()) {
                Some(Value::Bool(trusted)) => Some(TrustEntry {
                    path: dir.to_path_buf(),
                    trusted: *trusted,
                }),
                _ => None,
            }))
    }

    /// Save one decision (pi `set`).
    ///
    /// # Errors
    ///
    /// See [`Self::set_many`].
    pub fn set(&self, cwd: &Path, decision: ProjectTrustDecision) -> Result<(), TrustStoreError> {
        self.set_many(&[TrustUpdate {
            path: cwd.to_path_buf(),
            decision,
        }])
    }

    /// Apply several edits under one lock; keys are written sorted with a trailing newline.
    ///
    /// # Errors
    ///
    /// [`TrustStoreError`] for an invalid existing store or a write failure.
    pub fn set_many(&self, updates: &[TrustUpdate]) -> Result<(), TrustStoreError> {
        self.locked(|| {
            let mut data = self.read()?;
            for update in updates {
                let key = normalize_dir(&update.path).to_string_lossy().into_owned();
                match update.decision {
                    Some(trusted) => {
                        data.insert(key, Value::Bool(trusted));
                    }
                    None => {
                        data.shift_remove(&key);
                    }
                }
            }
            let mut keys: Vec<&String> = data.keys().collect();
            keys.sort();
            let sorted: JsonObject = keys
                .into_iter()
                .map(|key| (key.clone(), data[key].clone()))
                .collect();
            let text = serde_json::to_string_pretty(&Value::Object(sorted)).map_err(|e| {
                TrustStoreError::Write {
                    path: self.path.clone(),
                    message: e.to_string(),
                }
            })?;
            write_atomic(&self.path, format!("{text}\n").as_bytes()).map_err(|e| {
                TrustStoreError::Write {
                    path: self.path.clone(),
                    message: e.to_string(),
                }
            })
        })
    }

    fn locked<T>(
        &self,
        f: impl FnOnce() -> Result<T, TrustStoreError>,
    ) -> Result<T, TrustStoreError> {
        let _lock = FileLock::acquire(&self.path).map_err(|e| TrustStoreError::Write {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        f()
    }

    fn read(&self) -> Result<JsonObject, TrustStoreError> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(JsonObject::new());
            }
            Err(error) => {
                return Err(TrustStoreError::Read {
                    path: self.path.clone(),
                    message: error.to_string(),
                });
            }
        };
        let parsed: Value = serde_json::from_str(oino_settings::json::strip_bom(&content))
            .map_err(|error| TrustStoreError::Read {
                path: self.path.clone(),
                message: error.to_string(),
            })?;
        let Value::Object(data) = parsed else {
            return Err(TrustStoreError::NotAnObject {
                path: self.path.clone(),
            });
        };
        if let Some((key, _)) = data
            .iter()
            .find(|(_, value)| !matches!(value, Value::Bool(_) | Value::Null))
        {
            return Err(TrustStoreError::InvalidValue {
                path: self.path.clone(),
                key: Value::from(key.as_str()).to_string(),
            });
        }
        Ok(data)
    }
}

/// pi `hasTrustRequiringProjectResources`: protected entries under `<cwd>/.oino`, or an
/// `.agents/skills` dir in `cwd` or any ancestor other than `~/.agents/skills`.
#[must_use]
pub fn has_protected_resources(cwd: &Path, home: Option<&Path>) -> bool {
    let user_agents_skills = home.map(|home| normalize_dir(home).join(".agents").join("skills"));
    let start = normalize_dir(cwd);
    let config = start.join(CONFIG_DIR_NAME);
    if PROTECTED_PROJECT_RESOURCES
        .iter()
        .any(|entry| config.join(entry).exists())
    {
        return true;
    }
    start.ancestors().any(|dir| {
        let skills = dir.join(".agents").join("skills");
        Some(&skills) != user_agents_skills.as_ref() && skills.exists()
    })
}

/// Parent directory offered by "Trust parent folder" (pi `getProjectTrustParentPath`).
#[must_use]
pub fn trust_parent_path(cwd: &Path) -> Option<PathBuf> {
    normalize_dir(cwd).parent().map(Path::to_path_buf)
}

/// pi `getProjectTrustOptions`.
#[must_use]
pub fn trust_options(cwd: &Path, include_session_only: bool) -> Vec<TrustOption> {
    let trust_path = normalize_dir(cwd);
    let mut options = vec![TrustOption {
        label: "Trust".to_owned(),
        trusted: true,
        updates: vec![TrustUpdate {
            path: trust_path.clone(),
            decision: Some(true),
        }],
        saved_path: Some(trust_path.clone()),
    }];
    if let Some(parent) = trust_parent_path(cwd) {
        options.push(TrustOption {
            label: format!("Trust parent folder ({})", parent.display()),
            trusted: true,
            updates: vec![
                TrustUpdate {
                    path: parent.clone(),
                    decision: Some(true),
                },
                TrustUpdate {
                    path: trust_path.clone(),
                    decision: None,
                },
            ],
            saved_path: Some(parent),
        });
    }
    if include_session_only {
        options.push(TrustOption {
            label: "Trust (this session only)".to_owned(),
            trusted: true,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options.push(TrustOption {
        label: "Do not trust".to_owned(),
        trusted: false,
        updates: vec![TrustUpdate {
            path: trust_path.clone(),
            decision: Some(false),
        }],
        saved_path: Some(trust_path),
    });
    if include_session_only {
        options.push(TrustOption {
            label: "Do not trust (this session only)".to_owned(),
            trusted: false,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options
}

/// pi `formatProjectTrustPrompt` (product strings substituted).
#[must_use]
pub fn trust_prompt(cwd: &Path) -> String {
    format!(
        "Trust project folder?\n{}\n\nThis allows Oino to load {CONFIG_DIR_NAME} settings and resources, install missing project packages, and execute project extensions.",
        cwd.display()
    )
}

/// A `project_trust` extension handler's answer (pi `ProjectTrustEventResult`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensionTrustDecision {
    /// `trusted: "yes"`.
    pub trusted: bool,
    /// Save the decision to `trust.json`.
    pub remember: bool,
}

/// Inputs of [`resolve_project_trust`], in pi decision order.
#[derive(Clone, Debug)]
pub struct TrustRequest<'a> {
    /// Project directory.
    pub cwd: &'a Path,
    /// Home dir (to ignore `~/.agents/skills`).
    pub home: Option<&'a Path>,
    /// `-a` (`Some(true)`) / `-na` (`Some(false)`).
    pub cli_override: Option<bool>,
    /// Result of the `project_trust` extension event (emit it only when
    /// [`has_protected_resources`] is true).
    pub extension_decision: Option<ExtensionTrustDecision>,
    /// `defaultProjectTrust` from the global settings.
    pub default_trust: ProjectTrustDefault,
    /// Whether a UI can ask the user (interactive modes).
    pub has_ui: bool,
}

/// Why [`resolve_project_trust`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustSource {
    /// CLI `-a` / `-na`.
    Cli,
    /// The project has no protected resources.
    NoProtectedResources,
    /// A `project_trust` extension handler.
    Extension,
    /// A saved decision in `trust.json`.
    Saved,
    /// `defaultProjectTrust` = `always` / `never`.
    Default,
    /// `ask` without a UI.
    NonInteractive,
}

/// Outcome of [`resolve_project_trust`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustResolution {
    /// Decided without asking.
    Decided {
        /// Trusted or not.
        trusted: bool,
        /// Deciding rule.
        source: TrustSource,
    },
    /// Ask the user with [`trust_prompt`] and these options; save the chosen option's
    /// `updates` with [`TrustStore::set_many`]; a dismissed prompt means untrusted.
    Ask {
        /// Options including the session-only ones.
        options: Vec<TrustOption>,
    },
}

/// pi `resolveProjectTrusted` minus the UI: CLI → no protected resources → extension decision
/// (saved when `remember`) → nearest saved decision → `defaultProjectTrust` → ask (UI) or
/// untrusted.
///
/// # Errors
///
/// [`TrustStoreError`] when `trust.json` cannot be read or a remembered decision cannot be
/// written.
pub fn resolve_project_trust(
    request: &TrustRequest<'_>,
    store: &TrustStore,
) -> Result<TrustResolution, TrustStoreError> {
    let decided = |trusted, source| TrustResolution::Decided { trusted, source };
    if let Some(trusted) = request.cli_override {
        return Ok(decided(trusted, TrustSource::Cli));
    }
    if !has_protected_resources(request.cwd, request.home) {
        return Ok(decided(true, TrustSource::NoProtectedResources));
    }
    if let Some(extension) = request.extension_decision {
        if extension.remember {
            store.set(request.cwd, Some(extension.trusted))?;
        }
        return Ok(decided(extension.trusted, TrustSource::Extension));
    }
    if let Some(trusted) = store.get(request.cwd)? {
        return Ok(decided(trusted, TrustSource::Saved));
    }
    match request.default_trust {
        ProjectTrustDefault::Always => Ok(decided(true, TrustSource::Default)),
        ProjectTrustDefault::Never => Ok(decided(false, TrustSource::Default)),
        ProjectTrustDefault::Ask if !request.has_ui => {
            Ok(decided(false, TrustSource::NonInteractive))
        }
        ProjectTrustDefault::Ask => Ok(TrustResolution::Ask {
            options: trust_options(request.cwd, true),
        }),
    }
}
