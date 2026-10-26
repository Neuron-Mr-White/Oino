//! Mobile preferences in `<agent>/mobile.json` (agent settings stay in `settings.json`):
//! the app preferences ([`AppSettings`], same shape as desktop) plus the paired desktops
//! (`remoteHosts`, design.md §6.10). A separate file from `desktop.json` so a shared agent
//! dir (tests, `adb push`) never mixes desktop and phone layouts.

use std::path::Path;

use oino_ui::host::AppSettings;
use oino_ui::mobile::RemoteHost;
use serde::{Deserialize, Serialize};

const FILE: &str = "mobile.json";

/// The file's contents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MobilePrefs {
    #[serde(flatten)]
    app: AppSettings,
    remote_hosts: Vec<RemoteHost>,
}

fn read(agent_dir: &Path) -> MobilePrefs {
    std::fs::read_to_string(agent_dir.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| MobilePrefs {
            // Phones read at 15 (design.md §6).
            app: AppSettings {
                ui_font_size: 15,
                ..AppSettings::default()
            },
            remote_hosts: Vec::new(),
        })
}

fn write(agent_dir: &Path, prefs: &MobilePrefs) {
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(agent_dir)?;
        let text = serde_json::to_string_pretty(prefs).map_err(std::io::Error::other)?;
        let tmp = agent_dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, agent_dir.join(FILE))
    })();
    if let Err(error) = result {
        tracing::warn!(%error, "saving mobile preferences");
    }
}

/// Loads the app preferences (defaults when missing or invalid).
#[must_use]
pub(crate) fn load(agent_dir: &Path) -> AppSettings {
    read(agent_dir).app
}

/// Saves the app preferences atomically (write temp, rename), keeping the paired hosts.
pub(crate) fn save(agent_dir: &Path, settings: &AppSettings) {
    let mut prefs = read(agent_dir);
    prefs.app = settings.clone();
    write(agent_dir, &prefs);
}

/// The paired desktops.
#[must_use]
pub(crate) fn load_hosts(agent_dir: &Path) -> Vec<RemoteHost> {
    read(agent_dir).remote_hosts
}

/// Saves the paired desktops, keeping the app preferences.
pub(crate) fn save_hosts(agent_dir: &Path, hosts: &[RemoteHost]) {
    let mut prefs = read(agent_dir);
    prefs.remote_hosts = hosts.to_vec();
    write(agent_dir, &prefs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_both_halves() {
        let dir = std::env::temp_dir().join(format!("oino-mobile-prefs-{}", std::process::id()));
        assert_eq!(load(&dir).ui_font_size, 15);
        let mut s = AppSettings {
            hide_thinking: false,
            ..AppSettings::default()
        };
        s.touch_cwd("/w");
        save(&dir, &s);
        let host = RemoteHost::from_url("oino://desk:7420?token=t", 1).unwrap();
        save_hosts(&dir, std::slice::from_ref(&host));
        assert_eq!(load(&dir), s);
        save(&dir, &s);
        assert_eq!(load_hosts(&dir), vec![host]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
