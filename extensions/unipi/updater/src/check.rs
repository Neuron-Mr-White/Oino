//! Update check decisions (unipi `checkForUpdates`), network-free so they are unit-testable.

use unipi_kit::fmt::compare_versions;

use crate::cache::{LastCheck, is_check_due};
use crate::channel::{CheckResult, Release, install_tag, pick_release};

/// Reuses the cache when it is for the same channel, not due, and not older than the
/// installed version (right after an upgrade the cache has not seen the new release).
pub(crate) fn cached(
    cache: Option<&LastCheck>,
    current: &str,
    channel: &str,
    interval_ms: u64,
    now_ms: i64,
    force: bool,
) -> Option<CheckResult> {
    let c = cache?;
    if force || c.channel() != channel || is_check_due(Some(c), interval_ms, now_ms) {
        return None;
    }
    if compare_versions(&c.latest_version, current).is_lt() {
        return None;
    }
    Some(CheckResult::new(
        &c.latest_version,
        current,
        channel,
        c.install_tag.as_deref().unwrap_or(channel),
    ))
}

/// A fetched release list → result + the cache to write.
pub(crate) fn from_releases(
    releases: &[Release],
    previous: Option<&LastCheck>,
    current: &str,
    channel: &str,
    now_iso: &str,
) -> Result<(CheckResult, LastCheck, Option<Release>), String> {
    let picked = pick_release(releases, channel)
        .ok_or_else(|| format!("No release on channel \"{channel}\" in the GitHub response"))?;
    let tag = install_tag(Some(picked), channel);
    let mut result = CheckResult::new(picked.version(), current, channel, &tag);
    if !picked.html_url.is_empty() {
        result.release_url = Some(picked.html_url.clone());
    }
    let cache = LastCheck {
        last_check: now_iso.to_owned(),
        latest_version: picked.version().to_owned(),
        skipped_version: previous.and_then(|p| p.skipped_version.clone()),
        channel: Some(channel.to_owned()),
        install_tag: Some(tag),
    };
    Ok((result, cache, Some(picked.clone())))
}

/// Network failure: reuse cached info for this channel only; never suggest a downgrade.
pub(crate) fn on_error(
    cache: Option<&LastCheck>,
    current: &str,
    channel: &str,
    error: &str,
) -> CheckResult {
    let usable = cache.filter(|c| c.channel() == channel);
    let mut r = match usable {
        Some(c) => CheckResult::new(
            &c.latest_version,
            current,
            channel,
            c.install_tag.as_deref().unwrap_or(channel),
        ),
        None => CheckResult {
            current_version: current.to_owned(),
            channel: channel.to_owned(),
            install_tag: channel.to_owned(),
            ..CheckResult::default()
        },
    };
    r.error = Some(if error.is_empty() {
        "Unknown error".to_owned()
    } else {
        error.to_owned()
    });
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(latest: &str, channel: Option<&str>, at: i64) -> LastCheck {
        LastCheck {
            last_check: unipi_kit::fmt::iso_from_ms(at),
            latest_version: latest.into(),
            channel: channel.map(Into::into),
            ..LastCheck::default()
        }
    }

    #[test]
    fn cache_reuse_rules() {
        let now = 100_000_000;
        let fresh = cache("1.2.0", None, now - 10);
        assert!(
            cached(Some(&fresh), "1.1.0", "latest", 3_600_000, now, false)
                .is_some_and(|r| r.update_available)
        );
        assert!(cached(Some(&fresh), "1.1.0", "latest", 3_600_000, now, true).is_none());
        assert!(
            cached(Some(&fresh), "1.1.0", "alpha", 3_600_000, now, false).is_none(),
            "an old cache for another channel is not reused"
        );
        assert!(
            cached(Some(&fresh), "1.3.0", "latest", 3_600_000, now, false).is_none(),
            "a cache older than the install is refreshed"
        );
        assert!(cached(Some(&fresh), "1.1.0", "latest", 5, now, false).is_none());
    }

    #[test]
    fn release_list_to_cache() {
        let releases = vec![
            Release {
                tag_name: "v1.0.0".into(),
                html_url: "https://github.com/x/r/1".into(),
                ..Release::default()
            },
            Release {
                tag_name: "v1.1.0-alpha.1".into(),
                ..Release::default()
            },
        ];
        let prev = LastCheck {
            skipped_version: Some("0.9.0".into()),
            ..cache("0.9.0", None, 0)
        };
        let (r, c, picked) = from_releases(&releases, Some(&prev), "1.0.0-alpha.3", "alpha", "T")
            .unwrap_or_default();
        assert_eq!(r.latest_version, "1.1.0-alpha.1");
        assert!(r.update_available);
        assert_eq!(c.skipped_version.as_deref(), Some("0.9.0"), "skips survive");
        assert_eq!(c.channel.as_deref(), Some("alpha"));
        assert_eq!(c.install_tag.as_deref(), Some("alpha"));
        assert_eq!(picked.map(|p| p.tag_name), Some("v1.1.0-alpha.1".into()));
        assert!(from_releases(&[], None, "1.0.0", "latest", "T").is_err());
    }

    #[test]
    fn errors_fall_back_to_same_channel_cache() {
        let c = cache("2.0.0", Some("latest"), 0);
        let r = on_error(Some(&c), "1.0.0", "latest", "HTTP 500");
        assert!(r.update_available);
        assert_eq!(r.error.as_deref(), Some("HTTP 500"));
        let other = on_error(Some(&c), "1.0.0", "alpha", "");
        assert!(!other.update_available);
        assert_eq!(other.latest_version, "");
        assert_eq!(other.error.as_deref(), Some("Unknown error"));
        let stale = on_error(Some(&cache("0.5.0", None, 0)), "1.0.0", "latest", "x");
        assert!(
            !stale.update_available,
            "a stale cache never suggests a downgrade"
        );
    }
}
