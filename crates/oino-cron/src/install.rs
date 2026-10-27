//! `oino cron install` / `uninstall`: registers a user service that runs
//! `oino cron serve` at login, per platform.
//!
//! - **Linux**: a systemd user unit (`~/.config/systemd/user/oino-cron.service`), enabled
//!   with `systemctl --user`.
//! - **macOS**: a launchd agent (`~/Library/LaunchAgents/dev.oino.cron.plist`), loaded
//!   with `launchctl`.
//! - **Windows**: a Task Scheduler logon task (`schtasks /Create … /SC ONLOGON`).
//!
//! The unit/plist text generators are pure and unit-tested; the install commands shell
//! out to the platform tool.

use std::path::PathBuf;

use crate::error::{CronError, Result};
use crate::paths::CronPaths;

/// The service name on every platform.
pub const SERVICE_NAME: &str = "oino-cron";

/// Where the systemd user unit lives.
#[must_use]
pub fn systemd_unit_path() -> Option<PathBuf> {
    dirs::config_dir().map(|c| {
        c.join("systemd/user")
            .join(format!("{SERVICE_NAME}.service"))
    })
}

/// Where the launchd agent lives.
#[must_use]
pub fn launchd_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("LaunchAgents")
            .join("dev.oino.cron.plist")
    })
}

/// The systemd user unit text for `oino cron serve` at `bin`.
#[must_use]
pub fn systemd_unit(bin: &str) -> String {
    format!(
        "[Unit]\nDescription=Oino cron scheduler (scheduled, unattended Oino runs)\nPartOf=graphical-session.target\n\n[Service]\nType=simple\nExecStart={bin} cron serve\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n"
    )
}

/// The launchd agent plist text for `bin` (adapted for the current machine).
#[must_use]
pub fn launchd_plist(bin: &str) -> String {
    let label = "dev.oino.cron";
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>{label}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{bin}</string>\n    <string>cron</string>\n    <string>serve</string>\n  </array>\n  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><true/>\n  <key>StandardErrorPath</key><string>/tmp/oino-cron.log</string>\n</dict>\n</plist>\n"
    )
}

/// The `schtasks` argv that creates the Windows logon task for `bin`.
#[must_use]
pub fn schtasks_create(bin: &str) -> Vec<String> {
    vec![
        "/Create".to_owned(),
        "/F".to_owned(),
        "/TN".to_owned(),
        SERVICE_NAME.to_owned(),
        "/TR".to_owned(),
        format!("\\\"{bin}\\\" cron serve"),
        "/SC".to_owned(),
        "ONLOGON".to_owned(),
        "/RL".to_owned(),
        "LIMITED".to_owned(),
    ]
}

/// The `schtasks` argv that removes the task.
#[must_use]
pub fn schtasks_delete() -> Vec<String> {
    vec![
        "/Delete".to_owned(),
        "/F".to_owned(),
        "/TN".to_owned(),
        SERVICE_NAME.to_owned(),
    ]
}

fn run_quiet(program: &str, args: &[String]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| CronError::control(format!("couldn't run {program}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CronError::control(format!(
            "{program} {} failed with {status}",
            args.join(" ")
        )))
    }
}

/// The `oino` binary install should point at (this process's executable).
fn oino_binary() -> Result<PathBuf> {
    std::env::current_exe().map_err(|e| CronError::io(e, "current_exe"))
}

/// Installs the user service. Returns a user-facing description of what was done.
///
/// # Errors
/// [`CronError::Control`] naming the platform tool on failure.
pub fn install() -> Result<String> {
    let bin = oino_binary()?;
    let bin = bin.to_string_lossy().into_owned();
    if cfg!(target_os = "linux") {
        let unit = systemd_unit_path()
            .ok_or_else(|| CronError::control("no ~/.config/systemd/user directory"))?;
        if let Some(parent) = unit.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CronError::io(e, parent))?;
        }
        std::fs::write(&unit, systemd_unit(&bin)).map_err(|e| CronError::io(e, &unit))?;
        let written = format!("wrote {}", unit.display());
        run_quiet("systemctl", &["--user".into(), "daemon-reload".into()]).map_err(|e| {
            CronError::control(format!(
                "{written}, but systemd --user is not reachable ({e}) — start the scheduler with `oino cron serve`, or run `oino cron install` in a desktop session"
            ))
        })?;
        run_quiet(
            "systemctl",
            &[
                "--user".into(),
                "enable".into(),
                "--now".into(),
                format!("{SERVICE_NAME}.service"),
            ],
        )?;
        Ok(format!(
            "installed and started the systemd user service ({})",
            unit.display()
        ))
    } else if cfg!(target_os = "macos") {
        let plist = launchd_plist_path()
            .ok_or_else(|| CronError::control("no ~/Library/LaunchAgents directory"))?;
        if let Some(parent) = plist.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CronError::io(e, parent))?;
        }
        std::fs::write(&plist, launchd_plist(&bin)).map_err(|e| CronError::io(e, &plist))?;
        run_quiet(
            "launchctl",
            &["unload".into(), plist.to_string_lossy().into_owned()],
        )?;
        run_quiet(
            "launchctl",
            &["load".into(), plist.to_string_lossy().into_owned()],
        )?;
        Ok(format!(
            "installed and loaded the launchd agent ({})",
            plist.display()
        ))
    } else if cfg!(windows) {
        run_quiet("schtasks", &schtasks_create(&bin))?;
        run_quiet(
            "schtasks",
            &["/Run".into(), "/TN".into(), SERVICE_NAME.to_owned()],
        )?;
        Ok("installed and started the Task Scheduler task".to_owned())
    } else {
        Err(CronError::control(
            "no service manager known for this platform — run `oino cron serve` at login yourself",
        ))
    }
}

