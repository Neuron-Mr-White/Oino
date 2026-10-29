//! A private, hidden X server for CloakBrowser (UNI-299): the host runs its
//! own headed CloakBrowser on a display nobody looks at, instead of on the
//! user's real desktop — so it never pops a window up on the PC, but still
//! isn't the plain `--headless=new` CloakBrowser's own README says some
//! sites detect even with the stealth patches.
//!
//! - **Xvfb** `:N -screen 0 1920x1080x24 -nolisten tcp`, `:N` the first free
//!   display number from 99 up (checked against `/tmp/.X11-unix/X<N>` and
//!   the lock files so two hosts, or a host and something else, never
//!   collide), `-displayfd` used when available to get the number
//!   race-free.
//! - **Xwayland** `:N -noreset` as the fallback when `Xvfb` is missing (it
//!   is, on this PC — Xwayland ships by default on most Wayland desktops;
//!   Xvfb needs its own package). Nested under the user's own Wayland
//!   session, but nothing shows: nothing ever maps a window on it, and nothing
//!   asks the compositor to present one.
//! - Neither present: [`spawn`] reports it, and the caller falls back to
//!   plain headless Chrome (`mod.rs`'s "no hidden display possible").
//!
//! One display is started lazily (the first browser that needs `hidden`),
//! shared by every profile that runs hidden, and stopped when the last one
//! using it stops, or at host exit — [`Displays`] only ever stops a server
//! it itself started (its own child process), never someone else's X
//! server that happened to already be on that number.

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};

/// First display number tried (low numbers are mostly real desktops / X11
/// forwarding; high enough to almost never collide, low enough to never
/// approach the 16-bit display limit some tools assume).
pub const FIRST_DISPLAY: u32 = 99;
/// Give up looking for a free number after this many tries.
pub const MAX_TRIES: u32 = 200;

/// Which server binary runs the hidden display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Xvfb,
    Xwayland,
}

impl Backend {
    pub fn program(self) -> &'static str {
        match self {
            Backend::Xvfb => "Xvfb",
            Backend::Xwayland => "Xwayland",
        }
    }
}

/// Is `display` (`:N`, no leading colon required) free: no socket and no
/// lock file for it? Linux/Unix only (X11 doesn't exist elsewhere, and
/// hidden-display mode is unavailable there anyway).
pub fn display_free(n: u32) -> bool {
    !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
}

/// The first free display number from `start`, trying at most `tries`.
/// `None` when they are all taken (vanishingly unlikely).
pub fn first_free(start: u32, tries: u32, free: &dyn Fn(u32) -> bool) -> Option<u32> {
    (start..start.saturating_add(tries)).find(|n| free(*n))
}

/// A binary on `PATH` (names only — `Xvfb`/`Xwayland` are never configured
/// paths), from `dirs` in order.
fn find_on_path(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(name)).find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}
#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

/// Which backend is available here: `Xvfb` first (purpose-built, never
/// touches a real compositor), then `Xwayland` (ships with most Wayland
/// desktops already). `None` = no hidden display is possible on this PC.
pub fn detect_backend(path_var: Option<&std::ffi::OsStr>) -> Option<Backend> {
    let dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    // Only Xvfb is truly invisible. Xwayland started on its own is *rootful*:
    // it opens a visible black window on the user's Wayland desktop to draw
    // the X screen in (seen 2026-10-16) — the opposite of "only in the app".
    // Without Xvfb the browser runs headless (marked less stealthy) instead.
    find_on_path(&dirs, "Xvfb").map(|_| Backend::Xvfb)
}

/// [`detect_backend`] with this process's `PATH`.
pub fn detect_backend_here() -> Option<Backend> {
    detect_backend(std::env::var_os("PATH").as_deref())
}

/// The argv for `backend` on display number `n` (`-displayfd` used instead
/// of `:n` when `displayfd` is `Some`, so the caller reads the real number
/// back race-free instead of trusting the one it guessed).
pub fn args_for(backend: Backend, n: u32, displayfd: Option<i32>) -> Vec<String> {
    match (backend, displayfd) {
        (Backend::Xvfb, Some(fd)) => {
            vec!["-displayfd".into(), fd.to_string(), "-screen".into(), "0".into(), "1920x1080x24".into(), "-nolisten".into(), "tcp".into()]
        }
        (Backend::Xvfb, None) => {
            vec![format!(":{n}"), "-screen".into(), "0".into(), "1920x1080x24".into(), "-nolisten".into(), "tcp".into()]
        }
        (Backend::Xwayland, Some(fd)) => vec!["-displayfd".into(), fd.to_string(), "-noreset".into()],
        (Backend::Xwayland, None) => vec![format!(":{n}"), "-noreset".into()],
    }
}

