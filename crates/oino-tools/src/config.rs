//! Tool configuration and the per-call context core supplies (pi `ToolsOptions` +
//! `ExtensionContext` fields the built-ins read).

use std::fmt;
use std::sync::Arc;

use oino_ai::{ImageResize, InputModality, Model};

use crate::bash::BashDetachRegistry;
use crate::env::{ExecutionEnv, LocalEnv};
use crate::mutation_queue::FileMutationQueue;

/// What the `read` tool needs to know about the current model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelImageInfo {
    /// The model accepts image input (pi `model.input.includes("image")`).
    pub accepts_images: bool,
    /// The model's image resize policy (pi `model.inputLimits.images.resize`).
    pub resize: Option<ImageResize>,
}

impl From<&Model> for ModelImageInfo {
    fn from(model: &Model) -> Self {
        Self {
            accepts_images: model.input.contains(&InputModality::Image),
            resize: model
                .input_limits
                .as_ref()
                .and_then(|limits| limits.images.as_ref())
                .and_then(|images| images.resize.clone()),
        }
    }
}

/// Session values exported to shell tools (runtime §9.5, D12).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionEnv {
    /// Session id.
    pub session_id: String,
    /// Session file, when persisted.
    pub session_file: Option<String>,
    /// Provider of the selected model.
    pub provider: Option<String>,
    /// Selected model id (the virtual selection when one is shown).
    pub model: Option<String>,
    /// Reasoning level.
    pub reasoning_level: Option<String>,
}

/// Every variable [`SessionEnv`] may set; inherited copies are removed before spawning.
pub const SESSION_ENV_NAMES: [&str; 10] = [
    "OINO_SESSION_ID",
    "OINO_SESSION_FILE",
    "OINO_PROVIDER",
    "OINO_MODEL",
    "OINO_REASONING_LEVEL",
    "PI_SESSION_ID",
    "PI_SESSION_FILE",
    "PI_PROVIDER",
    "PI_MODEL",
    "PI_REASONING_LEVEL",
];

impl SessionEnv {
    /// `OINO_*` variables followed by their identical `PI_*` aliases.
    #[must_use]
    pub fn vars(&self) -> Vec<(String, String)> {
        let values = [
            ("SESSION_ID", Some(&self.session_id)),
            ("SESSION_FILE", self.session_file.as_ref()),
            ("PROVIDER", self.provider.as_ref()),
            ("MODEL", self.model.as_ref()),
            ("REASONING_LEVEL", self.reasoning_level.as_ref()),
        ];
        let mut vars = Vec::new();
        for prefix in ["OINO_", "PI_"] {
            for (name, value) in values {
                if let Some(value) = value {
                    vars.push((format!("{prefix}{name}"), value.clone()));
                }
            }
        }
        vars
    }
}

/// Live values read at call time (pi reads them from the `ExtensionContext`).
pub trait ToolContext: Send + Sync {
    /// Current working directory; `None` uses [`ToolConfig::cwd`] (pi `ctx?.cwd || cwd`).
    fn cwd(&self) -> Option<String> {
        None
    }

    /// Image capabilities of the current model; `None` when no model is selected.
    fn model_images(&self) -> Option<ModelImageInfo> {
        None
    }

    /// Session values for shell tools; `None` exports nothing.
    fn session_env(&self) -> Option<SessionEnv> {
        None
    }
}

/// A context without a session or model.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoContext;

impl ToolContext for NoContext {}

/// Image handling of `read` (settings `images.autoResize` + fallback resize profile).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageOptions {
    /// Resize images to inline limits (pi `autoResizeImages`, default `true`).
    pub auto_resize: bool,
    /// Resize profile when the model has none (pi `resizeOptions`; `None` = 2000×2000, 4.5MB).
    pub fallback_resize: Option<ImageResize>,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            auto_resize: true,
            fallback_resize: None,
        }
    }
}

/// Shell tool options (pi `BashToolOptions`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellOptions {
    /// Explicit shell (setting `shellPath`).
    pub shell_path: Option<String>,
    /// Prepended to every command as `{prefix}\n{command}` (setting `shellCommandPrefix`).
    pub command_prefix: Option<String>,
    /// Export [`SessionEnv`] and advertise it in the prompt guideline (default `true`).
    pub expose_session_environment: bool,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            shell_path: None,
            command_prefix: None,
            expose_session_environment: true,
        }
    }
}

/// Everything the built-in tools are constructed with.
#[derive(Clone)]
pub struct ToolConfig {
    /// Default working directory (absolute).
    pub cwd: String,
    /// Agent directory (`~/.oino`): temp files go to `<agent_dir>/tmp`, `<agent_dir>/bin` is
    /// prepended to `PATH`.
    pub agent_dir: String,
    /// File system and process host.
    pub env: Arc<dyn ExecutionEnv>,
    /// Live per-call values.
    pub context: Arc<dyn ToolContext>,
    /// `read` image handling.
    pub images: ImageOptions,
    /// `bash` / `powershell` options.
    pub shell: ShellOptions,
    /// Per-file serialisation of `write` / `edit`.
    pub mutation_queue: FileMutationQueue,
    /// Registry through which core detaches running `bash` calls.
    pub bash_detach: BashDetachRegistry,
    /// Base environment of spawned shells; `None` uses this process's environment.
    pub process_env: Option<Vec<(String, String)>>,
}

impl fmt::Debug for ToolConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolConfig")
            .field("cwd", &self.cwd)
            .field("agent_dir", &self.agent_dir)
            .field("images", &self.images)
            .field("shell", &self.shell)
            .finish_non_exhaustive()
    }
}

impl ToolConfig {
    /// Defaults: [`LocalEnv`], [`NoContext`], the global mutation queue, a fresh detach
    /// registry, the process environment.
    #[must_use]
    pub fn new(cwd: impl Into<String>, agent_dir: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            agent_dir: agent_dir.into(),
            env: Arc::new(LocalEnv::new()),
            context: Arc::new(NoContext),
            images: ImageOptions::default(),
            shell: ShellOptions::default(),
            mutation_queue: FileMutationQueue::global(),
            bash_detach: BashDetachRegistry::default(),
            process_env: None,
        }
    }

    /// The call's working directory (pi `ctx?.cwd || cwd`).
    #[must_use]
    pub fn effective_cwd(&self) -> String {
        self.context
            .cwd()
            .filter(|cwd| !cwd.is_empty())
            .unwrap_or_else(|| self.cwd.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_env_exports_both_prefixes() {
        let env = SessionEnv {
            session_id: "s".into(),
            model: Some("m".into()),
            ..SessionEnv::default()
        };
        assert_eq!(
            env.vars(),
            vec![
                ("OINO_SESSION_ID".into(), "s".into()),
                ("OINO_MODEL".into(), "m".into()),
                ("PI_SESSION_ID".into(), "s".into()),
                ("PI_MODEL".into(), "m".into()),
            ]
        );
    }
}
