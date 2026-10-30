//! `unipi-host update` (UNI-263): self-update from the release feed.
//!
//! The feed is `https://unipi.nrn.one/latest.json` (written by
//! `scripts/release.sh`); `host.files.<platform>` gives the binary URL and its
//! sha256. One run:
//!
//! 1. fetch the feed, pick this machine's platform (`linux-x64`, `linux-arm64`,
//!    `darwin-x64`, `darwin-arm64`, `windows-x64`),
//! 2. compare `host.version` with ours (semver; never downgrades),
//! 3. download, check the sha256, run the new binary's `--version` (it must
//!    say the advertised version),
//! 4. replace the running executable atomically (rename over it on Unix; on
//!    Windows the running .exe is renamed aside first, which Windows allows),
//! 5. restart the always-on service if there is one: systemd user unit
//!    `unipi-host.service`, LaunchAgent `dev.unipi.host`, or the scheduled
//!    task `UniPi Host`.
//!
//! The owner setting `update.mode` in `config.json` ("prompt" | "auto",
//! default prompt: user decision 6) is read by `serve`, which checks the feed
//! every few hours and either just records "update available" (the desktop
//! app shows it) or runs this same update itself.
//!
//! Tests point the feed at a local server with `UNIPI_UPDATE_FEED` and the
//! target at a temp copy with `--exe`; the base-URL check is relaxed only for
//! an explicit feed override.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::data_dir::HostPaths;

pub const FEED_URL: &str = "https://unipi.nrn.one/latest.json";
pub const FEED_ENV: &str = "UNIPI_UPDATE_FEED";
pub const SERVICE_UNIT: &str = "unipi-host.service";
pub const LAUNCH_AGENT: &str = "dev.unipi.host";
pub const WINDOWS_TASK: &str = "UniPi Host";

/// `host` block of `latest.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostRelease {
    pub version: String,
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, HostFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostFile {
    pub url: String,
    pub sha256: String,
}

#[derive(Deserialize)]
struct Feed {
    host: Option<HostRelease>,
}

/// `config.json` → `update` block (owner setting, user decision 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Tell the owner (desktop app, `unipi-host update --check`); never act alone.
    #[default]
    Prompt,
    /// `serve` installs a newer host by itself and restarts.
    Auto,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateConfig {
    #[serde(default)]
    pub mode: UpdateMode,
}

impl UpdateConfig {
    pub fn load(path: &Path) -> anyhow::Result<UpdateConfig> {
        #[derive(Deserialize, Default)]
        struct File {
            #[serde(default)]
            update: UpdateConfig,
        }
        Ok(crate::data_dir::read_json::<File>(path)?
            .unwrap_or_default()
            .update)
    }
    /// Writes the block back, keeping every other key of `config.json`.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut value: serde_json::Value =
            crate::data_dir::read_json(path)?.unwrap_or_else(|| serde_json::json!({}));
        if !value.is_object() {
            value = serde_json::json!({});
        }
        value["update"] = serde_json::to_value(self)?;
        crate::data_dir::write_json(path, &value)
    }
}

/// What `serve` last saw in the feed (`<host dir>/update.json`), read by
/// `sharing status --json` → the desktop app's "update available".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateState {
    pub checked_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn state_path(paths: &HostPaths) -> PathBuf {
    paths.root.join("update.json")
}

/// This build's platform key in `host.files`.
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    }
}

/// `X.Y.Z` (pre-release/build suffixes ignored) → comparable triple.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let t = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(t)
}

/// Is `latest` strictly newer than `current`?
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

pub fn feed_url() -> (String, bool) {
    match std::env::var(FEED_ENV) {
        Ok(url) if !url.is_empty() => (url, true),
        _ => (FEED_URL.to_string(), false),
    }
}

