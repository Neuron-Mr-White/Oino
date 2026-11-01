//! `unipi-host update` (UNI-263; host.md §9.7): self-update from the oino/UniPi
//! release manifest on GitHub Releases.
//!
//! The feed is `release-manifest.json` — the same file `oino update` and
//! `scripts/install.sh` read (`{version, artifacts:[{name, target, url,
//! sha256}]}`). One run:
//!
//! 1. fetch the manifest, compare `version` with ours (semver; never
//!    downgrades),
//! 2. pick the `unipi-host` artifact of this target triple,
//! 3. download it, check the sha256, run the new binary's `--version` (it must
//!    say the advertised version),
//! 4. replace the running executable atomically (rename over it on Unix; on
//!    Windows the running .exe is renamed aside first, which Windows allows).
//!
//! **Restart without services** (v2 has no systemd unit / LaunchAgent / logon
//! task): after the replace the host **exits**. An app-managed host is
//! respawned by the desktop app's supervisor within its backoff; a manual
//! `unipi-host serve` prints "restarted binary; rerun unipi-host serve".
//!
//! The owner setting `update.mode` in `config.json` ("prompt" | "auto",
//! default prompt) is read by `serve`, which checks the feed every few hours.
//! `auto` installs and exits (see above); `prompt` records the check and
//! raises a [`Notice`] that `events_open` turns into a `HostNotice` for the
//! app — no desktop-app-only hook.
//!
//! Tests point the feed at a local manifest with `UNIPI_UPDATE_FEED` (a
//! `file://` URL works, so no server is needed) and the target at a temp copy
//! with `--exe`; the release-origin check is relaxed only for an explicit feed
//! override.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::data_dir::HostPaths;
use crate::instance::Mode;

/// The oino/UniPi release repository (dist.md §8: public GitHub Releases).
pub const ORIGIN: &str = "https://github.com/Neuron-Mr-White/Oino";
/// `release-manifest.json` of the latest release (the same feed `oino update` reads).
pub const FEED_URL: &str =
    "https://github.com/Neuron-Mr-White/Oino/releases/latest/download/release-manifest.json";
/// Overrides [`FEED_URL`] (tests, mirrors); relaxes the origin check.
pub const FEED_ENV: &str = "UNIPI_UPDATE_FEED";
/// The host's file name inside a release (`artifact.name`).
pub const ARTIFACT: &str = "unipi-host";
/// How often `serve` checks the feed (the first check is a minute after start).
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);
/// [`Notice::kind`] for "a newer host is on the feed" (prompt mode).
pub const NOTICE_UPDATE: &str = "update-available";

/// One binary in `release-manifest.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Artifact {
    /// `oino` | `unipi-host` | `oino-kanboard` | `unipi-kanboard`.
    #[serde(default)]
    pub name: String,
    /// Rust target triple (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, …).
    pub target: String,
    /// Download URL.
    pub url: String,
    /// Lower-case hex sha256.
    #[serde(default)]
    pub sha256: String,
}

/// `release-manifest.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Release {
    /// Release version (tag without `v`).
    pub version: String,
    /// Released binaries.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

/// `config.json` → `update` block (owner setting).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Tell the owner (`events_open` → `HostNotice`, `unipi-host update --check`); never act alone.
    #[default]
    Prompt,
    /// `serve` installs a newer host by itself, exits, and the app restarts it.
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

/// What `serve` last saw in the feed (`<host dir>/update.json`), also read by
/// `unipi-host sharing status --json`.
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

/// This build's Rust target triple, the key in `artifacts[].target`
/// (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`).
pub fn current_target() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "linux" => format!("{arch}-unknown-linux-gnu"),
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        other => format!("{arch}-{other}"),
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

/// Parses `release-manifest.json`. With the default feed, binaries must come
/// from this repo's Releases (a tampered manifest can't point elsewhere).
pub fn parse_manifest(bytes: &[u8], strict_origin: bool) -> anyhow::Result<Release> {
    let release: Release = serde_json::from_slice(bytes)
        .map_err(|e| anyhow::anyhow!("the release manifest is not valid JSON: {e}"))?;
    anyhow::ensure!(
        parse_version(&release.version).is_some(),
        "the release manifest has a bad version {:?}",
        release.version
    );
    for artifact in &release.artifacts {
        anyhow::ensure!(
            artifact.sha256.len() == 64 && artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "bad sha256 for {} ({})",
            artifact.name,
            artifact.target
        );
        if strict_origin {
            anyhow::ensure!(
                artifact.url.starts_with(&format!("{ORIGIN}/releases/")),
                "refusing {} from {}",
                artifact.target,
                artifact.url
            );
        }
    }
    Ok(release)
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent(concat!("unipi-host/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Reads a feed or artifact URL: `file://` (tests, mirrors) or https.
async fn read_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| anyhow::anyhow!("{path}: {e}"));
    }
    let res = client()?
        .get(url)
        .header("cache-control", "no-cache")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("couldn't reach {url}: {e}"))?;
    anyhow::ensure!(res.status().is_success(), "{url}: HTTP {}", res.status());
    Ok(res.bytes().await?.to_vec())
}

