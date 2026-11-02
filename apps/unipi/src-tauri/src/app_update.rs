//! Desktop app self-update (UNI-263), desktop builds only.
//!
//! The feed is `https://oino.dev/app/desktop/latest.json` in the Tauri v2
//! updater shape (`{version, notes, pub_date, platforms{"linux-x86_64":
//! {url, signature}, …}}`, written by `scripts/release.sh`). Artifacts are
//! signed with the minisign key in `~/Projects/Neuron/credentials/
//! unipi-updater/`; the public key is `plugins.updater.pubkey` in
//! tauri.conf.json, so a download that isn't signed by it is refused.
//!
//! The UI (lib/desktopUpdate.ts) checks on start and every few hours and
//! shows a prompt; nothing installs until the user says so. Install support:
//! - AppImage (Linux), .app (macOS), NSIS (Windows): download, verify,
//!   replace, relaunch.
//! - .deb / .rpm / a plain binary (tar.gz): the plugin can't replace those
//!   without root, so `kind` is "manual" and the UI links the download page.
//!
//! `UNIPI_DESKTOP_UPDATE_FEED` overrides the endpoint (tests; the
//! end-to-end check runs a local feed with a fake newer version).

use serde::Serialize;
use tauri::ipc::Channel;
use tauri_plugin_updater::UpdaterExt;
use unipi_proto::{AppErrorCode, AppErrorDto};

#[allow(dead_code)] // the endpoint itself is in tauri.conf.json (asserted by a test)
pub const FEED_URL: &str = "https://oino.dev/app/desktop/latest.json";
pub const FEED_ENV: &str = "UNIPI_DESKTOP_UPDATE_FEED";
pub const DOWNLOAD_PAGE: &str = "https://oino.dev/download/";

/// How this copy was installed, i.e. whether it can update itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// AppImage, macOS .app or Windows NSIS: in-place update + relaunch.
    InApp,
    /// .deb/.rpm/tar.gz or a dev build: show "download the new version".
    Manual,
}

/// `kind` from the bundle type the bundler patched into the binary and, on
/// Linux, whether we run from an AppImage.
pub fn install_kind(
    bundle: Option<tauri::utils::config::BundleType>,
    appimage_env: bool,
) -> InstallKind {
    use tauri::utils::config::BundleType as B;
    match bundle {
        Some(B::AppImage) if appimage_env || !cfg!(target_os = "linux") => InstallKind::InApp,
        Some(B::App | B::Dmg) => InstallKind::InApp,
        Some(B::Nsis | B::Msi) => InstallKind::InApp,
        _ => InstallKind::Manual,
    }
}

fn current_kind() -> InstallKind {
    install_kind(
        tauri::utils::platform::bundle_type(),
        std::env::var_os("APPIMAGE").is_some(),
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub kind: InstallKind,
    /// Newer version, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Where a manual install gets the new version.
    pub download_page: String,
}

fn err(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, message, true)
}

fn updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, AppErrorDto> {
    let mut builder = app.updater_builder();
    if let Some(feed) = std::env::var(FEED_ENV).ok().filter(|f| !f.is_empty()) {
        let url = feed
            .parse()
            .map_err(|e| err(format!("bad {FEED_ENV}: {e}")))?;
        builder = builder
            .endpoints(vec![url])
            .map_err(|e| err(e.to_string()))?;
    }
    builder.build().map_err(|e| err(format!("updater: {e}")))
}

/// Looks at the feed. Never installs anything.
#[tauri::command]
pub async fn app_update_check(app: tauri::AppHandle) -> Result<UpdateInfo, AppErrorDto> {
    let current = app.package_info().version.to_string();
    let kind = current_kind();
    let update = updater(&app)?
        .check()
        .await
        .map_err(|e| err(format!("Update check failed: {e}")))?;
    Ok(UpdateInfo {
        current,
        kind,
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update.as_ref().and_then(|u| u.body.clone()),
        date: update.as_ref().and_then(|u| u.date.map(|d| d.to_string())),
        download_page: DOWNLOAD_PAGE.into(),
    })
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
// `Progress::Progress` is the wire's `{kind:"progress"}` the UI already reads.
#[allow(clippy::enum_variant_names)]
pub enum Progress {
    Progress { done: u64, total: Option<u64> },
    Verifying,
    Installing,
}

/// Downloads (signature-checked by the plugin), installs and relaunches.
/// Only after the user said yes (the UI's prompt).
#[tauri::command]
pub async fn app_update_install(
    app: tauri::AppHandle,
    expect: String,
    on_progress: Channel<Progress>,
) -> Result<(), AppErrorDto> {
    if current_kind() != InstallKind::InApp {
        return Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "This copy of UniPi can't update itself",
            false,
        )
        .with_hint(format!("Download the new version from {DOWNLOAD_PAGE}")));
    }
    let update = updater(&app)?
        .check()
        .await
        .map_err(|e| err(format!("Update check failed: {e}")))?
        .ok_or_else(|| err("No update available any more"))?;
    // The user agreed to *this* version.
    if update.version != expect {
        return Err(err(format!(
            "The available version changed to {} — check again",
            update.version
        )));
    }
    let mut done = 0u64;
    let bytes = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                let _ = on_progress.send(Progress::Progress { done, total });
            },
            || {
                let _ = on_progress.send(Progress::Verifying);
            },
        )
        .await
        .map_err(|e| err(format!("Download failed: {e}")))?;
    let _ = on_progress.send(Progress::Installing);
    update
        .install(bytes)
        .map_err(|e| err(format!("Install failed: {e}")))?;
    log::info!("updated to {}; relaunching", update.version);
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::utils::config::BundleType as B;

    #[test]
    fn config_points_at_the_feed_with_a_key() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let u = &conf["plugins"]["updater"];
        assert_eq!(u["endpoints"][0], FEED_URL);
        assert!(
            u["pubkey"].as_str().is_some_and(|k| k.len() > 100),
            "updater pubkey missing"
        );
    }

    #[test]
    fn only_self_replaceable_bundles_update_in_app() {
        assert_eq!(install_kind(Some(B::Deb), false), InstallKind::Manual);
        assert_eq!(install_kind(Some(B::Rpm), false), InstallKind::Manual);
        assert_eq!(install_kind(None, false), InstallKind::Manual);
        assert_eq!(install_kind(Some(B::Nsis), false), InstallKind::InApp);
        assert_eq!(install_kind(Some(B::App), false), InstallKind::InApp);
        assert_eq!(install_kind(Some(B::AppImage), true), InstallKind::InApp);
        if cfg!(target_os = "linux") {
            // The AppImage's inner binary run directly (extracted) can't replace the image.
            assert_eq!(install_kind(Some(B::AppImage), false), InstallKind::Manual);
        }
    }
}