/// Parses `latest.json` and validates the host block. With the default feed,
/// binaries must come from `https://unipi.nrn.one/`.
pub fn parse_feed(bytes: &[u8], strict_origin: bool) -> anyhow::Result<HostRelease> {
    let feed: Feed = serde_json::from_slice(bytes)
        .map_err(|e| anyhow::anyhow!("the update feed is not valid JSON: {e}"))?;
    let host = feed
        .host
        .ok_or_else(|| anyhow::anyhow!("the update feed has no host block"))?;
    anyhow::ensure!(
        parse_version(&host.version).is_some(),
        "the update feed has a bad host version {:?}",
        host.version
    );
    for (plat, file) in &host.files {
        anyhow::ensure!(
            file.sha256.len() == 64 && file.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "bad sha256 for {plat}"
        );
        if strict_origin {
            anyhow::ensure!(
                file.url.starts_with("https://unipi.nrn.one/"),
                "refusing {plat} binary from {}",
                file.url
            );
        }
    }
    Ok(host)
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent(concat!("unipi-host/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

pub async fn fetch_feed() -> anyhow::Result<HostRelease> {
    let (url, custom) = feed_url();
    let res = client()?
        .get(&url)
        .header("cache-control", "no-cache")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("couldn't reach {url}: {e}"))?;
    anyhow::ensure!(res.status().is_success(), "{url}: HTTP {}", res.status());
    parse_feed(&res.bytes().await?, !custom)
}

/// The outcome of a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    UpToDate {
        current: String,
        latest: String,
    },
    Available {
        current: String,
        latest: String,
        file: HostFile,
    },
    NoBuild {
        latest: String,
        platform: String,
    },
}

pub fn decide(release: &HostRelease, current: &str, platform: Option<&str>) -> Check {
    if !is_newer(&release.version, current) {
        return Check::UpToDate {
            current: current.into(),
            latest: release.version.clone(),
        };
    }
    match platform.and_then(|p| release.files.get(p)) {
        Some(file) => Check::Available {
            current: current.into(),
            latest: release.version.clone(),
            file: file.clone(),
        },
        None => Check::NoBuild {
            latest: release.version.clone(),
            platform: platform.unwrap_or("unknown").into(),
        },
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Downloads `file` next to `exe`, checks it, swaps it in (on Windows the
/// old exe stays as `<exe>.old` until the next run removes it).
pub async fn download_and_replace(
    file: &HostFile,
    expect_version: &str,
    exe: &Path,
) -> anyhow::Result<()> {
    let res = client()?
        .get(&file.url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("download failed: {e}"))?;
    anyhow::ensure!(
        res.status().is_success(),
        "download failed: HTTP {}",
        res.status()
    );
    let bytes = res.bytes().await?;
    let got = sha256_hex(&bytes);
    anyhow::ensure!(
        got.eq_ignore_ascii_case(&file.sha256),
        "checksum mismatch (expected {}, got {got}) — nothing changed",
        file.sha256
    );
    install_bytes(&bytes, expect_version, exe)
}

/// Writes `bytes` as a sibling of `exe`, verifies `--version`, then replaces `exe`.
pub fn install_bytes(bytes: &[u8], expect_version: &str, exe: &Path) -> anyhow::Result<()> {
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("can't tell where {} lives", exe.display()))?;
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unipi-host");
    let tmp = dir.join(format!(".{name}.new-{}", std::process::id()));
    std::fs::write(&tmp, bytes)
        .map_err(|e| anyhow::anyhow!("can't write {}: {e}", tmp.display()))?;
    let cleanup = |e: anyhow::Error| {
        let _ = std::fs::remove_file(&tmp);
        e
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| cleanup(e.into()))?;
    }
    let out = std::process::Command::new(&tmp)
        .arg("--version")
        .output()
        .map_err(|e| cleanup(anyhow::anyhow!("the new binary doesn't run here: {e}")))?;
    let said = String::from_utf8_lossy(&out.stdout);
    let said = said.split_whitespace().nth(1).unwrap_or("").to_string();
    if said != expect_version {
        return Err(cleanup(anyhow::anyhow!(
            "the new binary says {said:?}, expected {expect_version} — nothing changed"
        )));
    }
    #[cfg(windows)]
    {
        // A running .exe can't be overwritten but can be renamed.
        let old = dir.join(format!("{name}.old"));
        let _ = std::fs::remove_file(&old);
        if exe.exists() {
            std::fs::rename(exe, &old)
                .map_err(|e| cleanup(anyhow::anyhow!("can't move the old binary aside: {e}")))?;
        }
        if let Err(e) = std::fs::rename(&tmp, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(cleanup(anyhow::anyhow!(
                "can't put the new binary in place: {e}"
            )));
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(&tmp, exe)
        .map_err(|e| cleanup(anyhow::anyhow!("can't replace {}: {e}", exe.display())))?;
    Ok(())
}

/// Which service manager runs `serve` here, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Systemd,
    LaunchAgent,
    ScheduledTask,
}

pub fn detect_service() -> Option<Service> {
    if cfg!(target_os = "linux") {
        let ok = std::process::Command::new("systemctl")
            .args(["--user", "is-enabled", "--quiet", SERVICE_UNIT])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        return ok.then_some(Service::Systemd);
    }
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        return home
            .join("Library/LaunchAgents")
            .join(format!("{LAUNCH_AGENT}.plist"))
            .exists()
            .then_some(Service::LaunchAgent);
    }
    if cfg!(windows) {
        let ok = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", WINDOWS_TASK])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        return ok.then_some(Service::ScheduledTask);
    }
    None
}

/// Does the service start this exe? (Never restart a service that runs some
/// other copy — e.g. a test replacing a temp binary on a PC whose real host
/// runs as a service.)
pub fn service_runs(service: Service, exe: &Path) -> bool {
    let definition = match service {
        Service::Systemd => std::process::Command::new("systemctl")
            .args(["--user", "cat", SERVICE_UNIT])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()),
        Service::LaunchAgent => std::env::var_os("HOME").map(PathBuf::from).and_then(|h| {
            std::fs::read_to_string(
                h.join("Library/LaunchAgents")
                    .join(format!("{LAUNCH_AGENT}.plist")),
            )
            .ok()
        }),
        Service::ScheduledTask => std::process::Command::new("schtasks")
            .args(["/Query", "/TN", WINDOWS_TASK, "/XML"])
            .output()
            .ok()
            .map(|o| {
                String::from_utf16_lossy(
                    &o.stdout
                        .chunks(2)
                        .map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)]))
                        .collect::<Vec<_>>(),
                ) + &String::from_utf8_lossy(&o.stdout)
            }),
    };
    definition.is_some_and(|d| definition_runs(&d, exe))
}

