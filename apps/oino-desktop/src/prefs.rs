//! Desktop preferences in `~/.oino/desktop.json` (agent settings stay in `settings.json`).

use std::path::Path;

use oino_ui::host::AppSettings;

const FILE: &str = "desktop.json";

/// Loads preferences (defaults when missing or invalid).
#[must_use]
pub(crate) fn load(agent_dir: &Path) -> AppSettings {
    std::fs::read_to_string(agent_dir.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Saves preferences atomically (write temp, rename); failures are logged.
pub(crate) fn save(agent_dir: &Path, settings: &AppSettings) {
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(agent_dir)?;
        let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
        let tmp = agent_dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, agent_dir.join(FILE))
    })();
    if let Err(error) = result {
        tracing::warn!(%error, "saving desktop preferences");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("oino-prefs-{}", std::process::id()));
        assert_eq!(load(&dir), AppSettings::default());
        let mut s = AppSettings {
            compact: true,
            ..AppSettings::default()
        };
        s.touch_cwd("/w");
        save(&dir, &s);
        assert_eq!(load(&dir), s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
