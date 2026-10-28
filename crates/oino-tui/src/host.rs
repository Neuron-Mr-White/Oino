//! Frontend services the `SessionBackend` does not offer yet.
//!
//! Session listing (`/resume`), settings reads/writes (`/settings`), the keybindings file,
//! `@` path completion, clipboard images and renderer calls go through [`TuiHost`]. The
//! binary passes a [`LocalHost`] (feature `local-host`) for in-process sessions; once the
//! matching `_oino/*` commands exist (modes.md §4: `list_sessions`, `get_settings`,
//! `set_setting`, `get_keybindings`, `render`) a backend-driven host replaces it, which also
//! makes `--remote` complete.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use oino_ai::{ImageContent, JsonObject, JsonValue};
use oino_view::sessions::{Scope, SessionItem};

/// Keybinding overrides from `keybindings.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeybindingOverrides {
    /// Action id → key ids.
    pub actions: IndexMap<String, Vec<String>>,
    /// Extension shortcut id → key ids (`extensionShortcuts`).
    pub extension_shortcuts: IndexMap<String, Vec<String>>,
}

/// Lazy render request for custom messages/entries and tools (ui-protocol §6.4).
#[derive(Clone, Debug, PartialEq)]
pub struct RenderRequest {
    /// `customMessage` | `customEntry` | `tool`.
    pub target: &'static str,
    /// Custom type or tool name (built-in tool names included).
    pub key: String,
    /// Entry or tool-call id.
    pub entry_id: Option<String>,
    /// `simple` | `regular` | `advanced`.
    pub density: &'static str,
    /// Expanded state.
    pub expanded: bool,
    /// `narrow` | `normal` | `wide`.
    pub width: &'static str,
    /// Payload.
    pub data: JsonValue,
}

impl RenderRequest {
    /// Density from the expanded state (`advanced` when expanded).
    #[must_use]
    pub const fn density_for(expanded: bool) -> &'static str {
        if expanded { "advanced" } else { "simple" }
    }

    /// Width class of `columns` cells.
    #[must_use]
    pub const fn width_for(columns: usize) -> &'static str {
        if columns < 60 {
            "narrow"
        } else if columns >= 100 {
            "wide"
        } else {
            "normal"
        }
    }
}

/// Services for the terminal client. Calls may block; the runner calls them off the UI
/// thread where it matters.
pub trait TuiHost: Send + Sync {
    /// Session working directory.
    fn cwd(&self) -> PathBuf;
    /// Home directory (for `~` display).
    fn home(&self) -> Option<PathBuf>;
    /// Keybinding overrides.
    fn keybindings(&self) -> KeybindingOverrides {
        KeybindingOverrides::default()
    }
    /// Saved sessions, newest first.
    ///
    /// # Errors
    ///
    /// Listing failures (shown in the picker).
    fn list_sessions(&self, scope: Scope) -> Result<Vec<SessionItem>, String> {
        let _ = scope;
        Ok(Vec::new())
    }
    /// Deletes a session file.
    ///
    /// # Errors
    ///
    /// I/O failure text.
    fn delete_session(&self, path: &str) -> Result<(), String> {
        let _ = path;
        Err("Deleting sessions is not supported by this backend".to_owned())
    }
    /// Effective (merged) settings.
    fn settings(&self) -> JsonObject {
        JsonObject::new()
    }
    /// Writes a global setting (`None` deletes it).
    ///
    /// # Errors
    ///
    /// Settings write failure text.
    fn set_setting(&self, path: &[&str], value: Option<JsonValue>) -> Result<(), String> {
        let _ = (path, value);
        Err("Settings are read-only for this backend".to_owned())
    }
    /// `@` completion candidates for `prefix` (paths relative to the cwd).
    fn complete_path(&self, prefix: &str) -> Vec<String> {
        complete_path_in(&self.cwd(), prefix)
    }
    /// Image currently on the clipboard.
    fn clipboard_image(&self) -> Option<ImageContent> {
        None
    }
    /// Text currently on the clipboard.
    fn clipboard_text(&self) -> Option<String> {
        None
    }
    /// Current git branch of the cwd.
    fn git_branch(&self) -> Option<String> {
        git_branch(&self.cwd())
    }
    /// Renders a custom message/entry/tool through its extension renderer.
    fn render(&self, request: &RenderRequest) -> Option<JsonValue> {
        let _ = request;
        None
    }
    /// Changelog markdown for `/changelog`.
    fn changelog(&self) -> Option<String> {
        None
    }
}