/// One hidden X server the host started.
pub struct Display {
    pub number: u32,
    pub backend: Backend,
    child: tokio::sync::Mutex<tokio::process::Child>,
    pid: Option<u32>,
}

impl Display {
    /// `DISPLAY=:<number>`.
    pub fn display_var(&self) -> String {
        format!(":{}", self.number)
    }

    /// The server is still running (Xwayland, for one, exits on its own when
    /// its last client goes, e.g. after the browser's idle shutdown).
    pub fn alive(&self) -> bool {
        match self.child.try_lock() {
            Ok(mut child) => matches!(child.try_wait(), Ok(None)),
            Err(_) => true, // being stopped/waited on right now: treat as busy
        }
    }

    /// Starts `backend` on the first free display from [`FIRST_DISPLAY`],
    /// using `-displayfd` when the backend supports it (both do) so the
    /// number is never guessed-and-raced. Waits for the socket to appear
    /// (or the process to die) before returning.
    /// No private X server outside Unix: the caller falls back to headless.
    #[cfg(not(unix))]
    async fn start(backend: Backend) -> anyhow::Result<Display> {
        anyhow::bail!("{} hidden displays are Unix-only", backend.program())
    }

    #[cfg(unix)]
    async fn start(backend: Backend) -> anyhow::Result<Display> {
        use std::os::fd::AsRawFd;
        let (read_fd, write_fd) = nix_pipe()?;
        let guess = first_free(FIRST_DISPLAY, MAX_TRIES, &display_free)
            .ok_or_else(|| anyhow::anyhow!("no free X display number found from :{FIRST_DISPLAY}"))?;
        let args = args_for(backend, guess, Some(write_fd.as_raw_fd()));
        let mut command = tokio::process::Command::new(backend.program());
        command.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        // `write_fd` has no FD_CLOEXEC (cleared in `nix_pipe`), so it survives
        // the fork+exec below without any extra plumbing; our own copy is
        // dropped right after spawn.
        let mut child = command.spawn().map_err(|e| anyhow::anyhow!("could not start {}: {e}", backend.program()))?;
        drop(write_fd);
        let pid = child.id();
        let stderr = child.stderr.take();
        // Read the real display number back from the fd the server wrote to.
        let mut fd_file = tokio::fs::File::from_std(std::fs::File::from(read_fd));
        let number = tokio::time::timeout(Duration::from_secs(10), read_display_number(&mut fd_file)).await;
        let number = match number {
            Ok(Ok(n)) => n,
            _ => {
                let _ = child.start_kill();
                let tail = stderr_tail(stderr).await;
                anyhow::bail!("{} did not report a display number — {}", backend.program(), tail.join("; "));
            }
        };
        // Drain stderr forever so the server never blocks on a full pipe.
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(_)) = lines.next_line().await {}
            });
        }
        tracing::info!(op = "browser_display", backend = backend.program(), display = number, pid, "hidden X server started");
        Ok(Display { number, backend, child: tokio::sync::Mutex::new(child), pid })
    }

    async fn stop(&self) {
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
        if tokio::time::timeout(Duration::from_secs(3), child.wait()).await.is_err() {
            if let Some(pid) = self.pid {
                let _ = std::process::Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]).status();
            }
            let _ = child.kill().await;
        }
        tracing::info!(op = "browser_display", backend = self.backend.program(), display = self.number, "hidden X server stopped");
    }
}

async fn stderr_tail(stderr: Option<tokio::process::ChildStderr>) -> Vec<String> {
    let Some(stderr) = stderr else { return Vec::new() };
    let mut lines = BufReader::new(stderr).lines();
    let mut tail = Vec::new();
    while let Ok(Some(line)) = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await.unwrap_or(Ok(None)) {
        tail.push(line);
        if tail.len() > 8 {
            tail.remove(0);
        }
    }
    tail
}

/// Reads the ASCII decimal line `-displayfd` writes (`"<n>\n"`).
async fn read_display_number(file: &mut tokio::fs::File) -> anyhow::Result<u32> {
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 16];
    let mut got = Vec::new();
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
        if got.contains(&b'\n') {
            break;
        }
    }
    let text = String::from_utf8_lossy(&got);
    text.trim().parse::<u32>().map_err(|_| anyhow::anyhow!("bad displayfd output {text:?}"))
}

/// A `pipe()` pair as owned fds (no extra crate: a thin libc wrapper).
#[cfg(unix)]
fn nix_pipe() -> anyhow::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        anyhow::bail!("pipe() failed: {}", std::io::Error::last_os_error());
    }
    // Clear O_CLOEXEC on the write end: the child needs it after exec.
    unsafe {
        let flags = libc::fcntl(fds[1], libc::F_GETFD);
        libc::fcntl(fds[1], libc::F_SETFD, flags & !libc::FD_CLOEXEC);
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}