/// [`fetch_feed`] for an explicit URL (its origin is not checked).
pub async fn fetch_feed_from(url: &str) -> anyhow::Result<Release> {
    parse_manifest(&read_bytes(url).await?, false)
}

pub async fn fetch_feed() -> anyhow::Result<Release> {
    let (url, custom) = feed_url();
    if custom {
        return fetch_feed_from(&url).await;
    }
    // 20 s, not the 300 s artifact timeout: a check must not hang `serve`.
    let res = client()?
        .get(&url)
        .header("cache-control", "no-cache")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("couldn't reach {url}: {e}"))?;
    anyhow::ensure!(res.status().is_success(), "{url}: HTTP {}", res.status());
    parse_manifest(&res.bytes().await?, true)
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
        file: Artifact,
    },
    NoBuild {
        latest: String,
        target: String,
    },
}

/// Is `release` newer than `current`, and does it have this target's host?
pub fn decide(release: &Release, current: &str, target: &str) -> Check {
    if !is_newer(&release.version, current) {
        return Check::UpToDate {
            current: current.into(),
            latest: release.version.clone(),
        };
    }
    match release
        .artifacts
        .iter()
        .find(|a| a.target == target && (a.name == ARTIFACT || a.name.is_empty()))
    {
        Some(file) => Check::Available {
            current: current.into(),
            latest: release.version.clone(),
            file: file.clone(),
        },
        None => Check::NoBuild {
            latest: release.version.clone(),
            target: target.into(),
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
    file: &Artifact,
    expect_version: &str,
    exe: &Path,
) -> anyhow::Result<()> {
    let bytes = read_bytes(&file.url)
        .await
        .map_err(|e| anyhow::anyhow!("download failed: {e}"))?;
    let got = sha256_hex(&bytes);
    anyhow::ensure!(
        got.eq_ignore_ascii_case(&file.sha256),
        "checksum mismatch (expected {}, got {got}) — nothing changed",
        file.sha256
    );
    install_bytes(&bytes, expect_version, exe)
}

/// Writes `bytes` as a sibling of `exe`, verifies `--version`, then replaces `exe`.
/// Runs `<binary> --version` once, retrying only a *spawn* failure a few
/// times: under heavy process churn (parallel tests, a busy machine)
/// fork/posix_spawn can come back transiently refused (EAGAIN →
/// `WouldBlock`) or the just-written binary can still be open for write
/// (ETXTBSY), and a self-updater should ride those out, not report them.
fn run_version(binary: &Path) -> anyhow::Result<std::process::Output> {
    let mut last = None;
    for attempt in 0..3 {
        match std::process::Command::new(binary).arg("--version").output() {
            Ok(output) => return Ok(output),
            Err(e) => {
                let transient = matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::ResourceBusy
                ) || e.raw_os_error() == Some(26 /* ETXTBSY */);
                last = Some(e);
                if !transient || attempt == 2 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
    }
    Err(anyhow::anyhow!(
        "the new binary doesn't run here: {}",
        last.expect("a failed spawn always sets last")
    ))
}

/// Runs the new binary's `--version` and refuses anything but
/// `expect_version` (the advertised one).
fn version_check(binary: &Path, expect_version: &str) -> anyhow::Result<()> {
    let out = run_version(binary)?;
    let said = String::from_utf8_lossy(&out.stdout);
    let said = said.split_whitespace().nth(1).unwrap_or("").to_string();
    if said != expect_version {
        return Err(anyhow::anyhow!(
            "the new binary says {said:?}, expected {expect_version} — nothing changed"
        ));
    }
    Ok(())
}

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
    version_check(&tmp, expect_version).map_err(cleanup)?;
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

/// What the host says after it replaced its own binary (host.md §9.7): there
/// is no service to restart — it exits, and whoever started it starts the new
/// one (the app's supervisor, or the owner by hand).
pub fn restart_hint(mode: Mode) -> String {
    match mode {
        Mode::App => "updated unipi-host; exiting so the app starts the new binary".to_string(),
        Mode::Manual => "restarted binary; rerun unipi-host serve".to_string(),
    }
}

/// A pending host→app notice (host.md §9.7): `serve` raises it in `prompt`
/// mode and `events_open` (WP-H4) drains it as a `HostNotice`. Kept as a small
/// JSON file so a check that finds an update before the app connects is not
/// lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// [`NOTICE_UPDATE`]; further kinds arrive with the events stream.
    pub kind: String,
    /// One line for the app to show.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

pub fn notices_path(paths: &HostPaths) -> PathBuf {
    paths.root.join("notices.json")
}

/// The notice a `prompt`-mode check raises, or `None` when there is nothing
/// newer (or no build for this target). Pure, so the text is testable.
pub fn prompt_notice(check: &Check) -> Option<Notice> {
    match check {
        Check::Available {
            current, latest, ..
        } => Some(Notice {
            kind: NOTICE_UPDATE.into(),
            message: format!("UniPi host {latest} is available (this is {current})."),
            version: Some(latest.clone()),
        }),
        _ => None,
    }
}

/// Queues `notice` for the next `events_open` (newest last, duplicates of the
/// same kind+version collapse).
pub fn raise_notice(paths: &HostPaths, notice: Notice) -> anyhow::Result<()> {
    let mut queued = pending_notices(paths);
    queued.retain(|n| n.kind != notice.kind || n.version != notice.version);
    queued.push(notice);
    let keep = queued.len().saturating_sub(16);
    queued.drain(..keep);
    crate::data_dir::write_json(&notices_path(paths), &queued)
}

/// What `events_open` would deliver next (the file stays).
pub fn pending_notices(paths: &HostPaths) -> Vec<Notice> {
    crate::data_dir::read_json::<Vec<Notice>>(&notices_path(paths))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// What `events_open` delivers and clears (host.md §9.7).
pub fn take_notices(paths: &HostPaths) -> Vec<Notice> {
    let queued = pending_notices(paths);
    if !queued.is_empty() {
        let _ = std::fs::remove_file(notices_path(paths));
    }
    queued
}

/// Records a check in `update.json` (errors are kept so the app can say why).
pub fn record(paths: &HostPaths, now: u64, result: &anyhow::Result<Release>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The feed as `cargo xtask dist publish` writes it.
    fn manifest(version: &str, target: &str, url: &str, sha256: &str) -> String {
        serde_json::json!({
            "version": version,
            "artifacts": [
                { "name": "oino", "target": target, "url": format!("{ORIGIN}/releases/download/v{version}/oino-{target}"), "sha256": "c".repeat(64) },
                { "name": ARTIFACT, "target": target, "url": url, "sha256": sha256 },
            ]
        })
        .to_string()
    }

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

    #[test]
    fn manifest_parses_and_checks_origin() {
        let good = format!(
            "{ORIGIN}/releases/download/v2.0.0/unipi-host-{t}",
            t = current_target()
        );
        let ok = parse_manifest(
            manifest("2.0.0", &current_target(), &good, &"b".repeat(64)).as_bytes(),
            true,
        )
        .unwrap();
        assert_eq!(ok.version, "2.0.0");
        assert_eq!(ok.artifacts.len(), 2);
        // A tampered manifest can't point the binary somewhere else.
        assert!(parse_manifest(
            manifest(
                "2.0.0",
                &current_target(),
                "https://evil.example/x",
                &"b".repeat(64)
            )
            .as_bytes(),
            true
        )
        .is_err());
        // An explicit feed (tests, mirrors) is not origin-checked.
        assert!(parse_manifest(
            manifest("2.0.0", &current_target(), "file:///tmp/x", &"b".repeat(64)).as_bytes(),
            false
        )
        .is_ok());
        assert!(parse_manifest(b"{}", true).is_err());
        assert!(parse_manifest(b"nope", true).is_err());
        let bad_sha = manifest("2.0.0", &current_target(), &good, &"b".repeat(64))
            .replace(&"b".repeat(64), "zz");
        assert!(parse_manifest(bad_sha.as_bytes(), true).is_err());
    }

    #[test]
    fn decide_picks_the_host_artifact_of_this_target() {
        let r = parse_manifest(
            manifest(
                "2.0.0",
                &current_target(),
                "https://evil.example/x",
                &"b".repeat(64),
            )
            .as_bytes(),
            false,
        )
        .unwrap();
        match decide(&r, "1.0.0", &current_target()) {
            Check::Available { file, latest, .. } => {
                assert_eq!(file.name, ARTIFACT, "the host, not the oino CLI");
                assert_eq!(latest, "2.0.0");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            decide(&r, "2.0.0", &current_target()),
            Check::UpToDate { .. }
        ));
        assert!(matches!(
            decide(&r, "3.0.0", &current_target()),
            Check::UpToDate { .. }
        ));
        // Another target has no build.
        match decide(&r, "1.0.0", "aarch64-unknown-linux-gnu") {
            Check::NoBuild { target, .. } => assert_eq!(target, "aarch64-unknown-linux-gnu"),
            other => panic!("{other:?}"),
        }
        // A manifest with only the CLI never installs a wrong binary.
        let cli_only = serde_json::json!({
            "version": "2.0.0",
            "artifacts": [{ "name": "oino", "target": current_target(), "url": "https://x/y", "sha256": "c".repeat(64) }]
        })
        .to_string();
        let r = parse_manifest(cli_only.as_bytes(), false).unwrap();
        assert!(matches!(
            decide(&r, "1.0.0", &current_target()),
            Check::NoBuild { .. }
        ));
    }

    /// The faux feed end to end: a local `release-manifest.json` + a local
    /// "binary", both `file://`; the check finds it, the download replaces the
    /// target, and the restart hint says who restarts the host (§9.7).
    #[tokio::test]
    async fn faux_feed_installs_a_newer_host_and_says_how_to_restart() {
        let dir = tempfile::tempdir().unwrap();
        let target = current_target();
        let script = b"#!/bin/sh\necho unipi-host 9.9.9\n";
        let artifact = dir.path().join("unipi-host-9.9.9");
        std::fs::write(&artifact, script).unwrap();
        let exe = dir.path().join("unipi-host");
        std::fs::write(&exe, "old").unwrap();
        let feed = dir.path().join("release-manifest.json");
        std::fs::write(
            &feed,
            manifest(
                "9.9.9",
                &target,
                &format!("file://{}", artifact.display()),
                &sha256_hex(script),
            ),
        )
        .unwrap();
        let url = format!("file://{}", feed.display());

        let release = fetch_feed_from(&url).await.unwrap();
        assert_eq!(release.version, "9.9.9");
        let Check::Available { file, latest, .. } =
            decide(&release, env!("CARGO_PKG_VERSION"), &target)
        else {
            panic!("the faux feed must be newer than this build");
        };
        download_and_replace(&file, &latest, &exe).await.unwrap();
        assert_eq!(
            std::fs::read(&exe).unwrap(),
            script,
            "the target was replaced"
        );
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".new-"))
                .count(),
            0,
            "no temp files left"
        );

        // No service any more: the host exits and says who starts it again.
        assert_eq!(
            restart_hint(Mode::Manual),
            "restarted binary; rerun unipi-host serve"
        );
        assert!(restart_hint(Mode::App).contains("the app starts the new binary"));
        assert!(!restart_hint(Mode::App).contains("rerun"));

        // A checksum that doesn't match changes nothing.
        let bad = Artifact {
            sha256: "0".repeat(64),
            ..file
        };
        assert!(download_and_replace(&bad, &latest, &exe).await.is_err());
        assert_eq!(
            std::fs::read(&exe).unwrap(),
            script,
            "the old binary is untouched"
        );

        // `serve` records what it saw next to the data dir.
        let paths = HostPaths::from_dir(dir.path().join("host").as_path());
        record(&paths, 7, &Ok(release));
        assert_eq!(status(&paths).latest.as_deref(), Some("9.9.9"));
        assert!(status(&paths).available);
    }

    #[test]
    fn prompt_mode_queues_one_update_notice() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(dir.path());
        let check = Check::Available {
            current: "1.0.0".into(),
            latest: "2.0.0".into(),
            file: Artifact {
                name: ARTIFACT.into(),
                target: current_target(),
                url: "https://x/y".into(),
                sha256: "b".repeat(64),
            },
        };
        let notice = prompt_notice(&check).unwrap();
        assert_eq!(notice.kind, NOTICE_UPDATE);
        assert!(notice.message.contains("2.0.0") && notice.message.contains("1.0.0"));
        // Two checks before the app connects: one notice, newest last.
        raise_notice(&paths, notice.clone()).unwrap();
        raise_notice(&paths, notice.clone()).unwrap();
        assert_eq!(pending_notices(&paths), vec![notice.clone()]);
        // `events_open` takes it once and clears it.
        assert_eq!(take_notices(&paths), vec![notice]);
        assert!(take_notices(&paths).is_empty());
        // Auto mode / no build / up to date raise nothing.
        assert!(prompt_notice(&Check::UpToDate {
            current: "2.0.0".into(),
            latest: "2.0.0".into()
        })
        .is_none());
        assert!(prompt_notice(&Check::NoBuild {
            latest: "2.0.0".into(),
            target: current_target()
        })
        .is_none());
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
            &Ok(Release {
                version: "999.0.0".into(),
                artifacts: Default::default(),
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
