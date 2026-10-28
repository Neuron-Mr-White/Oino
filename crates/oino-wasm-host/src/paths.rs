//! Extension directories (extension-api §5.7) and workspace identity (§5.8, D10).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use oino_extension_api::manifest::{FsAccess, Permissions};
use oino_extension_api::ops::WorkspaceInfo;
use oino_extension_api::{ExtensionId, HostError};
use sha2::{Digest, Sha256};

/// Host directories of one extension.
#[derive(Clone, Debug)]
pub(crate) struct ExtPaths {
    pub(crate) agent_dir: PathBuf,
    pub(crate) global: PathBuf,
    pub(crate) workspace_root: PathBuf,
    pub(crate) session_root: PathBuf,
    /// Read-only view of [`AGENT_FILES`] exposed as `/agent` (outside every preopen).
    pub(crate) agent_view: PathBuf,
    pub(crate) tmp: PathBuf,
    pub(crate) cache: PathBuf,
    pub(crate) logs: PathBuf,
    pub(crate) log_file: PathBuf,
}

impl ExtPaths {
    pub(crate) fn new(agent_dir: &Path, id: &ExtensionId) -> Self {
        let state = agent_dir.join("state").join(id.as_str());
        Self {
            agent_dir: agent_dir.to_path_buf(),
            global: state.join("global"),
            workspace_root: state.join("workspace"),
            session_root: state.join("session"),
            agent_view: state.join("agent-view"),
            tmp: agent_dir.join("tmp").join("ext").join(id.as_str()),
            cache: agent_dir.join("cache").join("ext").join(id.as_str()),
            logs: agent_dir.join("logs").join("ext").join(id.as_str()),
            log_file: agent_dir.join("logs").join("ext").join(format!("{id}.log")),
        }
    }

