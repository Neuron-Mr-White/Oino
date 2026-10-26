//! `unipi.updater` — Oino release checks and the docs browsers (unipi `packages/updater`,
//! unipi-port §3.23).
//!
//! | unipi | here |
//! |---|---|
//! | npm dist-tag check at `session_start`, `checkIntervalMs`, skipped versions, channels | GitHub releases of `Neuron-Mr-White/Oino` ([`channel`], [`check`]); at `session_start` and on a repeating `oino_timer`; cache `/cache/last-check.json` ([`cache`]) |
//! | update overlay (changelog between versions, `Y` install, `n` skip, auto countdown) | overlay panel with the changelog, "How to update" (`oino update` / desktop updater), release page, skip ([`view::update_panel`]); no in-extension install |
//! | remote changelog (tag → channel branch → `main`, cached per version) | same against `raw.githubusercontent.com/Neuron-Mr-White/Oino` ([`changelog`]) |
//! | `UPDATE_CHECK` / `UPDATE_AVAILABLE` / `UPDATE_APPLIED` | same topics; `UPDATE_APPLIED` when the installed version grew since the last run |
//! | `/unipi:readme [package]` | READMEs of bundled/installed packages under `/agent/bundled`, `/agent/extensions` ([`readme`]) as markdown panels |
//! | `/unipi:changelog` | version list → detail from the bundled `CHANGELOG.md` (else remote) |
//! | info-screen "Updater" group | `info-screen.groups` service |
//! | settings `updater` | same keys; `autoUpdate` `disabled`/`notify` (`auto` = `notify`) |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

mod cache;
mod changelog;
mod channel;
mod check;
mod readme;
mod view;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use oino_extension::command::item;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use unipi_kit::settings::Store;
use unipi_kit::topics::{UpdateApplied, UpdateAvailable, UpdateCheck};

use crate::changelog::Entry;
use crate::channel::{CheckResult, Release};
use crate::readme::Readme;
use crate::view::NAMESPACE;

const TIMER_ID: &str = "updater.check";
const FETCH_TIMEOUT_MS: u32 = 10_000;
const CHANGELOG_TIMEOUT_MS: u32 = 5_000;

/// What an open panel shows.
#[derive(Clone, Debug)]
enum View {
    Update(CheckResult, String),
    Instructions(CheckResult, String),
    Readmes(Vec<Readme>),
    Readme(Vec<Readme>),
    Changelog(Vec<Entry>),
    ChangelogEntry(Vec<Entry>),
}

#[derive(Default)]
struct SessionState {
    panels: BTreeMap<String, View>,
}

#[derive(Default)]
struct Last {
    result: Option<CheckResult>,
    checked_at: i64,
    release: Option<Release>,
}

/// The extension.
#[derive(Default)]
pub struct Updater {
    settings: Store,
    last: RefCell<Last>,
    offered: RefCell<BTreeSet<String>>,
    sessions: SessionMap<SessionState>,
}

fn now_iso() -> String {
    unipi_kit::fmt::iso_from_ms(timers::now_ms())
}

async fn github_json(path: &str) -> std::result::Result<Value, String> {
    let url = format!("{}{path}", channel::API_BASE);
    let resp = http::get(&url)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "oino-updater")
        .timeout_ms(FETCH_TIMEOUT_MS)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Value>().await.map_err(|e| e.to_string())
}

/// unipi `fetchRemoteChangelog`: cached per version; the tag is immutable, branch fallbacks
/// are never cached.
async fn remote_changelog(version: &str) -> Option<String> {
    let cache = unipi_kit::paths::cache(&format!("changelog-{version}.md"));
    if let Some(text) = unipi_kit::paths::try_read(&cache) {
        return Some(text);
    }
    let get = |r: String| async move {
        http::get(&format!(
            "{}/{r}/CHANGELOG.md",
            changelog::CHANGELOG_RAW_BASE
        ))
        .header("user-agent", "oino-updater")
        .timeout_ms(CHANGELOG_TIMEOUT_MS)
        .send()
        .await
        .ok()
    };
    let mut resp = get(format!("v{version}")).await?;
    let cacheable = (200..300).contains(&resp.status());
    for r in changelog::fallback_refs(version) {
        if resp.status() != 404 {
            break;
        }
        resp = get(r).await?;
    }
    if !(200..300).contains(&resp.status()) {
        return None;
    }
    let text = resp.text().await;
    if cacheable {
        let _ = unipi_kit::paths::ensure_parent(&cache);
        let _ = oino_extension::fs::atomic_write(&cache.to_string_lossy(), text.as_bytes());
    }
    Some(text)
}

