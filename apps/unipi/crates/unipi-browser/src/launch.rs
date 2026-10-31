//! Browser process launch (browser-control.md §4.4–4.5): flags, display, CDP transport,
//! process-group cleanup.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::cdp::{Cdp, Transport};
use crate::display::Display;
use crate::engine::{Engine, EngineKind};

/// Where the browser draws.
#[derive(Clone, Debug)]
pub(crate) enum Placement {
    /// `--headless=new`.
    Headless,
    /// Headed on a private Xvfb display.
    Hidden(Arc<Display>),
    /// A real window.
    Window,
}

impl Placement {
    /// Wire name.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Hidden(_) => "hidden",
            Self::Window => "window",
        }
    }
}

/// What to launch.
#[derive(Clone, Debug)]
pub(crate) struct LaunchSpec {
    /// Engine.
    pub(crate) engine: Engine,
    /// `--user-data-dir`.
    pub(crate) user_data_dir: PathBuf,
    /// Placement.
    pub(crate) placement: Placement,
    /// Pipe (true) or loopback port.
    pub(crate) pipe: bool,
}

/// Linux: unprivileged user namespaces are blocked by `AppArmor` (Ubuntu 23.10+) and the binary
/// has no `AppArmor` profile of its own (downloaded builds under the home directory), so the
/// Chromium sandbox cannot start — `--no-sandbox` is the documented workaround (unipi UNI-287).
pub(crate) fn needs_no_sandbox(engine: &Engine) -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    if engine.kind == EngineKind::Cloak {
        return true;
    }
    let restricted =
        std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
            .is_ok_and(|s| s.trim() == "1");
    let system = ["/usr/", "/opt/", "/snap/", "/var/lib/flatpak/"]
        .iter()
        .any(|p| engine.executable.starts_with(p));
    restricted && !system
}

/// The common flags (§4.4).
pub(crate) fn flags(spec: &LaunchSpec) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", spec.user_data_dir.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--password-store=basic".into(),
        "--use-mock-keychain".into(),
        "--hide-crash-restore-bubble".into(),
        "--disable-session-crashed-bubble".into(),
        "--disable-backgrounding-occluded-windows".into(),
        "--disable-renderer-backgrounding".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-features=Translate,MediaRouter".into(),
        "--window-size=1280,800".into(),
    ];
    if spec.pipe {
        args.push("--remote-debugging-pipe".into());
    } else {
        args.push("--remote-debugging-port=0".into());
        args.push("--remote-debugging-address=127.0.0.1".into());
    }
    match &spec.placement {
        Placement::Headless => {
            args.push("--headless=new".into());
            args.push("--hide-scrollbars".into());
            args.push("--mute-audio".into());
        }
        Placement::Hidden(_) => args.push("--ozone-platform=x11".into()),
        Placement::Window => {}
    }
    if spec.engine.kind == EngineKind::Cloak {
        let seed = seed(&spec.user_data_dir);
        let platform = if cfg!(target_os = "macos") {
            "macos"
        } else {
            "windows"
        };
        args.push(format!("--fingerprint={seed}"));
        args.push(format!("--fingerprint-platform={platform}"));
        args.push("--ignore-gpu-blocklist".into());
    }
    if needs_no_sandbox(&spec.engine) {
        args.push("--no-sandbox".into());
    }
    args.push("about:blank".into());
    args
}

/// Per-profile `CloakBrowser` fingerprint seed, generated once.
fn seed(dir: &Path) -> u32 {
    let file = dir.join(".cloakbrowser-seed");
    if let Some(n) = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        return n;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let n = 10_000 + (nanos ^ std::process::id().rotate_left(13)) % 89_999;
    let _ = std::fs::write(&file, n.to_string());
    n
}

/// A running browser.
pub(crate) struct BrowserProcess {
    child: std::sync::Mutex<Option<tokio::process::Child>>,
    pid: Option<u32>,
    /// CDP connection.
    pub(crate) cdp: Arc<Cdp>,
    /// Display it draws on.
    pub(crate) placement: Placement,
}