/// `@path` completion over the filesystem (pi uses `fd`; this lists one directory level,
/// hiding dot-entries unless the prefix starts with a dot).
#[must_use]
pub fn complete_path_in(cwd: &Path, prefix: &str) -> Vec<String> {
    let (dir_part, name_part) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    let dir = if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else if let Some(rest) = dir_part.strip_prefix("~/") {
        std::env::var_os("HOME").map_or_else(|| cwd.join(dir_part), |h| PathBuf::from(h).join(rest))
    } else {
        cwd.join(dir_part)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let lower = name_part.to_lowercase();
    let mut out: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !name_part.starts_with('.') {
                return None;
            }
            if !name.to_lowercase().starts_with(&lower) {
                return None;
            }
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            Some(format!("{dir_part}{name}{}", if is_dir { "/" } else { "" }))
        })
        .collect();
    out.sort();
    out.truncate(50);
    out
}

/// Reads `.git/HEAD` upwards from `cwd` (pi footer branch).
#[must_use]
pub fn git_branch(cwd: &Path) -> Option<String> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let git = d.join(".git");
        let head_path = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            let text = std::fs::read_to_string(&git).ok()?;
            let gitdir = text.trim().strip_prefix("gitdir: ")?;
            let base = if Path::new(gitdir).is_absolute() {
                PathBuf::from(gitdir)
            } else {
                d.join(gitdir)
            };
            base.join("HEAD")
        } else {
            dir = d.parent();
            continue;
        };
        let head = std::fs::read_to_string(head_path).ok()?;
        let head = head.trim();
        return Some(
            head.strip_prefix("ref: refs/heads/")
                .map_or_else(|| "detached".to_owned(), str::to_owned),
        );
    }
    None
}

/// Runs a clipboard helper and returns its stdout.
fn clipboard_command(candidates: &[(&str, &[&str])]) -> Option<Vec<u8>> {
    for (program, args) in candidates {
        if let Ok(output) = std::process::Command::new(program).args(*args).output()
            && output.status.success()
            && !output.stdout.is_empty()
        {
            return Some(output.stdout);
        }
    }
    None
}

#[cfg(feature = "local-host")]
pub use local::LocalHost;

