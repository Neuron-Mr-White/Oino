//! UNI-243 (plan S4, docs/m8/SELF-INSTALL-AND-EMBEDDED-HOST.md §4.3 point 6):
//! "what is newer than what this PC has", for the Settings → This computer →
//! Components screen.
//!
//! Four sources, one shape:
//!
//! - **npm dist-tags** for `@pi-unipi/unipi` — the channel `channel_for`
//!   picks, so an alpha install is compared against alpha and a stable one
//!   against latest (§4.3, decision 5);
//! - **npm dist-tags** for `@earendil-works/pi-coding-agent` (pi);
//! - **`latest.json`'s `host` block** — the version the feed offers, next to
//!   the host this PC actually runs (`host.lock` or the bundled sidecar);
//! - **herdr** — GitHub's latest release (herdr has no `--check`).
//!
//! Everything is *reported*, never installed: the Components screen offers
//! Update / Update all, which run the same `setup_run` engine as a first
//! install. The desktop app itself is the one exception — and it is not in this
//! report either: its check is the existing signed updater (`app_update.rs`,
//! driven by `lib/desktopUpdate.ts`), so the Components screen takes the
//! desktop row from there.
//!
//! Nothing here talks to the network at import time; the fetchers are
//! injectable so tests run against a fake registry.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::{channel_for, DistTags};
use unipi_proto::AppErrorDto;

/// How the version the feed offers compares to what is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum Update {
    /// Up to date. `offered` is the version the feed had — it can be *older*
    /// than what is installed (decision 4 never downgrades a user's version),
    /// so the UI can tell "up to date" from "newer than tested".
    Current {
        installed: String,
        offered: Option<String>,
    },
    /// The feed offers something newer: `from` → `to`.
    Available { from: String, to: String },
    /// Nothing installed yet: an install, not an update.
    Missing { to: String },
    /// Couldn't tell (offline, registry unreachable). Never shown as "up to date".
    Unknown,
}

/// One component's update state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentUpdate {
    /// `node` | `pi` | `unipi` | `kanboard` | `herdr` | `host`.
    pub name: String,
    pub update: Update,
    /// Where the newer version comes from (a channel, a URL, a page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The whole picture the Components screen renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReport {
    pub components: Vec<ComponentUpdate>,
    /// True when anything needs a click.
    pub any_available: bool,
    /// True when the network couldn't be reached.
    pub offline: bool,
}

impl UpdateReport {
    pub fn get(&self, name: &str) -> Option<&ComponentUpdate> {
        self.components.iter().find(|c| c.name == name)
    }
}

// ------------------------------------------------------------------ versions

/// A version split the way semver compares it: the `(X, Y, Z)` tuple, and
/// the prerelease identifiers (`alpha.41` → `["alpha", "41"]`).
fn version_key(text: &str) -> ((u64, u64, u64), Option<Vec<String>>) {
    let trimmed = text.trim().trim_start_matches('v');
    let (core, pre) = match trimmed.split_once('-') {
        Some((core, pre)) => (core, Some(pre.to_string())),
        None => (trimmed, None),
    };
    let mut parts = core.split('.').map(|p| p.parse().unwrap_or(0));
    let tuple = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    (
        tuple,
        pre.map(|p| p.split('.').map(str::to_string).collect()),
    )
}