/// How many running browsers use the shared hidden display right now.
pub struct Displays {
    display: tokio::sync::Mutex<Option<Arc<Display>>>,
}

impl Displays {
    pub fn new() -> Displays {
        Displays { display: tokio::sync::Mutex::new(None) }
    }

    /// The shared hidden display, starting it if needed. `Err` carries why
    /// not (no backend, or the server failed to start) — the caller falls
    /// back to plain headless.
    pub async fn ensure(&self) -> anyhow::Result<Arc<Display>> {
        let mut guard = self.display.lock().await;
        if let Some(d) = guard.as_ref() {
            if d.alive() {
                return Ok(d.clone());
            }
            // It died (Xwayland quits after its last client): start a fresh one
            // rather than handing a dead :N to the browser.
            tracing::info!(op = "browser_display", display = d.number, "hidden X server is gone; starting a new one");
            *guard = None;
        }
        let backend = detect_backend_here().ok_or_else(|| anyhow::anyhow!("no Xvfb or Xwayland found on this PC"))?;
        let display = Arc::new(Display::start(backend).await?);
        *guard = Some(display.clone());
        Ok(display)
    }

    /// Stops the hidden display if one is running (no active user left).
    pub async fn stop(&self) {
        let display = self.display.lock().await.take();
        if let Some(display) = display {
            display.stop().await;
        }
    }
}

impl Default for Displays {
    fn default() -> Displays {
        Displays::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug the user hit (2026-10-16): Xwayland exited on its own after the
    /// browser's idle shutdown, `ensure()` kept handing out the dead `:N`, and
    /// the next CloakBrowser died with "Missing X server or $DISPLAY".
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dead_display_is_replaced() {
        let Some(backend) = detect_backend_here() else { return }; // no X server binary here
        let displays = Displays::new();
        let first = displays.ensure().await.unwrap();
        // Kill it behind the manager's back, as Xwayland quitting would.
        first.child.lock().await.start_kill().unwrap();
        for _ in 0..50 {
            if !first.alive() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!first.alive());
        let second = displays.ensure().await.unwrap();
        assert!(second.alive(), "{} replaced", backend.program());
        displays.stop().await;
    }

    #[test]
    fn picks_the_first_free_number() {
        let taken = [99u32, 100, 101];
        let free = |n: u32| !taken.contains(&n);
        assert_eq!(first_free(99, 10, &free), Some(102));
        assert_eq!(first_free(50, 200, &free), Some(50));
        let all_taken = |_: u32| false;
        assert_eq!(first_free(99, 5, &all_taken), None);
    }

    #[test]
    fn display_free_checks_socket_and_lock() {
        let dir = tempfile::tempdir().unwrap();
        // We can't easily redirect /tmp/.X11-unix in a unit test (it's a
        // fixed path), so this only exercises a display we're sure is free.
        let _ = dir; // kept for parity with other tests' style
        assert!(display_free(54_321), "an absurd display number is free");
    }

    #[test]
    fn xvfb_args_carry_the_screen_and_no_tcp() {
        let args = args_for(Backend::Xvfb, 101, None);
        assert_eq!(args[0], ":101");
        assert!(args.contains(&"1920x1080x24".to_string()));
        assert!(args.contains(&"-nolisten".to_string()) && args.contains(&"tcp".to_string()));
        let args = args_for(Backend::Xvfb, 101, Some(7));
        assert_eq!(args[0], "-displayfd");
        assert_eq!(args[1], "7");
        assert!(!args.iter().any(|a| a == ":101"));
    }

    #[test]
    fn xwayland_args_are_minimal() {
        let args = args_for(Backend::Xwayland, 55, None);
        assert_eq!(args, vec![":55".to_string(), "-noreset".to_string()]);
        let args = args_for(Backend::Xwayland, 55, Some(9));
        assert_eq!(args, vec!["-displayfd".to_string(), "9".to_string(), "-noreset".to_string()]);
    }

    #[test]
    fn only_xvfb_is_a_hidden_display() {
        let dir = tempfile::tempdir().unwrap();
        let exe = |name: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        };
        let path_var = std::ffi::OsString::from(dir.path());
        assert_eq!(detect_backend(Some(&path_var)), None);
        exe("Xwayland");
        assert_eq!(detect_backend(Some(&path_var)), None, "standalone Xwayland opens a visible window: never used");
        exe("Xvfb");
        assert_eq!(detect_backend(Some(&path_var)), Some(Backend::Xvfb));
    }
}
