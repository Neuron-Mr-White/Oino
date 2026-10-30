//! UNI-238 (plan A7) "Keep available when UniPi is closed": install and remove
//! the systemd **user** unit that runs this PC's host when the app isn't.
//!
//! - the unit text is the same `scripts/install.sh` writes — the test below
//!   reads that script's heredoc and compares, so the two can't drift;
//! - the host always runs a **copy** in `~/.local/bin`, never the bundle path
//!   (an AppImage is read-only and its mount point moves);
//! - the data dir never changes, so the endpoint id (and every paired phone)
//!   survives the hand-off from an app-managed host to the service;
//! - `loginctl enable-linger` needs root. The app only ever *asks*, behind a
//!   button, with the reason; it never sudo's by itself.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use serde::Serialize;

use unipi_proto::{AppErrorCode, AppErrorDto};

use crate::host_supervisor::{bin_dir, is_executable, systemctl, UNIT_NAME};

/// `~/.config/systemd/user`, or `$XDG_CONFIG_HOME/systemd/user`.
pub fn unit_dir(home: &Path) -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("systemd").join("user"),
        None => home.join(".config").join("systemd").join("user"),
    }
}

/// `<unit dir>/unipi-host.service`.
pub fn unit_path(home: &Path) -> PathBuf {
    unit_dir(home).join(format!("{UNIT_NAME}.service"))
}

/// `~/.unipi/app-host/env`: the ONE place the host's `PATH` comes from.
///
/// Both the app and `scripts/install.sh` write it with the merged PATH (the
/// shell's, not a GUI's — `setup::detect` merges them), so herdr, tmux,
/// kanboard and pi resolve the same way as in the user's shell. Keeping it in
/// a file (rather than an `Environment=PATH=` line) means the unit text stays
/// machine-independent: `install.sh` and this module produce byte-identical
/// units, which the parity test asserts, and a re-install only rewrites the
/// env file.
pub fn env_path(home: &Path) -> PathBuf {
    home.join(".unipi").join("app-host").join("env")
}

/// Writes `PATH=<merged>` for the host. Called by both writers.
pub fn write_env_file(home: &Path, path: &str) -> std::io::Result<()> {
    let path_file = env_path(home);
    if let Some(dir) = path_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // One `KEY=value` line; systemd quotes it for us.
    std::fs::write(path_file, format!("PATH={path}\n"))
}

/// The unit file text, byte-identical to `install.sh`'s. The PATH comes from
/// the env file next to the host's data dir, so the text has nothing
/// machine-specific in it.
pub fn unit_text(bin: &Path, home: &Path) -> String {
    let bin_display = bin.display().to_string();
    let env_display = env_path(home).display().to_string();
    format!(
        "[Unit]\n\
         Description=UniPi host (phone access to herdr/tmux sessions and pi chats)\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={bin_display} serve --mode service\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         EnvironmentFile={env_display}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// Installs the unit (copying `from` over `<bin dir>/unipi-host` first) and
/// starts it. The copy is what the service runs, so an AppImage's read-only
/// bundle path is never referenced by the unit.
pub async fn enable(home: &Path, from: &Path) -> Result<(), AppErrorDto> {
    install_files(home, from).map_err(|e| io("install the host as a service", e))?;
    // The host's PATH: the merged one, the same file install.sh writes.
    let merged = crate::setup::merged_path_here(home).await;
    write_env_file(home, &merged).map_err(|e| io("write the host env file", e))?;
    run_systemctl(&["daemon-reload"]).await?;
    run_systemctl(&["enable", "--now", UNIT_NAME]).await?;
    Ok(())
}

/// The file side of `enable`, without systemd: copies `from` — the bundled
/// sidecar when the app has one (§3.2's skew path) — over
/// `<bin dir>/unipi-host` and writes the unit that runs the copy.
pub fn install_files(home: &Path, from: &Path) -> std::io::Result<PathBuf> {
    let target = bin_dir(home).join("unipi-host");
    std::fs::create_dir_all(bin_dir(home))?;
    std::fs::copy(from, &target)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
    }
    let path = unit_path(home);
    std::fs::create_dir_all(path.parent().expect("unit dir"))?;
    std::fs::write(&path, unit_text(&target, home))?;
    Ok(target)
}

/// Disables the unit and removes it. The copied binary stays (removing it
/// would break a host that is still running from it).
pub async fn disable(home: &Path) -> Result<(), AppErrorDto> {
    let installed = unit_path(home).exists();
    if installed {
        let _ = run_systemctl(&["disable", "--now", UNIT_NAME]).await;
        let _ = std::fs::remove_file(unit_path(home));
        let _ = run_systemctl(&["daemon-reload"]).await;
    }
    Ok(())
}

/// Is the unit installed (our marker: the file this module writes)?
pub fn installed(home: &Path) -> bool {
    unit_path(home).exists()
}

/// `loginctl show-user <me> -p Linger` == yes? (Needs root to turn on; the app
/// only reads it.)
pub fn linger_enabled() -> Option<bool> {
    let out = std::process::Command::new("loginctl")
        .args(["show-user", &whoami(), "-p", "Linger", "--value"])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim() == "yes")
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| {
        std::process::Command::new("id").arg("-un").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_default().trim().to_string()
    })
}