fn bundled_changelog() -> Vec<Entry> {
    readme::changelog_path(&readme::roots())
        .and_then(|p| unipi_kit::paths::try_read(&p))
        .map(|t| changelog::parse(&t))
        .unwrap_or_default()
}

/// unipi `loadUpdateChangelog` + the release notes as a last resort.
async fn update_changelog(current: &str, latest: &str, release: Option<&Release>) -> String {
    let remote = remote_changelog(latest)
        .await
        .map(|t| changelog::parse(&t))
        .unwrap_or_default();
    let mut entries = changelog::up_to(changelog::newer_versions(&remote, current), latest);
    if entries.is_empty() {
        entries = changelog::up_to(
            changelog::newer_versions(&bundled_changelog(), current),
            latest,
        );
    }
    if !entries.is_empty() {
        return changelog::render(&entries);
    }
    release
        .and_then(|r| r.body.clone())
        .filter(|b| !b.trim().is_empty())
        .map(|b| format!("## {latest}\n\n{b}"))
        .unwrap_or_default()
}

impl Updater {
    fn mode(&self) -> String {
        self.settings
            .get(NAMESPACE)
            .str("autoUpdate", "notify")
            .to_owned()
    }

    fn interval(&self) -> u64 {
        view::interval_ms(self.settings.get(NAMESPACE).get("checkIntervalMs"))
    }

    /// unipi `checkForUpdates`.
    async fn check(&self, force: bool) -> CheckResult {
        let current = host::info().oino_version;
        let channel = channel::resolve_channel(&current);
        let path = cache::path();
        let prev = cache::read(&path);
        if let Some(r) = check::cached(
            prev.as_ref(),
            &current,
            &channel,
            self.interval(),
            timers::now_ms(),
            force,
        ) {
            return r;
        }
        let fetched = if channel == "latest" {
            github_json("/releases/latest").await.and_then(|v| {
                serde_json::from_value::<Release>(v)
                    .map(|r| vec![r])
                    .map_err(|e| e.to_string())
            })
        } else {
            github_json("/releases?per_page=50")
                .await
                .and_then(|v| serde_json::from_value::<Vec<Release>>(v).map_err(|e| e.to_string()))
        };
        let outcome = fetched.and_then(|releases| {
            check::from_releases(&releases, prev.as_ref(), &current, &channel, &now_iso())
        });
        match outcome {
            Ok((result, new_cache, picked)) => {
                cache::write(&path, &new_cache);
                self.last.borrow_mut().release = picked;
                result
            }
            Err(e) => check::on_error(prev.as_ref(), &current, &channel, &e),
        }
    }

    /// Check, publish the bus events, and offer the update once per version.
    async fn run_check(&self, cx: &Ctx, force: bool) -> CheckResult {
        let result = self.check(force).await;
        {
            let mut last = self.last.borrow_mut();
            last.result = Some(result.clone());
            last.checked_at = timers::now_ms();
        }
        unipi_kit::bus::emit(&UpdateCheck {
            current_version: result.current_version.clone(),
            latest_version: result.latest_version.clone(),
            update_available: result.update_available,
            error: result.error.clone(),
        });
        if !result.update_available || result.error.is_some() {
            return result;
        }
        if cache::is_skipped(cache::read(&cache::path()).as_ref(), &result.latest_version) {
            return result;
        }
        unipi_kit::bus::emit(&UpdateAvailable {
            current_version: result.current_version.clone(),
            latest_version: result.latest_version.clone(),
        });
        if cx.has_ui()
            && self
                .offered
                .borrow_mut()
                .insert(result.latest_version.clone())
        {
            let _ = self.open_update(cx, &result).await;
        }
        result
    }

    async fn open_update(&self, cx: &Ctx, result: &CheckResult) -> Result<()> {
        let release = self.last.borrow().release.clone();
        let md = update_changelog(
            &result.current_version,
            &result.latest_version,
            release.as_ref(),
        )
        .await;
        let id = ui::panel_open(&view::update_panel(result, &md))?;
        self.sessions.with(cx.session_id(), |s| {
            s.panels.insert(id, View::Update(result.clone(), md));
        });
        Ok(())
    }

