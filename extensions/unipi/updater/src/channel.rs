//! Release channels and GitHub release selection (unipi `src/channel.ts`, `src/checker.ts`).
//!
//! unipi followed an npm dist-tag; Oino follows GitHub releases of
//! `Neuron-Mr-White/Oino` (unipi-port §3.23). The channel is the installed version's
//! prerelease id (`1.0.0-alpha.3` → `alpha`), else `latest`. A prerelease channel also
//! considers the newest stable release so a graduation is offered; a stable install never
//! sees prereleases. Downgrades are never offered.

use serde::Deserialize;
use unipi_kit::fmt::{compare_versions, is_newer_version};

/// GitHub repository of Oino releases.
pub(crate) const REPO: &str = "Neuron-Mr-White/Oino";
/// Releases API base.
pub(crate) const API_BASE: &str = "https://api.github.com/repos/Neuron-Mr-White/Oino";

/// `1.0.0-alpha.36` → `alpha`; stable or numeric prerelease → `None`.
pub(crate) fn prerelease_channel(version: &str) -> Option<String> {
    let v = version.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    let (core, pre) = v.split_once('-')?;
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let id: String = pre
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if id.is_empty() || id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(id)
}

/// The channel an install of `version` follows.
pub(crate) fn resolve_channel(version: &str) -> String {
    prerelease_channel(version).unwrap_or_else(|| "latest".to_owned())
}

/// One GitHub release (subset of the REST payload).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct Release {
    /// Tag (`v1.2.3`).
    #[serde(default)]
    pub tag_name: String,
    /// Draft release.
    #[serde(default)]
    pub draft: bool,
    /// Marked prerelease.
    #[serde(default)]
    pub prerelease: bool,
    /// Release notes (markdown).
    #[serde(default)]
    pub body: Option<String>,
    /// Release page.
    #[serde(default)]
    pub html_url: String,
}

impl Release {
    /// Version without the `v` prefix.
    pub(crate) fn version(&self) -> &str {
        self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name)
    }
}

/// The channel a release belongs to (`latest` for stable tags).
pub(crate) fn release_channel(r: &Release) -> String {
    prerelease_channel(r.version()).unwrap_or_else(|| "latest".to_owned())
}

/// unipi `pickChannelVersion` over releases: the newest non-draft release of `channel`, or
/// the newest stable release when that is newer (graduation). `None` when nothing fits.
pub(crate) fn pick_release<'a>(releases: &'a [Release], channel: &str) -> Option<&'a Release> {
    let newest = |ch: &str| {
        releases
            .iter()
            .filter(|r| !r.draft && !r.tag_name.is_empty() && release_channel(r) == ch)
            .max_by(|a, b| compare_versions(a.version(), b.version()))
    };
    let own = newest(channel);
    let stable = if channel == "latest" {
        None
    } else {
        newest("latest")
    };
    match (own, stable) {
        (Some(a), Some(b)) => Some(if compare_versions(b.version(), a.version()).is_gt() {
            b
        } else {
            a
        }),
        (a, b) => a.or(b),
    }
}

/// unipi `pickInstallTag`: the channel of the picked release (ties go to `channel`).
pub(crate) fn install_tag(picked: Option<&Release>, channel: &str) -> String {
    picked.map_or_else(|| channel.to_owned(), release_channel)
}

/// unipi `UpdateCheckResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckResult {
    /// A newer release exists.
    pub update_available: bool,
    /// Latest version found (`""` when the check failed with no cache).
    pub latest_version: String,
    /// Installed version.
    pub current_version: String,
    /// Channel followed.
    pub channel: String,
    /// Channel the offered version comes from.
    pub install_tag: String,
    /// Release page of the offered version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    /// Failure message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CheckResult {
    /// unipi `toUpdateResult`: never reports a downgrade as an update.
    pub(crate) fn new(latest: &str, current: &str, channel: &str, install_tag: &str) -> Self {
        Self {
            update_available: is_newer_version(latest, current),
            latest_version: latest.to_owned(),
            current_version: current.to_owned(),
            channel: channel.to_owned(),
            install_tag: install_tag.to_owned(),
            release_url: None,
            error: None,
        }
    }
}