/// The card's note: what the user has to do themselves, and never silently.
pub fn linger_hint() -> String {
    "A user service stops when you log out. To keep it running with nobody logged in, allow it once — it needs your password.".to_string()
}

/// The pkexec command for the linger hint (run only from the user's button).
pub fn linger_command() -> String {
    let user = whoami();
    format!("pkexec loginctl enable-linger {user}")
}

async fn run_systemctl(args: &[&str]) -> Result<(), AppErrorDto> {
    let Some(bin) = systemctl(std::env::var_os("PATH").as_deref()) else {
        return Err(AppErrorDto::new(AppErrorCode::E_UNSUPPORTED, "systemctl isn't on this PC", false));
    };
    let out = tokio::process::Command::new(&bin)
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| io("run systemctl", e))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(AppErrorDto::new(
        AppErrorCode::E_INTERNAL,
        format!("systemctl --user {} failed: {}", args.join(" "), stderr.trim()),
        false,
    ))
}

fn io(what: &str, e: std::io::Error) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, format!("{what}: {e}"), false)
}

/// What Settings → This computer shows for "Keep available when UniPi is
/// closed" (UNI-238).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlwaysOnState {
    /// The unit is installed (this app installed it).
    pub installed: bool,
    /// The unit is running right now.
    pub running: bool,
    /// Where the host binary the service runs lives.
    pub bin: Option<String>,
    /// `true`/`false`, or `null` when `loginctl` isn't there.
    pub linger: Option<bool>,
    /// The card's note about needing the user's password (only shown when
    /// linger is off and they asked).
    pub linger_hint: Option<String>,
    /// The command the "Allow it" button runs (never run silently).
    pub linger_command: Option<String>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty()).map(PathBuf::from).unwrap_or_default()
}

/// `always_on_state`.
#[tauri::command]
pub async fn always_on_state() -> Result<AlwaysOnState, AppErrorDto> {
    let home = home();
    let running = tokio::process::Command::new(systemctl(std::env::var_os("PATH").as_deref()).unwrap_or_else(|| "systemctl".into()))
        .args(["--user", "is-active", UNIT_NAME])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    let bin = bin_dir(&home).join("unipi-host");
    let linger = linger_enabled();
    Ok(AlwaysOnState {
        installed: installed(&home),
        running,
        bin: is_executable(&bin).then(|| bin.display().to_string()),
        linger_hint: (linger == Some(false)).then(linger_hint),
        linger_command: (linger == Some(false)).then(linger_command),
        linger,
    })
}