    /// Creates every always-on directory.
    pub(crate) fn create(&self) -> std::io::Result<()> {
        for dir in [
            &self.global,
            &self.workspace_root,
            &self.session_root,
            &self.agent_view,
            &self.tmp,
            &self.cache,
            &self.logs,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// `state/<id>/workspace/<wsid>`.
    pub(crate) fn workspace(&self, wsid: &str) -> PathBuf {
        self.workspace_root.join(wsid)
    }

    /// `state/<id>/session/<sid>`.
    pub(crate) fn session(&self, sid: &str) -> PathBuf {
        self.session_root.join(sanitize(sid))
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// One named preopen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Preopen {
    pub(crate) name: String,
    pub(crate) host: PathBuf,
    pub(crate) writable: bool,
}

impl Preopen {
    pub(crate) fn guest_path(&self) -> String {
        format!("/{}", self.name)
    }
}

/// Agent-dir subdirectories exposed by `fs.agentDir: read` (never `auth.json`/`secrets.json`).
pub(crate) const AGENT_SUBDIRS: &[&str] = &[
    "sessions",
    "skills",
    "prompts",
    "themes",
    "extensions",
    "bundled",
];

/// Agent-dir files exposed read-only as `/agent/<name>` (pi `mcp.json`-style config, R1).
pub(crate) const AGENT_FILES: &[&str] = &["mcp.json"];

// A live read-only view of `AGENT_FILES` (`/agent`): a directory of links to the agent-dir
// files, so guests read the *current* file through an ordinary preopen while `auth.json` /
// `secrets.json` stay out of reach. The view lives in `state/<id>/agent-view`, which no guest
// can write, so a link can never be swapped for an escape.

#[cfg(unix)]
fn link_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

#[cfg(not(unix))]
fn link_file(from: &Path, to: &Path) -> std::io::Result<()> {
    // Windows symlinks need a privilege or Developer Mode; a hard link works everywhere on the
    // same volume and is refreshed whenever the instance is created.
    std::fs::hard_link(from, to)
}

/// Recreates the `/agent` view so a replaced `mcp.json` is visible without a restart.
fn refresh_agent_view(paths: &ExtPaths) -> std::io::Result<PathBuf> {
    let view = paths.agent_view.clone();
    std::fs::create_dir_all(&view)?;
    for name in AGENT_FILES {
        let source = paths.agent_dir.join(name);
        let target = view.join(name);
        let _ = std::fs::remove_file(&target);
        if !source.is_file() {
            continue;
        }
        if link_file(&source, &target).is_err() {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(view)
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

/// Every preopen granted to an extension.
pub(crate) fn preopens(paths: &ExtPaths, perms: &Permissions, cwd: &Path) -> Vec<Preopen> {
    let rw = |name: &str, host: &Path| Preopen {
        name: name.to_owned(),
        host: host.to_path_buf(),
        writable: true,
    };
    let mut out = vec![
        rw("ext-global", &paths.global),
        rw("ext-workspace", &paths.workspace_root),
        rw("ext-session", &paths.session_root),
        rw("tmp", &paths.tmp),
        rw("cache", &paths.cache),
        rw("logs", &paths.logs),
    ];
    if perms.fs.project != FsAccess::None {
        out.push(Preopen {
            name: "project".into(),
            host: cwd.to_path_buf(),
            writable: perms.fs.project == FsAccess::Write,
        });
    }
    if perms.fs.agent_dir != FsAccess::None {
        for sub in AGENT_SUBDIRS {
            let host = paths.agent_dir.join(sub);
            if host.is_dir() {
                out.push(Preopen {
                    name: format!("agent/{sub}"),
                    host,
                    writable: false,
                });
            }
        }
        if let Ok(view) = refresh_agent_view(paths) {
            out.push(Preopen {
                name: "agent".into(),
                host: view,
                writable: false,
            });
        }
    }
    for grant in &perms.fs.paths {
        if grant.access == FsAccess::None {
            continue;
        }
        let host = expand_home(&grant.path);
        if host.is_dir() {
            out.push(Preopen {
                name: grant.alias.trim_matches('/').to_owned(),
                host,
                writable: grant.access == FsAccess::Write,
            });
        }
    }
    out
}

/// Resolves a guest path (`/<preopen>/rest`) to a host path; `write` requires a writable
/// preopen. `..` components are rejected.
pub(crate) fn resolve(
    preopens: &[Preopen],
    guest: &str,
    write: bool,
) -> Result<PathBuf, HostError> {
    let trimmed = guest.trim_start_matches('/');
    let best = preopens
        .iter()
        .filter(|p| trimmed == p.name || trimmed.starts_with(&format!("{}/", p.name)))
        .max_by_key(|p| p.name.len())
        .ok_or_else(|| HostError::Denied(format!("fs: `{guest}` is outside every preopen")))?;
    let rest = trimmed[best.name.len()..].trim_start_matches('/');
    let rel = Path::new(rest);
    if rel
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(HostError::InvalidArgument(format!(
            "fs: `{guest}` must be a normalized path"
        )));
    }
    if write && !best.writable {
        return Err(HostError::Denied(format!("fs write: /{}", best.name)));
    }
    Ok(best.host.join(rel))
}

/// Project root: nearest ancestor with `.git`, else `cwd`.
pub(crate) fn git_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

fn git_branch(root: &Path) -> Option<String> {
    let head = std::fs::read_to_string(root.join(".git").join("HEAD")).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(ToOwned::to_owned)
}

/// D10 workspace id: first 16 hex chars of sha256(canonical project root).
#[must_use]
pub fn workspace_id(root: &Path) -> String {
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    let mut out = String::with_capacity(16);
    for b in &digest[..8] {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Workspace info for `cwd`.
pub(crate) fn workspace_info(cwd: &Path) -> WorkspaceInfo {
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let git = git_root(&canonical);
    let root = git.clone().unwrap_or_else(|| canonical.clone());
    WorkspaceInfo {
        id: workspace_id(&root),
        root: root.to_string_lossy().into_owned(),
        git_branch: git.as_deref().and_then(git_branch),
        git_root: git.map(|g| g.to_string_lossy().into_owned()),
        cwd: cwd.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_preopens() {
        let pre = vec![
            Preopen {
                name: "ext-global".into(),
                host: "/s/g".into(),
                writable: true,
            },
            Preopen {
                name: "project".into(),
                host: "/p".into(),
                writable: false,
            },
            Preopen {
                name: "agent/sessions".into(),
                host: "/a/s".into(),
                writable: false,
            },
        ];
        assert_eq!(
            resolve(&pre, "/ext-global/a/b.json", true).unwrap(),
            PathBuf::from("/s/g/a/b.json")
        );
        assert_eq!(
            resolve(&pre, "/project/x", false).unwrap(),
            PathBuf::from("/p/x")
        );
        assert!(matches!(
            resolve(&pre, "/project/x", true),
            Err(HostError::Denied(_))
        ));
        assert!(matches!(
            resolve(&pre, "/etc/passwd", false),
            Err(HostError::Denied(_))
        ));
        assert!(matches!(
            resolve(&pre, "/project/../etc", false),
            Err(HostError::InvalidArgument(_))
        ));
        assert_eq!(
            resolve(&pre, "/agent/sessions/x", false).unwrap(),
            PathBuf::from("/a/s/x")
        );
    }

    /// `fs.agentDir: read` exposes the agent-dir config files as `/agent/<name>` (pi `mcp.json`)
    /// and never the secrets next to them (R1).
    #[test]
    fn agent_view_exposes_config_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        std::fs::write(agent_dir.join("auth.json"), "{}").unwrap();
        std::fs::write(agent_dir.join("secrets.json"), "{}").unwrap();
        let paths = ExtPaths::new(&agent_dir, &ExtensionId::new("acme.ext").unwrap());
        paths.create().unwrap();
        let perms = Permissions {
            fs: oino_extension_api::manifest::FsPermission {
                agent_dir: FsAccess::Read,
                ..oino_extension_api::manifest::FsPermission::default()
            },
            ..Permissions::default()
        };
        let pre = preopens(&paths, &perms, dir.path());
        let view = pre.iter().find(|p| p.name == "agent").expect("/agent");
        assert!(!view.writable);
        assert!(
            !view.host.starts_with(&paths.cache),
            "view outside writable preopens"
        );
        assert_eq!(
            resolve(&pre, "/agent/mcp.json", false).unwrap(),
            view.host.join("mcp.json")
        );
        assert!(
            std::fs::read_to_string(resolve(&pre, "/agent/mcp.json", false).unwrap())
                .unwrap()
                .contains("mcpServers")
        );
        // Only the allow-listed names are linked, so the secrets are not reachable.
        assert!(!resolve(&pre, "/agent/auth.json", false).unwrap().exists());
        assert!(
            !resolve(&pre, "/agent/secrets.json", false)
                .unwrap()
                .exists()
        );
        // The view follows a replaced file (atomic rename) at the next preopen refresh.
        std::fs::remove_file(agent_dir.join("mcp.json")).unwrap();
        std::fs::write(agent_dir.join("mcp.json"), r#"{"mcpServers":{"x":{}}}"#).unwrap();
        let pre = preopens(&paths, &perms, dir.path());
        assert!(
            std::fs::read_to_string(resolve(&pre, "/agent/mcp.json", false).unwrap())
                .unwrap()
                .contains("\"x\"")
        );
    }

    #[test]
    fn wsid_is_16_hex() {
        let id = workspace_id(Path::new("/home/u/project"));
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, workspace_id(Path::new("/home/u/project")));
    }
}
