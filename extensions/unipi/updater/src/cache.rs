//! Last-check cache (unipi `src/cache.ts`): `/cache/last-check.json`, same shape as unipi's
//! `~/.unipi/cache/updater/last-check.json`.

use std::path::{Path, PathBuf};
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// unipi `LastCheckCache`.
#[allow(clippy::struct_field_names, reason = "unipi field names")]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LastCheck {
    /// ISO timestamp of the last check.
    pub last_check: String,
    /// Latest version found.
    pub latest_version: String,
    /// Version the user chose to skip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dismissals: BTreeMap<String, Dismissal>,
    /// Channel the check followed (absent = `latest`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Channel the offered version comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_tag: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Dismissal {
    pub skipped: Option<String>,
    pub snoozed: Option<String>,
    pub until: Option<i64>,
}
pub(crate) const SNOOZE_MS: i64 = 24 * 60 * 60 * 1000;

impl LastCheck {
    /// Channel with the pre-channel default.
    pub(crate) fn channel(&self) -> &str {
        self.channel.as_deref().unwrap_or("latest")
    }
}

/// Cache file path.
pub(crate) fn path() -> PathBuf {
    unipi_kit::paths::cache("last-check.json")
}

/// unipi `readLastCheck`: `None` when missing, invalid, or lacking `lastCheck`/`latestVersion`.
pub(crate) fn read(path: &Path) -> Option<LastCheck> {
    let c: LastCheck = unipi_kit::paths::read_json(path)?;
    (!c.last_check.is_empty() && !c.latest_version.is_empty()).then_some(c)
}

/// unipi `writeLastCheck`.
pub(crate) fn write(path: &Path, c: &LastCheck) {
    #[cfg(target_arch = "wasm32")]
    let _ = unipi_kit::paths::write_json(path, c);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::fs::write(path, serde_json::to_vec(c).unwrap_or_default());
}

/// unipi `isCheckDue`.
pub(crate) fn is_check_due(cache: Option<&LastCheck>, interval_ms: u64, now_ms: i64) -> bool {
    let Some(c) = cache else { return true };
    let Some(at) = parse_iso_ms(&c.last_check) else {
        return true;
    };
    now_ms.saturating_sub(at) >= i64::try_from(interval_ms).unwrap_or(i64::MAX)
}

/// unipi `writeSkippedVersion` (only when a cache exists).
pub(crate) fn skip_version(path: &Path, channel: &str, version: &str) {
    if let Some(mut c) = read(path) {
        c.dismissals.entry(channel.to_owned()).or_default().skipped = Some(version.to_owned());
        write(path, &c);
    }
}

pub(crate) fn snooze(path: &Path, channel: &str, version: &str, now: i64) {
    if let Some(mut c) = read(path) {
        let d = c.dismissals.entry(channel.to_owned()).or_default();
        d.snoozed = Some(version.to_owned());
        d.until = Some(now.saturating_add(SNOOZE_MS));
        write(path, &c);
    }
}

pub(crate) fn is_dismissed(c: Option<&LastCheck>, version: &str, channel: &str, now: i64) -> bool {
    c.is_some_and(|c| {
        let legacy = c.channel() == channel && (c.skipped_version.as_deref() == Some(version)
            || (c.snoozed_version.as_deref() == Some(version) && c.snoozed_until.is_some_and(|until| now < until)));
        legacy || c.dismissals.get(channel).is_some_and(|d| d.skipped.as_deref() == Some(version)
            || (d.snoozed.as_deref() == Some(version) && d.until.is_some_and(|until| now < until)))
    })
}

pub(crate) fn clear_current(path: &Path, channel: &str) {
    if let Some(mut c) = read(path) {
        c.dismissals.remove(channel);
        if c.channel() == channel { c.skipped_version = None; c.snoozed_version = None; c.snoozed_until = None; }
        write(path, &c);
    }
}