/// `always_on_set {on}`: install + start, or disable + remove. `from` is the
/// host binary to copy into `~/.local/bin` (the bundled sidecar when there is
/// one, else the one already there).
#[tauri::command]
pub async fn always_on_set(on: bool, from: Option<String>, app: tauri::AppHandle) -> Result<AlwaysOnState, AppErrorDto> {
    let home = home();
    if on {
        // §3.2's skew path: the service runs a copy of what this app ships, so
        // `host_bin` (sidecar first, then ~/.local/bin / PATH) is the source.
        let source = from
            .filter(|f| !f.is_empty())
            .map(PathBuf::from)
            .or_else(|| crate::host_supervisor::host_bin(Some(&app)))
            .ok_or_else(|| AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "no host binary to install as a service", false))?;
        enable(&home, &source).await?;
    } else {
        disable(&home).await?;
    }
    always_on_state().await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heredoc `scripts/install.sh` writes — the shared fixture. Extracted
    /// with the same substitutions the script applies.
    fn install_sh_unit(bin_dir: &Path, home: &Path) -> String {
        let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../scripts/install.sh"))
            .expect("scripts/install.sh");
        let start = script.find("cat > \"$unit_dir/unipi-host.service\" <<EOF").expect("the unit heredoc");
        let body = &script[start..];
        let body = &body[body.find('\n').expect("heredoc opens") + 1..];
        let end = body.find("\nEOF").expect("heredoc closes");
        let text = &body[..end];
        text.replace("$host_env", &env_path(home).display().to_string())
            .replace("$BIN_DIR", &bin_dir.display().to_string())
    }

    #[test]
    fn the_unit_is_exactly_what_install_sh_writes() {
        let home = Path::new("/home/tester");
        let bin = bin_dir(home).join("unipi-host");
        let ours = unit_text(&bin, home);
        let theirs = install_sh_unit(&bin_dir(home), home);
        assert_eq!(ours.trim_end(), theirs.trim_end(), "unit drift:\n{ours}\n---\n{theirs}");
        assert!(ours.contains("ExecStart=/home/tester/.local/bin/unipi-host serve --mode service"));
        // The PATH is NOT in the unit: it comes from the env file both write.
        assert!(!ours.contains("Environment=PATH="));
        assert!(ours.contains(&format!("EnvironmentFile={}", env_path(home).display())));
        assert!(ours.contains("WantedBy=default.target"));
    }

    #[test]
    fn the_host_env_file_holds_the_merged_path_for_both_writers() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        assert_eq!(env_path(&home), home.join(".unipi/app-host/env"));
        write_env_file(&home, "/home/t/.local/bin:/usr/bin").unwrap();
        let written = std::fs::read_to_string(env_path(&home)).unwrap();
        assert_eq!(written, "PATH=/home/t/.local/bin:/usr/bin\n");
    }

    #[test]
    fn the_unit_runs_a_copy_of_the_bundled_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        // A bundle layout: the sidecar sits next to the app exe.
        let exe_dir = tmp.path().join("usr/bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let sidecar = exe_dir.join(crate::host_supervisor::SIDECAR_NAME);
        std::fs::write(&sidecar, b"#!/bin/sh\necho I am the bundled host\n").unwrap();
        let target = install_files(&home, &sidecar).expect("copy the host and write the unit");
        let unit = std::fs::read_to_string(unit_path(&home)).unwrap();
        // ExecStart points at the copy in ~/.local/bin, never at the bundle path
        // (an AppImage is read-only and its mount point moves).
        assert!(unit.contains(&format!("ExecStart={} serve --mode service", target.display())), "{unit}");
        assert!(!unit.contains(&exe_dir.display().to_string()), "{unit}");
        assert_eq!(target, bin_dir(&home).join("unipi-host"));
        // The copy is the sidecar's bytes, and it is executable.
        assert!(std::fs::read_to_string(&target).unwrap().contains("I am the bundled host"));
        assert!(is_executable(&target));
    }

    #[test]
    fn install_remove_round_trip_in_a_fake_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        assert!(!installed(&home));
        // What `enable` writes (without systemctl): the unit and the copy.
        let source = tmp.path().join("bundle-host");
        std::fs::write(&source, b"#!/bin/sh\n").unwrap();
        let target = install_files(&home, &source).expect("copy the host and write the unit");
        assert!(installed(&home));
        // The unit never points into a bundle (the copy is what runs).
        assert!(!std::fs::read_to_string(unit_path(&home)).unwrap().contains("bundle-host"));
        assert_eq!(unit_path(&home), unit_dir(&home).join("unipi-host.service"));
        assert_eq!(target, bin_dir(&home).join("unipi-host"));
        std::fs::remove_file(unit_path(&home)).unwrap();
        assert!(!installed(&home));
    }

    #[test]
    fn linger_is_asked_about_never_assumed() {
        // The hint mentions the password; the command is only run from a button.
        assert!(linger_hint().to_lowercase().contains("password"));
        let user = whoami();
        assert_eq!(linger_command(), format!("pkexec loginctl enable-linger {user}"));
    }
}