/// Uninstalls the user service.
///
/// # Errors
/// [`CronError::Control`] naming the platform tool on failure.
pub fn uninstall() -> Result<String> {
    if cfg!(target_os = "linux") {
        // Best effort: without a user manager (headless shells, containers) `systemctl
        // --user` fails, but the unit file still has to go.
        let stopped = run_quiet(
            "systemctl",
            &[
                "--user".into(),
                "disable".into(),
                "--now".into(),
                format!("{SERVICE_NAME}.service"),
            ],
        )
        .is_ok();
        let removed =
            systemd_unit_path().filter(|unit| std::fs::remove_file(unit).is_ok() || !unit.exists());
        let _ = run_quiet("systemctl", &["--user".into(), "daemon-reload".into()]);
        Ok(match (removed, stopped) {
            (Some(unit), true) => format!(
                "stopped and removed the systemd user service ({})",
                unit.display()
            ),
            (Some(unit), false) => format!(
                "removed the systemd user service unit ({}); systemd --user did not answer, so it may not have been running",
                unit.display()
            ),
            (None, _) => "no systemd user service to remove".to_owned(),
        })
    } else if cfg!(target_os = "macos") {
        if let Some(plist) = launchd_plist_path() {
            run_quiet(
                "launchctl",
                &["unload".into(), plist.to_string_lossy().into_owned()],
            )?;
            let _ = std::fs::remove_file(&plist);
        }
        Ok("unloaded and removed the launchd agent".to_owned())
    } else if cfg!(windows) {
        run_quiet("schtasks", &schtasks_delete())?;
        Ok("removed the Task Scheduler task".to_owned())
    } else {
        Err(CronError::control(
            "no service manager known for this platform",
        ))
    }
}

/// Where `install` puts its files on this platform (status display).
#[must_use]
pub fn describe(paths: &CronPaths) -> String {
    let daemon = if crate::client::daemon_running(paths) {
        "scheduler running (daemon)"
    } else {
        "scheduler not running"
    };
    let service = if cfg!(target_os = "linux") {
        systemd_unit_path().filter(|p| p.is_file()).map_or_else(
            || "no systemd user service".to_owned(),
            |_| "systemd user service installed".to_owned(),
        )
    } else if cfg!(target_os = "macos") {
        launchd_plist_path().filter(|p| p.is_file()).map_or_else(
            || "no launchd agent".to_owned(),
            |_| "launchd agent installed".to_owned(),
        )
    } else if cfg!(windows) {
        "Task Scheduler task (run `schtasks /Query /TN oino-cron` to check)".to_owned()
    } else {
        "no service manager".to_owned()
    };
    format!("{daemon}; {service}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_runs_serve() {
        let unit = systemd_unit("/usr/local/bin/oino");
        assert!(unit.contains("ExecStart=/usr/local/bin/oino cron serve"));
        assert!(unit.contains("[Install]"));
    }

    #[test]
    fn launchd_plist_is_valid_shaped() {
        let plist = launchd_plist("/usr/local/bin/oino");
        assert!(plist.contains("<string>dev.oino.cron</string>"));
        assert!(plist.contains("<string>cron</string>"));
        assert!(plist.contains("<string>serve</string>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn schtasks_argv_creates_and_deletes_the_named_task() {
        let create = schtasks_create("C:\\oino\\oino.exe");
        assert!(create.contains(&"/Create".to_owned()));
        assert!(create.contains(&SERVICE_NAME.to_owned()));
        assert!(
            create
                .windows(2)
                .any(|w| w[0] == "/TR" && w[1].contains("cron serve"))
        );
        let delete = schtasks_delete();
        assert!(delete.contains(&"/Delete".to_owned()));
        assert!(delete.contains(&SERVICE_NAME.to_owned()));
    }
}