/// Text match of the exe path in a unit / plist / task XML (case-insensitive on Windows).
pub fn definition_runs(definition: &str, exe: &Path) -> bool {
    let exe = exe.to_string_lossy();
    if cfg!(windows) {
        definition.to_lowercase().contains(&exe.to_lowercase())
    } else {
        definition.contains(exe.as_ref())
    }
}

/// The command that restarts the service (argv).
pub fn restart_argv(service: Service, uid: Option<u32>) -> Vec<String> {
    restart_argv_for(service, uid, Path::new("unipi-host.exe"))
}

/// Like [`restart_argv`], for the scheduled task with the exe whose `serve`
/// must be stopped: the task runs `conhost --headless unipi-host serve`, and
/// `schtasks /End` only ends conhost — the old serve kept running (seen
/// updating coffee 0.4.4 → 0.4.5), so the task's new serve couldn't start.
pub fn restart_argv_for(service: Service, uid: Option<u32>, exe: &Path) -> Vec<String> {
    if service == Service::ScheduledTask {
        let exe = exe.to_string_lossy().replace('\'', "''");
        // No double quotes inside: Windows argv quoting would strip them.
        let script = format!(
            "schtasks /End /TN '{WINDOWS_TASK}' | Out-Null; \
             Get-CimInstance Win32_Process -Filter 'Name=''unipi-host.exe''' | \
             Where-Object {{ $_.ExecutablePath -eq '{exe}' -and $_.CommandLine -match '\\sserve(\\s|$)' }} | \
             ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}; \
             Start-Sleep -Milliseconds 500; schtasks /Run /TN '{WINDOWS_TASK}' | Out-Null; exit $LASTEXITCODE"
        );
        return vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script,
        ];
    }
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match service {
        Service::Systemd => s(&["systemctl", "--user", "restart", SERVICE_UNIT]),
        Service::LaunchAgent => vec![
            "launchctl".into(),
            "kickstart".into(),
            "-k".into(),
            format!("gui/{}/{LAUNCH_AGENT}", uid.unwrap_or(0)),
        ],
        // /End then /Run: the task action starts the new exe.
        Service::ScheduledTask => s(&[
            "cmd",
            "/C",
            &format!("schtasks /End /TN \"{WINDOWS_TASK}\" & schtasks /Run /TN \"{WINDOWS_TASK}\""),
        ]),
    }
}