/// Semver's rule for two prerelease lists: identifier by identifier, numeric
/// ones compared numerically (and ranked below alphanumeric ones), then the
/// longer list wins.
fn compare_prerelease(a: &[String], b: &[String]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for (x, y) in a.iter().zip(b) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

/// Is `to` newer than `from`, numerically — with semver's prerelease rule, so
/// `3.0.0-alpha.42` is newer than `3.0.0-alpha.41` while `3.0.0` is newer
/// than any `3.0.0-…`. A plain comparison, never a semver caret.
pub fn is_newer(to: &str, from: &str) -> bool {
    use std::cmp::Ordering;
    let (to_key, to_pre) = version_key(to);
    let (from_key, from_pre) = version_key(from);
    to_key
        .cmp(&from_key)
        .then_with(|| match (&to_pre, &from_pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => compare_prerelease(x, y),
        })
        == Ordering::Greater
}

/// The update state of one component.
pub fn compare(
    name: &str,
    installed: Option<&str>,
    offered: Option<&str>,
    detail: Option<String>,
) -> ComponentUpdate {
    let update = match (installed, offered) {
        (None, Some(to)) => Update::Missing { to: to.to_string() },
        (None, None) => Update::Unknown,
        (Some(_), None) => Update::Unknown,
        (Some(from), Some(to)) => {
            let offered = Some(to.to_string());
            if is_newer(to, from) {
                Update::Available {
                    from: from.to_string(),
                    to: to.to_string(),
                }
            } else {
                // Equal, or ours is newer (decision 4 never downgrades it). The
                // offered version rides along either way, so the UI can tell
                // "up to date" from "newer than tested".
                Update::Current {
                    installed: from.to_string(),
                    offered,
                }
            }
        }
    };
    ComponentUpdate {
        name: name.to_string(),
        update,
        detail,
    }
}

// ----------------------------------------------------------------- fetching

/// The network access, injectable for tests.
pub type Fetch = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// npm's dist-tags endpoint for a package.
pub fn npm_tags_url(package: &str) -> String {
    format!("{}/{package}/dist-tags", super::NPM_TAGS_URL)
}

/// GitHub's latest-release API for a repo.
pub fn github_release_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/releases/latest")
}

/// A version out of GitHub's release JSON (`tag_name`, `v1.2.3`).
pub fn version_from_github(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = value.get("tag_name")?.as_str()?;
    (!tag.is_empty()).then(|| tag.trim().trim_start_matches('v').to_string())
}

/// The real fetcher: GET, 5 s, body as text.
pub fn http_fetch() -> Fetch {
    Arc::new(|url| {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .ok()?;
        let response = client.get(url).send().ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.text().ok()
    })
}

/// What a Components row needs to know about the world.
#[derive(Clone)]
pub struct CheckCtx {
    /// The installed versions, by component name.
    pub installed: std::collections::BTreeMap<String, Option<String>>,
    /// The pi settings spec (for the UniPi channel).
    pub unipi_spec: Option<String>,
    /// The feed fetcher (injectable in tests).
    pub get: Fetch,
}

impl CheckCtx {
    /// GET a URL's body, or None (offline, error, non-200).
    pub fn fetch(&self, url: &str) -> Option<String> {
        (self.get)(url)
    }
}

/// Builds the report. Offline (`fetch` returns nothing for everything) shows
/// as `Unknown` for every component, never as "up to date", and `offline`.
pub fn check_with(ctx: &CheckCtx) -> UpdateReport {
    let installed = |name: &str| ctx.installed.get(name).and_then(|v| v.clone());
    let mut components: Vec<ComponentUpdate> = Vec::new();

    // pi: the `latest` dist-tag (a newer user pi is left alone).
    let pi_tags = ctx
        .fetch(&npm_tags_url(super::install::PI_PACKAGE))
        .map(|body| DistTags::parse(&body));
    components.push(compare(
        "pi",
        installed("pi").as_deref(),
        pi_tags.as_ref().and_then(|t| t.latest.clone()).as_deref(),
        Some("npm latest".to_string()),
    ));

    // UniPi: the channel this install follows.
    let unipi_tags = ctx
        .fetch(&npm_tags_url("@pi-unipi/unipi"))
        .map(|body| DistTags::parse(&body))
        .unwrap_or_default();
    let channel = channel_for(
        ctx.unipi_spec.as_deref(),
        installed("unipi").as_deref(),
        &unipi_tags,
    );
    components.push(compare(
        "unipi",
        installed("unipi").as_deref(),
        offered_for_channel(&channel, ctx.unipi_spec.as_deref(), &unipi_tags).as_deref(),
        Some(format!("channel: {channel}")),
    ));

    // agent-browser: the `latest` dist-tag, like pi (no channel of its own).
    let agent_browser_tags = ctx
        .fetch(&npm_tags_url(super::install::AGENT_BROWSER_PACKAGE))
        .map(|body| DistTags::parse(&body));
    components.push(compare(
        "agent-browser",
        installed("agent-browser").as_deref(),
        agent_browser_tags
            .as_ref()
            .and_then(|t| t.latest.clone())
            .as_deref(),
        Some("npm latest".to_string()),
    ));

    // kanboard: comes from the extensions, so its answer is the extensions'.
    components.push(component_from(&components, "kanboard", "unipi"));

    // herdr: GitHub's latest release.
    let herdr = ctx
        .fetch(&github_release_url("herdr-dev/herdr"))
        .as_deref()
        .and_then(version_from_github);
    components.push(compare(
        "herdr",
        installed("herdr").as_deref(),
        herdr.as_deref(),
        Some("GitHub".to_string()),
    ));

    // host: the version the feed offers.
    let host_offered = ctx
        .fetch("https://unipi.nrn.one/latest.json")
        .as_deref()
        .and_then(host_version_from_feed);
    components.push(compare(
        "host",
        installed("host").as_deref(),
        host_offered.as_deref(),
        Some("latest.json".to_string()),
    ));

    let any_available = components
        .iter()
        .any(|c| matches!(c.update, Update::Available { .. } | Update::Missing { .. }));
    let offline = components
        .iter()
        .all(|c| matches!(c.update, Update::Unknown));
    UpdateReport {
        components,
        any_available,
        offline,
    }
}

