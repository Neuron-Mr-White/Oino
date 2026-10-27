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

/// Marker written while the opt-in mascot window opens (A2.20 S5, GA-32 hardening).
const MASCOT_GUARD: &str = "desktop.mascot-pending";

/// Startup guard for the mascot window: when the previous run died while the mascot was
/// opening (the marker is still there), the mascot is turned off instead of crash-looping.
/// Returns whether it was reset. Arms the marker again when the mascot stays on.
pub(crate) fn guard_mascot(agent_dir: &Path) -> bool {
    let marker = agent_dir.join(MASCOT_GUARD);
    let mut settings = load(agent_dir);
    if !settings.mascot_window {
        let _ = std::fs::remove_file(&marker);
        return false;
    }
    if marker.exists() {
        settings.mascot_window = false;
        save(agent_dir, &settings);
        let _ = std::fs::remove_file(&marker);
        tracing::warn!("the last run stopped while the mascot window opened; mascot turned off");
        return true;
    }
    let _ = std::fs::write(&marker, b"1");
    false
}

/// Clears the mascot marker once the app has run for a while.
pub(crate) fn clear_mascot_guard(agent_dir: &Path) {
    let _ = std::fs::remove_file(agent_dir.join(MASCOT_GUARD));
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

    #[test]
    fn mascot_guard_turns_the_mascot_off_after_a_failed_open() {
        let dir = std::env::temp_dir().join(format!("oino-mascot-{}", std::process::id()));
        save(
            &dir,
            &AppSettings {
                mascot_window: true,
                ..AppSettings::default()
            },
        );
        assert!(!guard_mascot(&dir), "first start arms the marker");
        assert!(
            guard_mascot(&dir),
            "a run that died while opening resets it"
        );
        assert!(!load(&dir).mascot_window);
        assert!(!guard_mascot(&dir));
        clear_mascot_guard(&dir);
        let _ = std::fs::remove_dir_all(dir);
    }
}