/// How the user updates Oino (no in-extension install, unipi-port §3.23).
pub(crate) fn update_instructions(latest: &str) -> String {
    format!(
        "Oino {latest} is available.\n\n\
         - Terminal: run `oino update`.\n\
         - Oino Desktop: use the built-in updater (it checks the same release feed).\n\
         - Manual: download it from https://github.com/{REPO}/releases/tag/v{latest}\n\n\
         Restart Oino after updating."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            ..Release::default()
        }
    }

    #[test]
    fn prerelease_ids() {
        assert_eq!(
            prerelease_channel("3.0.0-alpha.36").as_deref(),
            Some("alpha")
        );
        assert_eq!(prerelease_channel("v3.0.0-beta").as_deref(), Some("beta"));
        assert_eq!(prerelease_channel("3.0.0-rc.1").as_deref(), Some("rc"));
        assert_eq!(prerelease_channel("3.0.0"), None);
        assert_eq!(
            prerelease_channel("3.0.0-1"),
            None,
            "numeric id is no channel"
        );
        assert_eq!(prerelease_channel("nonsense"), None);
        assert_eq!(resolve_channel("1.0.0"), "latest");
        assert_eq!(resolve_channel("1.0.0-alpha.2"), "alpha");
    }

    #[test]
    fn picks_channel_release_and_graduation() {
        let releases = vec![
            rel("v2.9.0"),
            rel("v3.0.0-alpha.36"),
            rel("v3.0.0-alpha.40"),
            rel("v3.0.0-beta.1"),
        ];
        assert_eq!(
            pick_release(&releases, "alpha").map(Release::version),
            Some("3.0.0-alpha.40")
        );
        assert_eq!(
            pick_release(&releases, "latest").map(Release::version),
            Some("2.9.0"),
            "a stable install never sees prereleases"
        );
        let graduated = [releases.clone(), vec![rel("v3.0.0")]].concat();
        let picked = pick_release(&graduated, "alpha");
        assert_eq!(picked.map(Release::version), Some("3.0.0"));
        assert_eq!(
            install_tag(picked, "alpha"),
            "latest",
            "graduation installs latest"
        );
        assert_eq!(
            install_tag(pick_release(&releases, "alpha"), "alpha"),
            "alpha"
        );
        assert_eq!(pick_release(&[], "alpha"), None);
        let drafts = vec![Release {
            draft: true,
            ..rel("v9.0.0")
        }];
        assert_eq!(pick_release(&drafts, "latest"), None, "drafts are ignored");
    }

    #[test]
    fn never_offers_a_downgrade() {
        assert!(!CheckResult::new("2.0.4", "2.0.5", "latest", "latest").update_available);
        assert!(CheckResult::new("2.0.6", "2.0.5", "latest", "latest").update_available);
        assert!(!CheckResult::new("v2.0.5", "2.0.5", "latest", "latest").update_available);
        assert!(CheckResult::new("3.0.0", "3.0.0-alpha.0", "alpha", "latest").update_available);
        assert!(!CheckResult::new("3.0.0-alpha.0", "3.0.0", "latest", "latest").update_available);
    }

    #[test]
    fn version_order_matches_unipi() {
        use std::cmp::Ordering::{Equal, Greater};
        assert_eq!(compare_versions("2.0.10", "2.0.9"), Greater);
        assert_eq!(compare_versions("2.10.0", "2.9.9"), Greater);
        assert_eq!(compare_versions("3.0.0-alpha.10", "3.0.0-alpha.9"), Greater);
        assert_eq!(compare_versions("3.0.0-beta.0", "3.0.0-alpha.7"), Greater);
        assert_eq!(compare_versions("3.0.0-rc.1", "3.0.0-beta.9"), Greater);
        assert_eq!(compare_versions("3.0.0", "3.0.0-alpha.0"), Greater);
        assert_eq!(compare_versions("3.0.0-alpha.0", "3.0.0-alpha.0"), Equal);
    }

    #[test]
    fn instructions_name_the_release() {
        let t = update_instructions("1.2.3");
        assert!(t.contains("oino update"));
        assert!(t.contains("releases/tag/v1.2.3"));
    }
}