#[cfg(unix)]
fn current_uid() -> Option<u32> {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
}
#[cfg(not(unix))]
fn current_uid() -> Option<u32> {
    None
}

/// Restarts the service so it runs the new binary. The restart kills the
/// process doing it when that process *is* the service (auto mode), which is fine.
pub fn restart_service(service: Service) -> anyhow::Result<()> {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("unipi-host.exe"));
    let argv = restart_argv_for(service, current_uid(), &exe);
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .map_err(|e| anyhow::anyhow!("couldn't run {}: {e}", argv[0]))?;
    anyhow::ensure!(status.success(), "`{}` failed", argv.join(" "));
    Ok(())
}

/// Records a check in `update.json` (errors are kept so the app can say why).
pub fn record(paths: &HostPaths, now: u64, result: &anyhow::Result<HostRelease>) {
    let state = match result {
        Ok(r) => UpdateState {
            checked_at: now,
            latest: Some(r.version.clone()),
            error: None,
        },
        Err(e) => {
            let prev = crate::data_dir::read_json::<UpdateState>(&state_path(paths))
                .ok()
                .flatten()
                .unwrap_or_default();
            UpdateState {
                checked_at: now,
                latest: prev.latest,
                error: Some(e.to_string()),
            }
        }
    };
    let _ = crate::data_dir::write_json(&state_path(paths), &state);
}

/// Status for `sharing status --json` (`update` field).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UpdateStatus {
    pub mode: UpdateMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn status(paths: &HostPaths) -> UpdateStatus {
    let mode = UpdateConfig::load(&paths.config())
        .map(|c| c.mode)
        .unwrap_or_default();
    let state = crate::data_dir::read_json::<UpdateState>(&state_path(paths))
        .ok()
        .flatten();
    let latest = state.as_ref().and_then(|s| s.latest.clone());
    UpdateStatus {
        mode,
        available: latest
            .as_deref()
            .is_some_and(|l| is_newer(l, env!("CARGO_PKG_VERSION"))),
        latest,
        checked_at: state.as_ref().map(|s| s.checked_at),
        error: state.and_then(|s| s.error),
    }
}

/// Removes a leftover `<exe>.old` from an earlier Windows update.
pub fn cleanup_old(exe: &Path) {
    if let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().and_then(|n| n.to_str())) {
        let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
    }
}