    /// `UPDATE_APPLIED`: the installed version grew since the previous run.
    fn note_applied(cx: &Ctx) {
        let current = host::info().oino_version;
        let path = unipi_kit::paths::global("state.json");
        let prev: Option<Value> = unipi_kit::paths::read_json(&path);
        let last = prev
            .as_ref()
            .and_then(|v| v.get("lastRunVersion"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if last.as_deref() != Some(current.as_str()) {
            if let Some(previous) = last.filter(|p| unipi_kit::fmt::is_newer_version(&current, p)) {
                unipi_kit::bus::emit(&UpdateApplied {
                    previous_version: previous,
                    new_version: current.clone(),
                });
                if cx.has_ui() {
                    let _ = ui::notify(&format!("Updated to {current}."), "info");
                }
            }
            let _ = unipi_kit::paths::write_json(&path, &json!({"lastRunVersion": current}));
        }
    }

    fn open_view(&self, cx: &Ctx, name: &str, title: &str, spec: &Value, v: View) -> Result<()> {
        if !cx.has_ui() {
            return Ok(());
        }
        let id = ui::panel_open(&view::browser_panel(name, title, spec))?;
        self.sessions.with(cx.session_id(), |s| {
            s.panels.insert(id, v);
        });
        Ok(())
    }

    fn readme_command(&self, args: &str, cx: &CommandCtx) -> Result<()> {
        let readmes = readme::discover(&readme::roots());
        let wanted = args.trim();
        if !wanted.is_empty() {
            match readme::resolve(&readmes, wanted)
                .and_then(|r| unipi_kit::paths::try_read(&r.path).map(|t| (r.clone(), t)))
            {
                Some((r, text)) => {
                    return self.open_view(
                        &cx.cx,
                        "readme",
                        &format!("README — {} {}", r.name, r.version),
                        &view::doc_spec(&text),
                        View::Readme(readmes),
                    );
                }
                None => {
                    let _ = ui::notify(&format!("No README for \"{wanted}\"."), "warning");
                }
            }
        }
        let spec = view::readme_list_spec(&readmes);
        self.open_view(&cx.cx, "readme", "READMEs", &spec, View::Readmes(readmes))
    }

    async fn changelog_command(&self, cx: &CommandCtx) -> Result<()> {
        let mut entries = bundled_changelog();
        if entries.is_empty() {
            let current = host::info().oino_version;
            entries = remote_changelog(&current)
                .await
                .map(|t| changelog::parse(&t))
                .unwrap_or_default();
        }
        let spec = view::changelog_list_spec(&entries);
        self.open_view(
            &cx.cx,
            "changelog",
            "Changelog",
            &spec,
            View::Changelog(entries),
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one match over every panel view and action"
    )]
    fn on_ui_action(&self, ev: &Event, cx: &Ctx) -> Result<()> {
        let Some(panel) = ev
            .get("panelId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            return Ok(());
        };
        let action = ev.get("actionId").and_then(Value::as_str).unwrap_or("");
        let value = ev
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(current) = self
            .sessions
            .with(cx.session_id(), |s| s.panels.get(&panel).cloned())
        else {
            return Ok(());
        };
        let set = |v: Option<View>| {
            self.sessions.with(cx.session_id(), |s| match v {
                Some(v) => {
                    s.panels.insert(panel.clone(), v);
                }
                None => {
                    s.panels.remove(&panel);
                }
            });
        };
        if matches!(action, "$close" | "close") {
            if action == "close" {
                ui::panel_close(&panel);
            }
            set(None);
            return Ok(());
        }
        match (current, action) {
            (View::Update(r, md), "how") => {
                let text = channel::update_instructions(&r.latest_version);
                ui::panel_update(
                    &panel,
                    &json!({"spec": view::instructions_spec(&text), "title": "How to update"}),
                )?;
                set(Some(View::Instructions(r, md)));
            }
            (View::Update(r, _) | View::Instructions(r, _), "open") => {
                let url = r.release_url.clone().unwrap_or_else(|| {
                    format!(
                        "https://github.com/{}/releases/tag/v{}",
                        channel::REPO,
                        r.latest_version
                    )
                });
                if ui::open_url(&url).is_err() {
                    let _ = ui::notify(&url, "info");
                }
            }
            (View::Update(r, _) | View::Instructions(r, _), "skip") => {
                cache::skip_version(&cache::path(), &r.latest_version);
                ui::panel_close(&panel);
                set(None);
                let _ = ui::notify(
                    &format!(
                        "Skipping {} — you will be told about the next release.",
                        r.latest_version
                    ),
                    "info",
                );
            }
            (View::Instructions(r, md), "back") => {
                ui::panel_update(
                    &panel,
                    &json!({"spec": view::update_panel(&r, &md)["spec"], "title": "Update available"}),
                )?;
                set(Some(View::Update(r, md)));
            }
            (View::Readmes(list), "open") => {
                if let Some(r) = list.iter().find(|r| r.name == value)
                    && let Some(text) = unipi_kit::paths::try_read(&r.path)
                {
                    ui::panel_update(
                        &panel,
                        &json!({"spec": view::doc_spec(&text), "title": format!("README — {} {}", r.name, r.version)}),
                    )?;
                    set(Some(View::Readme(list)));
                }
            }
            (View::Readme(list), "back") => {
                ui::panel_update(
                    &panel,
                    &json!({"spec": view::readme_list_spec(&list), "title": "READMEs"}),
                )?;
                set(Some(View::Readmes(list)));
            }
            (View::Changelog(entries), "open") => {
                if let Some(e) = value.parse::<usize>().ok().and_then(|i| entries.get(i)) {
                    let md = changelog::render(std::slice::from_ref(e));
                    ui::panel_update(
                        &panel,
                        &json!({"spec": view::doc_spec(&md), "title": format!("Changelog — {}", e.version)}),
                    )?;
                    set(Some(View::ChangelogEntry(entries)));
                }
            }
            (View::ChangelogEntry(entries), "back") => {
                ui::panel_update(
                    &panel,
                    &json!({"spec": view::changelog_list_spec(&entries), "title": "Changelog"}),
                )?;
                set(Some(View::Changelog(entries)));
            }
            _ => {}
        }
        Ok(())
    }

    fn arm_timer(&self) {
        if view::checks_enabled(&self.mode()) {
            timers::set(TIMER_ID, Duration::from_millis(self.interval()), true);
        } else {
            timers::cancel(TIMER_ID);
        }
    }
}

impl Extension for Updater {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = view::namespace();
        cx.settings_namespace(&ns.to_json());
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            if cx.is_child() {
                return Ok(());
            }
            Self::note_applied(&cx);
            ext.arm_timer();
            if view::checks_enabled(&ext.mode()) {
                // Update check failures are silent and non-critical (unipi).
                let _ = ext.run_check(&cx, false).await;
            }
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            if ev.get("timerId").and_then(Value::as_str) == Some(TIMER_ID)
                && !cx.is_child()
                && view::checks_enabled(&ext.mode())
            {
                let _ = ext.run_check(&cx, false).await;
            }
            Ok(())
        });
        cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
            ext.on_ui_action(&ev, &cx)
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            if ext.settings.on_changed(&ev.payload).is_some() {
                ext.arm_timer();
            }
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.sessions.remove(cx.session_id());
            Ok(())
        });

        cx.command(
            Command::<Self>::new(unipi_kit::suite::command("readme"))
                .description("Browse package README files")
                .argument_hint("[package]")
                .run(|ext, args, cx| async move { ext.readme_command(&args, &cx) })
                .complete(|_ext, prefix, _cx| async move {
                    let p = prefix.trim().to_owned();
                    readme::discover(&readme::roots())
                        .into_iter()
                        .filter(|r| r.name.starts_with(&p))
                        .map(|r| item(r.name))
                        .collect()
                }),
        );
        cx.command(
            Command::<Self>::new(unipi_kit::suite::command("changelog"))
                .description("Browse changelog (Keep a Changelog format)")
                .run(|ext, _args, cx| async move { ext.changelog_command(&cx).await }),
        );
        cx.command(
            Command::<Self>::new("unipi:update-check")
                .description("Check for an Oino update now")
                .hidden()
                .run_value(|ext, _args, cx| async move {
                    let r = ext.run_check(&cx.cx, true).await;
                    if cx.has_ui() {
                        let msg = match (&r.error, r.update_available) {
                            (Some(e), _) => format!("Update check failed: {e}"),
                            (None, true) => format!(
                                "Oino {} is available (installed {}).",
                                r.latest_version, r.current_version
                            ),
                            (None, false) => format!(
                                "Oino {} is up to date (checks every {}).",
                                r.current_version,
                                view::interval_label(ext.interval())
                            ),
                        };
                        let _ =
                            ui::notify(&msg, if r.error.is_some() { "warning" } else { "info" });
                    }
                    Ok(serde_json::to_value(&r).ok())
                }),
        );
        cx.service(
            unipi_kit::services::fanout::INFO_SCREEN_GROUPS,
            |ext, _payload, _caller, _cx| async move {
                let last = ext.last.borrow();
                Ok(view::info_group(
                    &host::info().oino_version,
                    last.result.as_ref(),
                    last.checked_at,
                    &ext.mode(),
                ))
            },
        );
        Ok(())
    }
}

/// Paths the README browser searches (exposed for the harness docs).
#[must_use]
pub fn readme_roots() -> Vec<PathBuf> {
    readme::roots()
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Updater);