#[cfg(feature = "local-host")]
mod local {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, PoisonError};

    use oino_client::{FrontendApi, SessionBackend};
    use std::time::UNIX_EPOCH;

    use base64::Engine;
    use oino_ai::{ImageContent, JsonObject, JsonValue};
    use oino_session::{SessionInfo, SessionManager};
    use oino_settings::{SettingsManager, SettingsScope};
    use oino_view::sessions::{Scope, SessionItem};

    use super::{KeybindingOverrides, RenderRequest, TuiHost, clipboard_command};

    /// [`TuiHost`] over the local agent directory.
    pub struct LocalHost {
        cwd: PathBuf,
        agent_dir: PathBuf,
        sessions_root: PathBuf,
        settings: Mutex<SettingsManager>,
        /// Backend the host asks for extension render specs (ui-protocol §6.4).
        render: Option<Arc<dyn SessionBackend>>,
    }

    impl std::fmt::Debug for LocalHost {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LocalHost")
                .field("cwd", &self.cwd)
                .field("agent_dir", &self.agent_dir)
                .finish_non_exhaustive()
        }
    }

    fn item(info: SessionInfo) -> SessionItem {
        let modified_ms = info
            .modified
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        SessionItem {
            path: info.path.display().to_string(),
            id: info.id,
            cwd: info.cwd,
            name: info.name,
            parent_path: info.parent_session_path.map(|p| p.display().to_string()),
            modified_ms,
            message_count: info.message_count,
            first_message: info.first_message,
            all_text: info.all_messages_text,
        }
    }

    impl LocalHost {
        /// Host for `cwd` with `agent_dir`; `sessions_root` defaults to `<agent>/sessions`.
        #[must_use]
        pub fn new(
            cwd: &Path,
            agent_dir: &Path,
            sessions_root: Option<PathBuf>,
            project_trusted: bool,
        ) -> Self {
            Self {
                cwd: cwd.to_path_buf(),
                agent_dir: agent_dir.to_path_buf(),
                sessions_root: sessions_root.unwrap_or_else(|| agent_dir.join("sessions")),
                settings: Mutex::new(SettingsManager::create(cwd, agent_dir, project_trusted)),
                render: None,
            }
        }

        /// Asks `backend` for extension render specs (built-in tool rows included, U1–U2).
        #[must_use]
        pub fn with_render(mut self, backend: Arc<dyn SessionBackend>) -> Self {
            self.render = Some(backend);
            self
        }
    }

    impl TuiHost for LocalHost {
        fn cwd(&self) -> PathBuf {
            self.cwd.clone()
        }

        fn home(&self) -> Option<PathBuf> {
            oino_settings::paths::home_dir()
        }

        fn keybindings(&self) -> KeybindingOverrides {
            let file = oino_resources::keybindings::KeybindingsFile::load(&self.agent_dir);
            let mut out = KeybindingOverrides::default();
            for (id, binding) in file.bindings {
                out.actions
                    .insert(id, binding.keys().into_iter().map(str::to_owned).collect());
            }
            // `extensionShortcuts` is an Oino addition read straight from the file.
            if let Ok(text) = std::fs::read_to_string(self.agent_dir.join("keybindings.json"))
                && let Ok(JsonValue::Object(raw)) = serde_json::from_str::<JsonValue>(&text)
                && let Some(JsonValue::Object(shortcuts)) = raw.get("extensionShortcuts")
            {
                for (id, keys) in shortcuts {
                    let keys = match keys {
                        JsonValue::String(k) => vec![k.clone()],
                        JsonValue::Array(ks) => ks
                            .iter()
                            .filter_map(|k| k.as_str().map(str::to_owned))
                            .collect(),
                        _ => continue,
                    };
                    out.extension_shortcuts.insert(id.clone(), keys);
                }
                out.actions.shift_remove("extensionShortcuts");
            }
            out
        }

        fn list_sessions(&self, scope: Scope) -> Result<Vec<SessionItem>, String> {
            let infos = match scope {
                Scope::Current => SessionManager::list(&self.cwd, &self.sessions_root, None, None),
                Scope::All => SessionManager::list_all(&self.sessions_root, None, None),
            }
            .map_err(|e| e.to_string())?;
            Ok(infos.into_iter().map(item).collect())
        }

        fn delete_session(&self, path: &str) -> Result<(), String> {
            let path = Path::new(path);
            if !path.starts_with(&self.sessions_root) {
                return Err("Refusing to delete a file outside the sessions directory".into());
            }
            std::fs::remove_file(path).map_err(|e| e.to_string())
        }

        fn settings(&self) -> JsonObject {
            let mut manager = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
            manager.reload();
            manager.settings().merged().clone()
        }

        fn set_setting(&self, path: &[&str], value: Option<JsonValue>) -> Result<(), String> {
            let mut manager = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
            manager
                .set(SettingsScope::Global, path, value)
                .map_err(|e| e.to_string())
        }

        fn render(&self, request: &RenderRequest) -> Option<JsonValue> {
            let backend = self.render.as_ref()?;
            let row = oino_client::RenderRow {
                target: request.target.to_owned(),
                key: request.key.clone(),
                entry_id: request.entry_id.clone(),
                density: Some(request.density.to_owned()),
                expanded: request.expanded,
                width: Some(request.width.to_owned()),
                data: request.data.clone(),
            };
            let handle = tokio::runtime::Handle::try_current().ok()?;
            if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
                return None;
            }
            tokio::task::block_in_place(|| handle.block_on(backend.render(row)))
                .ok()
                .flatten()
        }

        fn clipboard_image(&self) -> Option<ImageContent> {
            let bytes = clipboard_command(&[
                ("wl-paste", &["--no-newline", "--type", "image/png"]),
                (
                    "xclip",
                    &["-selection", "clipboard", "-t", "image/png", "-o"],
                ),
                ("pngpaste", &["-"]),
            ])?;
            bytes.starts_with(b"\x89PNG").then(|| ImageContent {
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                mime_type: "image/png".to_owned(),
            })
        }

        fn clipboard_text(&self) -> Option<String> {
            let bytes = clipboard_command(&[
                ("wl-paste", &["--no-newline"]),
                ("xclip", &["-selection", "clipboard", "-o"]),
                ("pbpaste", &[]),
            ])?;
            String::from_utf8(bytes).ok()
        }

        fn changelog(&self) -> Option<String> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_completion_lists_matching_entries() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let found = complete_path_in(&root, "sr");
        assert_eq!(found, vec!["src/".to_owned()]);
        let nested = complete_path_in(&root, "src/ho");
        assert_eq!(nested, vec!["src/host.rs".to_owned()]);
        assert!(git_branch(&root).is_some() || git_branch(&root).is_none());
    }
}