/// How often `serve` checks the feed (the first check is a minute after start).
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.4.10", "0.4.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.4.4", "0.4.4"));
        assert!(!is_newer("0.4.3", "0.4.4"));
        assert!(!is_newer("garbage", "0.4.4"));
        assert_eq!(parse_version("v0.4.4-alpha.1"), Some((0, 4, 4)));
        assert_eq!(parse_version("0.4"), None);
        assert_eq!(parse_version("0.4.4.1"), None);
    }

    fn feed(url: &str) -> String {
        format!(
            r#"{{"app":{{"version":"0.4.4","build":4004,"url":"https://unipi.nrn.one/a.apk","sha256":"{}"}},
               "host":{{"version":"0.4.4","files":{{"linux-x64":{{"url":"{url}","sha256":"{}"}}}}}}}}"#,
            "a".repeat(64),
            "b".repeat(64)
        )
    }

    #[test]
    fn feed_parses_and_checks_origin() {
        let ok = parse_feed(
            feed("https://unipi.nrn.one/releases/host/0.4.4/unipi-host-linux-x64").as_bytes(),
            true,
        )
        .unwrap();
        assert_eq!(ok.version, "0.4.4");
        assert!(parse_feed(feed("https://evil.example/x").as_bytes(), true).is_err());
        assert!(parse_feed(feed("http://127.0.0.1:1/x").as_bytes(), false).is_ok());
        assert!(parse_feed(b"{}", true).is_err());
        assert!(parse_feed(b"nope", true).is_err());
        let bad_sha = feed("https://unipi.nrn.one/x").replace(&"b".repeat(64), "zz");
        assert!(parse_feed(bad_sha.as_bytes(), true).is_err());
    }

    #[test]
    fn decide_picks_platform_and_never_downgrades() {
        let r = parse_feed(feed("https://unipi.nrn.one/x").as_bytes(), true).unwrap();
        assert!(matches!(
            decide(&r, "0.4.3", Some("linux-x64")),
            Check::Available { .. }
        ));
        assert!(matches!(
            decide(&r, "0.4.4", Some("linux-x64")),
            Check::UpToDate { .. }
        ));
        assert!(matches!(
            decide(&r, "0.5.0", Some("linux-x64")),
            Check::UpToDate { .. }
        ));
        assert!(matches!(
            decide(&r, "0.4.3", Some("darwin-arm64")),
            Check::NoBuild { .. }
        ));
    }

    #[test]
    fn service_match_is_by_exe_path() {
        let unit = "[Service]\nExecStart=/home/u/.local/bin/unipi-host serve\n";
        assert!(definition_runs(
            unit,
            Path::new("/home/u/.local/bin/unipi-host")
        ));
        assert!(!definition_runs(unit, Path::new("/tmp/qa/unipi-host")));
    }

    #[test]
    fn restart_commands() {
        assert_eq!(
            restart_argv(Service::Systemd, None).join(" "),
            "systemctl --user restart unipi-host.service"
        );
        assert_eq!(
            restart_argv(Service::LaunchAgent, Some(501)).join(" "),
            "launchctl kickstart -k gui/501/dev.unipi.host"
        );
        let win = restart_argv_for(
            Service::ScheduledTask,
            None,
            Path::new(r"C:\Users\o'i\AppData\Local\UniPi\unipi-host.exe"),
        );
        assert_eq!(win[0], "powershell.exe");
        let script = &win[4];
        // Ends the task, stops that exe's `serve` (not this `update` CLI), runs the task again.
        assert!(
            script.starts_with("schtasks /End /TN 'UniPi Host'"),
            "{script}"
        );
        assert!(
            script.contains(
                r"$_.ExecutablePath -eq 'C:\Users\o''i\AppData\Local\UniPi\unipi-host.exe'"
            ),
            "{script}"
        );
        assert!(
            script.contains("serve(") && script.contains("schtasks /Run /TN 'UniPi Host'"),
            "{script}"
        );
        assert!(
            !script.contains('"'),
            "no double quotes (Windows argv quoting strips them): {script}"
        );
    }

    #[test]
    fn config_round_trip_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"forwarding":{"enabled":true}}"#).unwrap();
        assert_eq!(UpdateConfig::load(&path).unwrap().mode, UpdateMode::Prompt);
        UpdateConfig {
            mode: UpdateMode::Auto,
        }
        .save(&path)
        .unwrap();
        assert_eq!(UpdateConfig::load(&path).unwrap().mode, UpdateMode::Auto);
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["forwarding"]["enabled"], true);
    }

    #[test]
    fn status_reports_available_from_state() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(dir.path());
        assert!(!status(&paths).available);
        record(
            &paths,
            5,
            &Ok(HostRelease {
                version: "999.0.0".into(),
                files: Default::default(),
            }),
        );
        let s = status(&paths);
        assert!(s.available);
        assert_eq!(s.latest.as_deref(), Some("999.0.0"));
        record(&paths, 6, &Err(anyhow::anyhow!("offline")));
        let s = status(&paths);
        assert_eq!(s.error.as_deref(), Some("offline"));
        assert_eq!(s.latest.as_deref(), Some("999.0.0"));
    }

    #[cfg(unix)]
    #[test]
    fn install_refuses_wrong_version_and_keeps_old() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("unipi-host");
        std::fs::write(&exe, "old").unwrap();
        let script = b"#!/bin/sh\necho unipi-host 9.9.9\n";
        let err = install_bytes(script, "1.0.0", &exe)
            .unwrap_err()
            .to_string();
        assert!(err.contains("expected 1.0.0"), "{err}");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        install_bytes(script, "9.9.9", &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), script);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp files left"
        );
    }
}