/// Epoch ms of an ISO-8601 UTC timestamp (`2026-10-26T06:22:37.855Z`, offsets honoured).
#[allow(clippy::many_single_char_names, reason = "civil-date arithmetic")]
pub(crate) fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let (clock, offset_min) = if let Some(c) = time.strip_suffix('Z') {
        (c, 0)
    } else if let Some(pos) = time.rfind(['+', '-']) {
        let (c, off) = time.split_at(pos);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let off = &off[1..];
        let (h, mm) = off.split_once(':').unwrap_or((off, "0"));
        (
            c,
            sign * (h.parse::<i64>().ok()? * 60 + mm.parse::<i64>().ok()?),
        )
    } else {
        (time, 0)
    };
    let mut parts = clock.split(':');
    let hh: i64 = parts.next()?.parse().ok()?;
    let mi: i64 = parts.next()?.parse().ok()?;
    let sec_s = parts.next().unwrap_or("0");
    let (sec, frac) = sec_s.split_once('.').unwrap_or((sec_s, "0"));
    let ss: i64 = sec.parse().ok()?;
    let ms: i64 = format!("{frac:0<3}")[..3].parse().ok()?;
    // Howard Hinnant's days_from_civil.
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + hh * 3600 + mi * 60 + ss) - offset_min * 60) * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_round_trip() {
        for ms in [0_i64, 1_761_459_757_855, 951_782_400_000] {
            assert_eq!(parse_iso_ms(&unipi_kit::fmt::iso_from_ms(ms)), Some(ms));
        }
        assert_eq!(
            parse_iso_ms("1970-01-01T01:00:00+01:00"),
            Some(0),
            "offsets are honoured"
        );
        assert_eq!(parse_iso_ms("garbage"), None);
    }

    #[test]
    fn check_due_and_skip() {
        let now = 10_000_000_i64;
        let c = LastCheck {
            last_check: unipi_kit::fmt::iso_from_ms(now - 1000),
            latest_version: "1.0.0".into(),
            ..LastCheck::default()
        };
        assert!(is_check_due(None, 3_600_000, now));
        assert!(!is_check_due(Some(&c), 3_600_000, now));
        assert!(is_check_due(Some(&c), 500, now));
        assert!(!is_dismissed(Some(&c), "1.0.0", "latest", now));
        let skipped = LastCheck {
            skipped_version: Some("1.0.0".into()),
            ..c
        };
        assert!(is_dismissed(Some(&skipped), "1.0.0", "latest", now));
        assert_eq!(skipped.channel(), "latest");
    }

    #[test]
    fn dismissal_is_channel_and_time_scoped() {
        let c = LastCheck { skipped_version: Some("1.0.2".into()), snoozed_version: Some("1.0.3".into()), snoozed_until: Some(200), ..LastCheck::default() };
        assert!(is_dismissed(Some(&c), "1.0.2", "latest", 500));
        assert!(!is_dismissed(Some(&c), "1.0.2", "alpha", 0));
        assert!(is_dismissed(Some(&c), "1.0.3", "latest", 199));
        assert!(!is_dismissed(Some(&c), "1.0.3", "latest", 200));
    }

    #[test]
    fn persisted_skip_and_snooze_survive_reload_and_expire() {
        let dir = std::env::temp_dir().join(format!("oin38-dismiss-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        write(&path, &LastCheck { last_check: "2026-01-01T00:00:00Z".into(), latest_version: "1.0.2".into(), ..LastCheck::default() });
        skip_version(&path, "latest", "1.0.2");
        snooze(&path, "alpha", "2.0.0-alpha.1", 100);
        assert!(is_dismissed(read(&path).as_ref(), "1.0.2", "latest", 999));
        assert!(is_dismissed(read(&path).as_ref(), "2.0.0-alpha.1", "alpha", 100 + SNOOZE_MS - 1));
        assert!(!is_dismissed(read(&path).as_ref(), "2.0.0-alpha.1", "alpha", 100 + SNOOZE_MS));
        clear_current(&path, "latest");
        assert!(!is_dismissed(read(&path).as_ref(), "1.0.2", "latest", 999));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cache_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("unipi-updater-cache-{}", std::process::id()));
        let file = dir.join("last-check.json");
        let _ = std::fs::create_dir_all(&dir);
        assert!(read(&file).is_none());
        let _ = std::fs::write(&file, br#"{"lastCheck":"2026-01-01T00:00:00.000Z","latestVersion":"1.2.3","channel":"alpha"}"#);
        let c = read(&file).unwrap_or_default();
        assert_eq!(c.latest_version, "1.2.3");
        assert_eq!(c.channel(), "alpha");
        let _ = std::fs::write(&file, br#"{"lastCheck":"","latestVersion":"1.2.3"}"#);
        assert!(read(&file).is_none(), "incomplete caches are ignored");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