/// The version an installed UniPi should be compared against, for the channel
/// it follows. A channel whose version we know gives that version; a
/// concrete pinned version (`npm:…@3.0.0`, which `channel_for` returns as the
/// "channel") is its own target. When the channel's version is unknowable —
/// offline — this is `None` and never the channel's name: comparing
/// `3.0.0-alpha.41` against `"alpha"` would read as up to date.
fn offered_for_channel(channel: &str, spec: Option<&str>, tags: &DistTags) -> Option<String> {
    if let Some(version) = tags.version_of(channel) {
        return Some(version.to_string());
    }
    let pinned =
        spec.is_some_and(|s| s.starts_with("npm:")) && !matches!(channel, "latest" | "alpha");
    pinned.then(|| channel.to_string())
}

/// A component that travels with another (kanboard with the extensions).
fn component_from(components: &[ComponentUpdate], name: &str, source: &str) -> ComponentUpdate {
    let other = components.iter().find(|c| c.name == source);
    ComponentUpdate {
        name: name.to_string(),
        update: other.map(|c| c.update.clone()).unwrap_or(Update::Unknown),
        detail: Some(format!("comes with {source}")),
    }
}

/// The `host` block's version out of `latest.json`.
pub fn host_version_from_feed(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value
        .get("host")?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

/// The real thing: one report for this PC.
pub async fn check_here(report: &super::SetupReport) -> UpdateReport {
    let mut installed = std::collections::BTreeMap::new();
    for name in [
        "node",
        "npm",
        "pi",
        "unipi",
        "agent-browser",
        "kanboard",
        "herdr",
        "host",
        "tmux",
        "systemd-user",
    ] {
        installed.insert(
            name.to_string(),
            report.get(name).and_then(|c| c.version.clone()),
        );
    }
    let ctx = CheckCtx {
        installed,
        unipi_spec: super::read_unipi_spec(
            &super::setup_json_path_of_home(),
            std::path::Path::new("."),
        ),
        get: http_fetch(),
    };
    check_with(&ctx)
}

/// `setup_check_updates`: what is newer than what this PC has, for Settings →
/// This computer → Components (S4). Detection runs again — the row says
/// "check now", so it must not answer from a stale report.
#[tauri::command]
pub async fn setup_check_updates() -> Result<UpdateReport, AppErrorDto> {
    let home = crate::host_supervisor::home_dir();
    let report = super::detect_here(&home).await;
    Ok(check_here(&report).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(map: &[(&str, Option<&str>)]) -> std::collections::BTreeMap<String, Option<String>> {
        map.iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
            .collect()
    }

    /// A fake registry: a URL → body map; anything else returns nothing.
    fn fake(map: &'static [(&'static str, &'static str)]) -> Fetch {
        Arc::new(move |url| {
            map.iter()
                .find(|(u, _)| *u == url)
                .map(|(_, b)| b.to_string())
        })
    }

    const NPM_UNIPI: &str = "https://registry.npmjs.org/-/package/@pi-unipi/unipi/dist-tags";
    const NPM_PI: &str =
        "https://registry.npmjs.org/-/package/@earendil-works/pi-coding-agent/dist-tags";
    const NPM_AGENT_BROWSER: &str = "https://registry.npmjs.org/-/package/agent-browser/dist-tags";
    const GH_HERDR: &str = "https://api.github.com/repos/herdr-dev/herdr/releases/latest";
    const LATEST_JSON: &str = "https://unipi.nrn.one/latest.json";

    #[test]
    fn compares_versions_numerically_and_never_downgrades() {
        assert!(is_newer("0.4.10", "0.4.9"));
        assert!(!is_newer("0.4.9", "0.4.9"));
        assert!(!is_newer("0.4.8", "0.4.9"));
        assert!(is_newer("3.0.0", "2.20.5"), "a major jump is newer");
        assert!(
            is_newer("3.0.0-alpha.42", "3.0.0-alpha.41"),
            "the next alpha is newer"
        );
        assert!(
            is_newer("3.0.0-alpha.9", "3.0.0-alpha.8"),
            "numerically, not as strings"
        );
        assert!(is_newer("3.0.1", "3.0.1-alpha.1"));
        // A prerelease of the same X.Y.Z is not newer than the release.
        assert!(!is_newer("3.0.0-alpha.42", "3.0.0"));
    }

    #[test]
    fn compare_maps_installed_and_offered_to_a_state() {
        assert_eq!(
            compare("pi", Some("1.0.0"), Some("1.2.0"), None).update,
            Update::Available {
                from: "1.0.0".into(),
                to: "1.2.0".into()
            }
        );
        assert_eq!(
            compare("pi", Some("1.2.0"), Some("1.2.0"), None).update,
            Update::Current {
                installed: "1.2.0".into(),
                offered: Some("1.2.0".into())
            }
        );
        // Decision 4: a newer user version is left alone, not downgraded — and
        // it says what it was compared against, so the UI can call it newer.
        assert_eq!(
            compare("pi", Some("9.9.9"), Some("1.2.0"), None).update,
            Update::Current {
                installed: "9.9.9".into(),
                offered: Some("1.2.0".into())
            }
        );
        assert_eq!(
            compare("pi", None, Some("1.2.0"), None).update,
            Update::Missing { to: "1.2.0".into() }
        );
        assert_eq!(
            compare("pi", Some("1.0.0"), None, None).update,
            Update::Unknown
        );
        assert_eq!(compare("pi", None, None, None).update, Update::Unknown);
    }

    #[test]
    fn a_report_with_a_newer_pi_and_unipi() {
        let ctx = CheckCtx {
            installed: install(&[
                ("node", Some("22.19.0")),
                ("pi", Some("1.0.0")),
                ("unipi", Some("3.0.0-alpha.41")),
                ("herdr", Some("0.9.0")),
                ("host", Some("0.4.9")),
            ]),
            unipi_spec: Some("npm:@pi-unipi/unipi@alpha".into()),
            get: fake(&[
                (NPM_PI, r#"{"latest":"1.2.0"}"#),
                (NPM_UNIPI, r#"{"latest":"2.20.5","alpha":"3.0.0-alpha.42"}"#),
                (GH_HERDR, r#"{"tag_name":"v0.9.1"}"#),
                (LATEST_JSON, r#"{"host":{"version":"0.5.0"}}"#),
            ]),
        };
        let report = check_with(&ctx);
        assert_eq!(
            report.get("pi").unwrap().update,
            Update::Available {
                from: "1.0.0".into(),
                to: "1.2.0".into()
            }
        );
        // The alpha install is compared against alpha, not latest.
        assert_eq!(
            report.get("unipi").unwrap().update,
            Update::Available {
                from: "3.0.0-alpha.41".into(),
                to: "3.0.0-alpha.42".into()
            }
        );
        assert_eq!(
            report.get("unipi").unwrap().detail.as_deref(),
            Some("channel: alpha")
        );
        assert_eq!(
            report.get("herdr").unwrap().update,
            Update::Available {
                from: "0.9.0".into(),
                to: "0.9.1".into()
            }
        );
        assert_eq!(
            report.get("host").unwrap().update,
            Update::Available {
                from: "0.4.9".into(),
                to: "0.5.0".into()
            }
        );
        // kanboard travels with the extensions.
        assert_eq!(
            report.get("kanboard").unwrap().detail.as_deref(),
            Some("comes with unipi")
        );
        assert!(report.any_available);
        assert!(!report.offline);
    }

    #[test]
    fn an_up_to_date_pc_has_nothing_to_offer() {
        let ctx = CheckCtx {
            installed: install(&[
                ("pi", Some("1.2.0")),
                ("unipi", Some("3.0.0-alpha.42")),
                ("herdr", Some("0.9.1")),
                ("host", Some("0.5.0")),
            ]),
            unipi_spec: Some("npm:@pi-unipi/unipi@alpha".into()),
            get: fake(&[
                (NPM_PI, r#"{"latest":"1.2.0"}"#),
                (NPM_UNIPI, r#"{"latest":"2.20.5","alpha":"3.0.0-alpha.42"}"#),
                (GH_HERDR, r#"{"tag_name":"v0.9.1"}"#),
                (LATEST_JSON, r#"{"host":{"version":"0.5.0"}}"#),
            ]),
        };
        let report = check_with(&ctx);
        assert!(!report.any_available, "{report:?}");
        assert!(!report.offline);
        for name in ["pi", "unipi", "herdr", "host"] {
            assert!(
                matches!(report.get(name).unwrap().update, Update::Current { .. }),
                "{name}: {:?}",
                report.get(name).unwrap().update
            );
        }
    }

    #[test]
    fn offline_is_unknown_never_up_to_date() {
        let ctx = CheckCtx {
            installed: install(&[("pi", Some("1.0.0")), ("unipi", Some("3.0.0-alpha.41"))]),
            unipi_spec: None,
            get: Arc::new(|_| None),
        };
        let report = check_with(&ctx);
        assert!(report.offline, "{report:?}");
        assert!(!report.any_available);
        for c in &report.components {
            assert_eq!(
                c.update,
                Update::Unknown,
                "{} must not read as up to date",
                c.name
            );
        }
    }

    #[test]
    fn agent_browser_follows_npm_latest_like_pi() {
        let ctx = CheckCtx {
            installed: install(&[("agent-browser", Some("1.0.0"))]),
            unipi_spec: None,
            get: fake(&[(NPM_AGENT_BROWSER, r#"{"latest":"1.1.0"}"#)]),
        };
        let report = check_with(&ctx);
        assert_eq!(
            report.get("agent-browser").unwrap().update,
            Update::Available {
                from: "1.0.0".into(),
                to: "1.1.0".into()
            }
        );
        assert_eq!(
            report.get("agent-browser").unwrap().detail.as_deref(),
            Some("npm latest")
        );
    }

    #[test]
    fn host_version_comes_from_the_feed_host_block() {
        assert_eq!(
            host_version_from_feed(r#"{"host":{"version":"0.5.0","files":{}}}"#).as_deref(),
            Some("0.5.0")
        );
        assert_eq!(host_version_from_feed(r#"{"app":{}}"#), None);
        assert_eq!(host_version_from_feed("garbage"), None);
        assert_eq!(
            version_from_github(r#"{"tag_name":"v1.4.2"}"#).as_deref(),
            Some("1.4.2")
        );
        assert_eq!(version_from_github(r#"{"tag_name":""}"#), None);
    }

    #[test]
    fn a_fresh_pc_shows_installs_not_updates() {
        let ctx = CheckCtx {
            installed: install(&[]),
            unipi_spec: None,
            get: fake(&[
                (NPM_PI, r#"{"latest":"1.2.0"}"#),
                (NPM_UNIPI, r#"{"latest":"2.20.5","alpha":"3.0.0-alpha.41"}"#),
            ]),
        };
        let report = check_with(&ctx);
        // Nothing installed and no spec: the channel floor applies, so alpha.
        assert_eq!(
            report.get("unipi").unwrap().update,
            Update::Missing {
                to: "3.0.0-alpha.41".into()
            }
        );
        assert_eq!(
            report.get("pi").unwrap().update,
            Update::Missing { to: "1.2.0".into() }
        );
        assert!(report.any_available);
    }
}