impl std::fmt::Debug for BrowserProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserProcess")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl BrowserProcess {
    /// Launches and connects.
    pub(crate) async fn launch(spec: &LaunchSpec) -> Result<Self, String> {
        std::fs::create_dir_all(&spec.user_data_dir)
            .map_err(|e| format!("profile dir {}: {e}", spec.user_data_dir.display()))?;
        let mut cmd = tokio::process::Command::new(&spec.engine.executable);
        cmd.args(flags(spec))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        match &spec.placement {
            Placement::Hidden(d) => {
                cmd.env("DISPLAY", d.var()).env_remove("WAYLAND_DISPLAY");
            }
            Placement::Headless => {
                cmd.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
            }
            Placement::Window => {}
        }
        #[cfg(unix)]
        cmd.process_group(0);
        let active_port = spec.user_data_dir.join("DevToolsActivePort");
        let _ = std::fs::remove_file(&active_port);
        #[cfg(unix)]
        let ours = if spec.pipe {
            Some(attach_pipes(&mut cmd)?)
        } else {
            None
        };
        #[cfg(not(unix))]
        if spec.pipe {
            return Err("the CDP pipe needs Unix".into());
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("{}: {e}", spec.engine.executable.display()))?;
        let pid = child.id();
        #[cfg(unix)]
        if let Some((read, write)) = ours {
            return Ok(Self {
                child: std::sync::Mutex::new(Some(child)),
                pid,
                cdp: Cdp::start(Transport::Pipe { read, write }),
                placement: spec.placement.clone(),
            });
        }
        let url = match wait_active_port(&active_port, &mut child).await {
            Ok(url) => url,
            Err(e) => {
                kill_group(pid);
                return Err(e);
            }
        };
        let transport = Transport::connect_ws(&url).await?;
        Ok(Self {
            child: std::sync::Mutex::new(Some(child)),
            pid,
            cdp: Cdp::start(transport),
            placement: spec.placement.clone(),
        })
    }

    /// OS process id.
    pub(crate) fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// `Browser.close`, then kill the process group after a grace period.
    pub(crate) async fn shutdown(&self) {
        let _ = self
            .cdp
            .call_timeout(
                None,
                "Browser.close",
                serde_json::json!({}),
                Duration::from_secs(2),
            )
            .await;
        let child = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(mut child) = child
            && tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .is_err()
        {
            kill_group(self.pid);
            let _ = child.kill().await;
        }
        kill_group(self.pid);
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        let alive = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if alive.is_some() {
            kill_group(self.pid);
        }
    }
}

/// Kills the browser's process group (renderers, GPU, zygote).
fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    #[cfg(not(unix))]
    let _ = pid;
}

#[cfg(unix)]
fn attach_pipes(
    cmd: &mut tokio::process::Command,
) -> Result<(std::io::PipeReader, std::io::PipeWriter), String> {
    use command_fds::{CommandFdExt as _, FdMapping};
    let (child_read, our_write) = std::io::pipe().map_err(|e| e.to_string())?;
    let (our_read, child_write) = std::io::pipe().map_err(|e| e.to_string())?;
    cmd.fd_mappings(vec![
        FdMapping {
            parent_fd: child_read.into(),
            child_fd: 3,
        },
        FdMapping {
            parent_fd: child_write.into(),
            child_fd: 4,
        },
    ])
    .map_err(|e| e.to_string())?;
    Ok((our_read, our_write))
}

async fn wait_active_port(
    file: &Path,
    child: &mut tokio::process::Child,
) -> Result<String, String> {
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(file) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                return Ok(format!("ws://127.0.0.1:{}{}", port.trim(), path.trim()));
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("the browser exited at start ({status})"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("the browser did not open its DevTools endpoint".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_sets() {
        let spec = LaunchSpec {
            engine: Engine {
                kind: EngineKind::Chrome,
                executable: "/usr/bin/chromium".into(),
                product: "Chromium".into(),
            },
            user_data_dir: "/tmp/p".into(),
            placement: Placement::Headless,
            pipe: true,
        };
        let f = flags(&spec);
        assert!(f.contains(&"--remote-debugging-pipe".to_owned()));
        assert!(f.contains(&"--headless=new".to_owned()));
        assert!(!f.iter().any(|a| a.contains("enable-automation")));
        assert!(!f.iter().any(|a| a.starts_with("--remote-debugging-port")));
        let port = LaunchSpec {
            pipe: false,
            placement: Placement::Window,
            ..spec
        };
        let f = flags(&port);
        assert!(f.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
        assert!(!f.iter().any(|a| a.starts_with("--headless")));
    }
}
