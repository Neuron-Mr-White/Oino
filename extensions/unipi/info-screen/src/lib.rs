//! `unipi.info-screen` — unipi `packages/info-screen` (unipi-port §3.7).
//!
//! | unipi | Here |
//! |---|---|
//! | `/info [page]` overlay (`ctx.ui.custom`) | `overlay` panel with a `tabs` node; keys `r`/`R`/`q`/`1–9` ([`InfoScreen::open_dashboard`]) |
//! | page registry on `globalThis.__unipi_info_registry` | built-in pages + `info-screen.groups` service fan-out ([`registry`]) |
//! | cache-first snapshots (`~/.unipi/cache/info-screen*.json`) | [`registry::Cache`] in `/cache` |
//! | usage parser over `~/.pi/agent/sessions/**` | [`usage`] over `sessions-list` / `session-read`, per-file cache `/cache/usage-stats.json` |
//! | Session / Usage / Tools / Skills / Modules + module pages | [`pages`] (OUS) |
//! | context bucket | [`stats::context_breakdown`] |
//! | Unicrab splash (`bootMode`, `bootTimeoutMs`) | `setHeader` ([`splash`]); auto-close timer / closes on input in `on` mode |
//! | `MODULE_READY` batching, `UPDATE_AVAILABLE`/`UPDATE_APPLIED` | bus subscriptions |
//! | settings `info-screen` (dynamic pages page, page order) | [`config`], re-registered when groups change |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod config;
pub mod kit;
pub mod pages;
pub mod registry;
pub mod splash;
pub mod stats;
pub mod time;
pub mod usage;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::Duration;

use oino_extension::command::CompletionItem;
use oino_extension::prelude::*;
use oino_extension::session::SessionsScope;
use oino_extension::sync::SessionMap;
use serde_json::Map;
use unipi_kit::bus::Delivery;
use unipi_kit::settings::Store;
use unipi_kit::topics::{self, ModuleReady};

use crate::config::{BootMode, InfoSettings};
use crate::kit::Scope;
use crate::pages::{Module, Named};
use crate::registry::{Cache, Group};
use crate::usage::{CACHE_VERSION, CacheFile, CachedFile};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.info-screen";
/// Command.
pub const COMMAND: &str = "info";
/// Dashboard panel name.
pub const PANEL: &str = "unipi.info-screen/dashboard";
/// unipi child-usage entry type.
pub const CHILD_USAGE_ENTRY: &str = "unipi-child-usage";
const SPLASH_TIMER: &str = "splash";

/// Built-in pages (unipi `registerCoreGroups`).
#[must_use]
pub fn builtin_groups() -> Vec<Group> {
    vec![
        Group::builtin(
            "session",
            "This session",
            10,
            &[
                ("cost", "Cost"),
                ("tokens", "Tokens"),
                ("replies", "Replies"),
            ],
        ),
        Group::builtin(
            "usage",
            "Usage history",
            20,
            &[
                ("tokensToday", "Tokens today"),
                ("costToday", "Cost today"),
                ("costAllTime", "Cost all time"),
                ("sessions", "Sessions"),
            ],
        ),
        Group::builtin("tools", "Tools", 30, &[("total", "Tools")]),
        Group::builtin("skills", "Skills", 40, &[("total", "Skills")]),
        Group::builtin("extensions", "Modules", 50, &[("count", "Modules")]),
    ]
}

#[derive(Default)]
struct Dash {
    panel: Option<String>,
    active: String,
    loading: Vec<String>,
}

/// The extension.
#[derive(Default)]
pub struct InfoScreen {
    settings: Store,
    cache: RefCell<Cache>,
    usage: RefCell<Option<CacheFile>>,
    groups: RefCell<Vec<Group>>,
    dash: SessionMap<Dash>,
    update: RefCell<Option<String>>,
    started: Cell<i64>,
    boot_ms: Cell<i64>,
    modules: RefCell<Vec<String>>,
    splash_open: SessionMap<bool>,
}

fn git_branch() -> Option<String> {
    let head = std::fs::read_to_string("/project/.git/HEAD").ok()?;
    let h = head.trim();
    Some(
        h.strip_prefix("ref: refs/heads/")
            .map_or_else(|| h.chars().take(7).collect(), ToOwned::to_owned),
    )
}

fn home_short(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && p.starts_with(&home) {
        format!("~{}", &p[home.len()..])
    } else {
        p.to_owned()
    }
}

impl InfoScreen {
    fn info(&self) -> InfoSettings {
        InfoSettings::from_json(&self.settings.get(config::NAMESPACE).0)
    }

    /// Built-ins + provider pages (provider data seeded into the cache), ordered, visible only.
    async fn collect_groups(&self) -> Vec<Group> {
        let replies = unipi_kit::fanout::call_all(
            unipi_kit::services::fanout::INFO_SCREEN_GROUPS,
            &json!({}),
            unipi_kit::fanout::DEFAULT_TIMEOUT,
            Some(EXTENSION_ID),
        )
        .await;
        let now = timers::now_ms();
        let mut all = builtin_groups();
        for r in replies {
            for (g, data) in registry::decode_reply(&r.extension, &r.value) {
                if !data.is_empty() {
                    self.cache.borrow_mut().put(&g.id, data, now);
                }
                all.retain(|x| x.id != g.id);
                all.push(g);
            }
        }
        let s = self.info();
        let ordered = registry::ordered(all, &s.group_order);
        let changed = *self.groups.borrow() != ordered;
        if changed {
            let infos: Vec<_> = ordered.iter().map(Group::info).collect();
            let _ = oino_extension::wit::registry::register_settings(
                &config::namespace(&infos).to_json().to_string(),
            );
            self.groups.borrow_mut().clone_from(&ordered);
        }
        ordered
            .into_iter()
            .filter(|g| s.group_shown(&g.id))
            .collect()
    }

    // ── data providers ───────────────────────────────────────────────────────

    fn session_page(cx: &Ctx) -> Map<String, Value> {
        let now = timers::now_ms();
        let branch = session::branch(None).unwrap_or_default();
        let header_ts = session::header().ok().and_then(|h| {
            h.get("timestamp")
                .and_then(Value::as_str)
                .and_then(time::parse_iso_ms)
        });
        let mut r = stats::collect(&branch, header_ts, now);
        r.cwd = cx.cwd().to_string_lossy().into_owned();
        r.name = session::session_name();
        let model = control::model().unwrap_or(Value::Null);
        model
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| model.get("id").and_then(Value::as_str))
            .unwrap_or("no model")
            .clone_into(&mut r.model);
        model
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .clone_into(&mut r.provider);
        r.thinking = control::thinking_level();
        let usage = control::context_usage().unwrap_or(Value::Null);
        r.ctx_tokens = usage.get("tokens").and_then(Value::as_f64);
        r.ctx_window = usage
            .get("contextWindow")
            .and_then(Value::as_f64)
            .or_else(|| model.get("contextWindow").and_then(Value::as_f64))
            .unwrap_or(0.0);
        r.ctx_pct = usage.get("percent").and_then(Value::as_f64);
        r.branch = git_branch().or_else(|| host::workspace().git_branch);
        let active = control::active_tools();
        #[allow(clippy::cast_precision_loss, reason = "char counts")]
        let schema_chars: f64 = control::all_tools()
            .unwrap_or_default()
            .iter()
            .filter(|t| {
                t.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| active.iter().any(|a| a == n))
            })
            .map(|t| {
                let n = |k: &str| {
                    t.get(k).map_or(0, |v| {
                        v.as_str().map_or_else(|| v.to_string().len(), str::len)
                    })
                };
                (n("name") + n("description") + n("parameters")) as f64
            })
            .sum();
        let messages = session::projection()
            .ok()
            .and_then(|p| p.get("messages").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        r.context = Some(stats::context_breakdown(
            &control::system_prompt(),
            schema_chars,
            &messages,
            r.ctx_tokens,
            r.ctx_window,
        ));
        pages::session_data(&r)
    }

    fn usage_cache(&self) -> CacheFile {
        let mut slot = self.usage.borrow_mut();
        slot.get_or_insert_with(|| {
            std::fs::read(unipi_kit::paths::cache("usage-stats.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
                .filter(|c| c.version == CACHE_VERSION)
                .unwrap_or(CacheFile {
                    version: CACHE_VERSION,
                    ..CacheFile::default()
                })
        })
        .clone()
    }

    async fn usage_page(&self) -> Map<String, Value> {
        let mut cache = self.usage_cache();
        let list = session::list_sessions(SessionsScope::All)
            .await
            .unwrap_or_default();
        let mut next = std::collections::BTreeMap::new();
        let mut dirty = false;
        for info in &list {
            let Some((path, mtime, size)) = usage::stamp(info) else {
                continue;
            };
            if let Some(hit) = cache
                .files
                .get(&path)
                .filter(|h| h.mtime_ms == mtime && h.size == size)
            {
                next.insert(path, hit.clone());
                continue;
            }
            dirty = true;
            let entries = session::read_session(&path)
                .await
                .ok()
                .and_then(|v| v.get("entries").and_then(Value::as_array).cloned())
                .unwrap_or_default();
            let (records, savings) = usage::extract(&entries, &mut cache.models);
            next.insert(
                path,
                CachedFile {
                    mtime_ms: mtime,
                    size,
                    records,
                    savings,
                },
            );
        }
        dirty |= next.len() != cache.files.len();
        cache.files = next;
        let stats = usage::aggregate(&cache, timers::now_ms());
        if dirty {
            let _ =
                unipi_kit::paths::write_json(&unipi_kit::paths::cache("usage-stats.json"), &cache);
        }
        *self.usage.borrow_mut() = Some(cache);
        pages::usage_data(&stats)
    }

    fn tools_page() -> Map<String, Value> {
        let active = control::active_tools();
        let items: Vec<Named> = control::all_tools()
            .unwrap_or_default()
            .iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_owned();
                Some(Named {
                    group: pages::source_label(t.get("sourceInfo").unwrap_or(&Value::Null)),
                    active: Some(active.contains(&name)),
                    name,
                })
            })
            .collect();
        let n = items.iter().filter(|i| i.active == Some(true)).count();
        pages::inventory_data(&items, Some(n), &[])
    }

    fn skills_page() -> Map<String, Value> {
        let items: Vec<Named> = control::commands()
            .unwrap_or_default()
            .iter()
            .filter(|c| c.get("source").and_then(Value::as_str) == Some("skill"))
            .filter_map(|c| {
                let info = c.get("sourceInfo").cloned().unwrap_or(Value::Null);
                let path = info.get("path").and_then(Value::as_str).unwrap_or_default();
                let group = if info.get("scope").and_then(Value::as_str) == Some("project") {
                    "project"
                } else if path.contains("/.agents/") {
                    "agents"
                } else if pages::source_label(&info) == "unipi" || path.contains("/bundled/unipi.")
                {
                    "unipi"
                } else if info.get("origin").and_then(Value::as_str) == Some("package") {
                    "package"
                } else {
                    "user"
                };
                Some(Named {
                    name: c
                        .get("name")?
                        .as_str()?
                        .trim_start_matches("skill:")
                        .to_owned(),
                    group: group.to_owned(),
                    active: None,
                })
            })
            .collect();
        let scopes = [
            ("project", Scope::Project),
            ("user", Scope::Global),
            ("agents", Scope::Global),
            ("unipi", Scope::Global),
            ("package", Scope::Global),
        ];
        pages::inventory_data(&items, None, &scopes)
    }

    fn modules_page(&self) -> Map<String, Value> {
        let mut by: std::collections::BTreeMap<String, Module> = std::collections::BTreeMap::new();
        let ext_of = |info: Option<&Value>| {
            info.and_then(|i| i.get("source"))
                .and_then(Value::as_str)
                .and_then(|s| s.strip_prefix("extension:"))
                .map(ToOwned::to_owned)
        };
        for t in control::all_tools().unwrap_or_default() {
            if let Some(id) = ext_of(t.get("sourceInfo")) {
                by.entry(id.clone())
                    .or_insert_with(|| Module {
                        name: id,
                        ..Module::default()
                    })
                    .tools += 1;
            }
        }
        for c in control::commands().unwrap_or_default() {
            if let Some(id) = ext_of(c.get("sourceInfo")) {
                by.entry(id.clone())
                    .or_insert_with(|| Module {
                        name: id,
                        ..Module::default()
                    })
                    .commands += 1;
            }
        }
        for (id, _) in services::list() {
            by.entry(id.clone())
                .or_insert_with(|| Module {
                    name: id,
                    ..Module::default()
                })
                .services += 1;
        }
        for m in self.modules.borrow().iter() {
            let id = format!("unipi.{}", m.trim_start_matches("@pi-unipi/"));
            by.entry(id.clone()).or_insert_with(|| Module {
                name: id,
                ..Module::default()
            });
        }
        let (mut unipi, mut others) = (Vec::new(), Vec::new());
        for (id, mut m) in by {
            if let Some(short) = id.strip_prefix("unipi.") {
                short.clone_into(&mut m.name);
                unipi.push(m);
            } else if !id.starts_with("oino.") {
                others.push(m);
            }
        }
        pages::modules_data(
            &unipi,
            &others,
            unipi_kit::suite::UNIPI_VERSION,
            self.boot_ms.get(),
        )
    }

    async fn fetch(&self, id: &str, cx: &Ctx, force: bool) -> Map<String, Value> {
        let now = timers::now_ms();
        if !force && let Some(p) = self.cache.borrow().fresh(id, now) {
            return p.data.clone();
        }
        let data = match id {
            "session" => Self::session_page(cx),
            "usage" => self.usage_page().await,
            "tools" => Self::tools_page(),
            "skills" => Self::skills_page(),
            "extensions" => self.modules_page(),
            other => {
                let owner = self
                    .groups
                    .borrow()
                    .iter()
                    .find(|g| g.id == other)
                    .and_then(|g| g.owner.clone());
                let Some(owner) = owner else {
                    return Map::new();
                };
                let reply = services::call(
                    Some(&owner),
                    unipi_kit::services::fanout::INFO_SCREEN_GROUPS,
                    &json!({"id": other}),
                    Some(Duration::from_secs(5)),
                )
                .await;
                let found = reply
                    .ok()
                    .map(|v| registry::decode_reply(&owner, &v))
                    .unwrap_or_default()
                    .into_iter()
                    .find(|(g, _)| g.id == other);
                match found {
                    Some((_, d)) => d,
                    None => {
                        return self
                            .cache
                            .borrow_mut()
                            .cached(other)
                            .map(|p| p.data)
                            .unwrap_or_default();
                    }
                }
            }
        };
        if !data.is_empty() {
            self.cache.borrow_mut().put(id, data.clone(), now);
        }
        data
    }

    fn compactor_history(&self) -> Value {
        let usage = self.cache.borrow_mut().cached("usage").map(|p| p.data);
        let raw = usage
            .as_ref()
            .and_then(|d| d.get("raw"))
            .and_then(|r| r.get("raw"))
            .cloned()
            .unwrap_or(Value::Null);
        let key = session::session_file().and_then(|f| {
            PathBuf::from(f)
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
        });
        let pick = |v: Option<&Value>| {
            v.filter(|v| v.get("count").and_then(Value::as_f64).unwrap_or(0.0) > 0.0)
                .cloned()
                .unwrap_or(Value::Null)
        };
        json!({
            "global": pick(raw.get("compaction")),
            "project": pick(key.and_then(|k| raw.get("compactionByDir").and_then(|d| d.get(k)))),
        })
    }

    fn render(&self, g: &Group, data: &Map<String, Value>, loading: bool) -> Value {
        if data.is_empty() {
            return if loading {
                json!({"t": "spinner", "label": "loading…"})
            } else {
                kit::empty("No data yet", "press r to refresh")
            };
        }
        let now = timers::now_ms();
        if let Some(r) = &g.render {
            return r.clone();
        }
        match g.id.as_str() {
            "session" => pages::render_session(data, now),
            "usage" => pages::render_usage(data),
            "tools" => {
                pages::render_inventory(data, "tools", "no tools registered", Some(Scope::Session))
            }
            "skills" => pages::render_inventory(
                data,
                "skills",
                "add skills under ~/.oino/skills or .oino/skills",
                None,
            ),
            "extensions" => pages::render_modules(data),
            id => pages::render_module(id, data, &self.compactor_history(), now).unwrap_or_else(
                || {
                    let s = self.info();
                    pages::render_generic(data, &g.stats, |stat| s.stat_enabled(id, stat))
                },
            ),
        }
    }

    fn dashboard_spec(&self, groups: &[Group], active: &str, loading: &[String]) -> Value {
        let tabs: Vec<Value> = groups
            .iter()
            .map(|g| {
                let data = self
                    .cache
                    .borrow_mut()
                    .cached(&g.id)
                    .map(|p| p.data)
                    .unwrap_or_default();
                let body = if g.id == active || !data.is_empty() {
                    self.render(g, &data, loading.contains(&g.id))
                } else {
                    json!({"t": "spinner"})
                };
                json!({"id": g.id, "title": g.short, "tone": g.accent, "body": body})
            })
            .collect();
        json!({"t": "tabs", "key": "pages", "tabs": tabs, "active": active, "onChange": "page"})
    }

    fn keys() -> Value {
        let mut keys = vec![
            json!({"key": "r", "actionId": "refresh", "label": "Refresh"}),
            json!({"key": "R", "actionId": "refreshAll", "label": "Refresh all"}),
            json!({"key": "q", "actionId": "close", "label": "Close"}),
        ];
        for n in 1..=9 {
            keys.push(json!({"key": n.to_string(), "actionId": format!("page:{n}"), "label": format!("Page {n}")}));
        }
        Value::Array(keys)
    }

    /// `/info [page]`.
    async fn open_dashboard(&self, page: Option<&str>, cx: &Ctx) -> Result<()> {
        if !cx.has_ui() {
            return Ok(());
        }
        self.settings.refresh();
        self.cache.borrow_mut().invalidate("session");
        let groups = self.collect_groups().await;
        let active = page
            .filter(|p| groups.iter().any(|g| g.id == *p))
            .or_else(|| groups.first().map(|g| g.id.as_str()))
            .unwrap_or("session")
            .to_owned();
        let spec = self.dashboard_spec(&groups, &active, std::slice::from_ref(&active));
        let id = ui::panel_open(
            &json!({"panelId": PANEL, "placement": "overlay", "title": "Info", "spec": spec, "keys": Self::keys(), "focus": true, "closeOnEscape": true}),
        )?;
        self.dash.with(cx.session_id(), |d| {
            *d = Dash {
                panel: Some(id),
                active: active.clone(),
                loading: vec![active.clone()],
            };
        });
        self.refresh(&active, cx, false).await;
        Ok(())
    }

    async fn refresh(&self, id: &str, cx: &Ctx, force: bool) {
        self.fetch(id, cx, force).await;
        self.repaint(cx, Some(id));
    }

    fn repaint(&self, cx: &Ctx, done: Option<&str>) {
        let (panel, active, loading) = self.dash.with(cx.session_id(), |d| {
            if let Some(done) = done {
                d.loading.retain(|l| l != done);
            }
            (d.panel.clone(), d.active.clone(), d.loading.clone())
        });
        let Some(panel) = panel else { return };
        let s = self.info();
        let groups: Vec<Group> = self
            .groups
            .borrow()
            .iter()
            .filter(|g| s.group_shown(&g.id))
            .cloned()
            .collect();
        let _ = ui::panel_update(
            &panel,
            &json!({"spec": self.dashboard_spec(&groups, &active, &loading)}),
        );
    }

    async fn on_action(&self, ev: &Event, cx: &Ctx) {
        let sid = cx.session_id().to_owned();
        let panel = self.dash.with(&sid, |d| d.panel.clone());
        let pid = ev
            .get("panelId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if panel.as_deref() != Some(pid) && pid != PANEL {
            return;
        }
        let action = ev
            .get("actionId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let visible: Vec<String> = {
            let s = self.info();
            self.groups
                .borrow()
                .iter()
                .filter(|g| s.group_shown(&g.id))
                .map(|g| g.id.clone())
                .collect()
        };
        let target = match action {
            "page" => ev
                .get("value")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            a if a.starts_with("page:") => a[5..]
                .parse::<usize>()
                .ok()
                .and_then(|n| visible.get(n.checked_sub(1)?).cloned()),
            _ => None,
        };
        match action {
            "$close" => self.dash.remove(&sid),
            "close" => {
                if let Some(p) = panel {
                    ui::panel_close(&p);
                }
                self.dash.remove(&sid);
            }
            "refresh" => {
                let active = self.dash.with(&sid, |d| {
                    d.loading.push(d.active.clone());
                    d.active.clone()
                });
                self.repaint(cx, None);
                self.refresh(&active, cx, true).await;
            }
            "refreshAll" => {
                self.dash.with(&sid, |d| d.loading.clone_from(&visible));
                self.repaint(cx, None);
                for id in &visible {
                    self.refresh(id, cx, true).await;
                }
            }
            _ => {
                if let Some(id) = target {
                    self.dash.with(&sid, |d| {
                        d.active.clone_from(&id);
                        d.loading.push(id.clone());
                    });
                    self.repaint(cx, None);
                    self.refresh(&id, cx, false).await;
                }
            }
        }
    }

    // ── splash ───────────────────────────────────────────────────────────────

    fn splash_facts(&self, cx: &Ctx) -> splash::Facts {
        let now = timers::now_ms();
        let usage = self.cache.borrow_mut().cached("usage");
        let same_day = usage
            .as_ref()
            .is_some_and(|p| p.at / 86_400_000 == now / 86_400_000);
        let raw = usage
            .as_ref()
            .and_then(|p| p.data.get("raw"))
            .and_then(|r| r.get("raw"))
            .cloned()
            .unwrap_or(Value::Null);
        let branch = session::branch(None).unwrap_or_default();
        let r = stats::collect(&branch, None, now);
        let resumed = (r.replies > 0).then(|| {
            let total = r.input + r.output + r.cache_read + r.cache_write;
            (
                r.replies,
                kit::compact(total),
                if r.cost > 0.0 {
                    format!("${:.2}", r.cost)
                } else {
                    "$0".into()
                },
            )
        });
        let modules = services::list()
            .iter()
            .map(|(e, _)| e.clone())
            .chain(self.modules.borrow().iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        splash::Facts {
            ready_ms: Some(self.boot_ms.get()),
            modules: Some(modules.max(self.modules.borrow().len())),
            tools: control::all_tools().ok().map(|t| t.len()),
            today_cost: if same_day {
                raw.pointer("/cost/today").and_then(Value::as_f64)
            } else {
                None
            },
            today_sessions: if same_day {
                raw.get("sessionsToday").and_then(Value::as_u64)
            } else {
                None
            },
            resumed,
            cwd: Some(home_short(&cx.cwd().to_string_lossy())),
            branch: git_branch().or_else(|| host::workspace().git_branch),
            update: self.update.borrow().clone(),
        }
    }

    fn show_splash(&self, cx: &Ctx) {
        let s = self.info();
        if !cx.has_ui() || s.boot_mode == BootMode::Off {
            return;
        }
        let auto = (s.boot_mode == BootMode::AutoClose).then(|| s.boot_timeout_ms.max(500));
        let info = host::info();
        let hour = (timers::now_ms() / 3_600_000).rem_euclid(24);
        let spec = splash::spec(
            unipi_kit::suite::UNIPI_VERSION,
            &info.oino_version,
            &self.splash_facts(cx),
            hour,
            auto.is_none(),
            auto,
        );
        let _ = ui::fire(&splash::header_request(spec));
        self.splash_open.with(cx.session_id(), |o| *o = true);
        if let Some(ms) = auto {
            timers::set(
                &format!("{SPLASH_TIMER}:{}", cx.session_id()),
                Duration::from_millis(ms),
                false,
            );
        }
    }

    fn close_splash(&self, cx: &Ctx) {
        if self.splash_open.with(cx.session_id(), std::mem::take) {
            let _ = ui::fire(&splash::clear_request());
        }
    }

    async fn on_session_start(&self, ev: &Event, cx: &Ctx) {
        self.settings.refresh();
        self.cache.borrow_mut().set_workspace(&host::workspace().id);
        if self.boot_ms.get() == 0 {
            self.boot_ms
                .set((timers::now_ms() - self.started.get()).max(1));
        }
        unipi_kit::bus::emit(&ModuleReady {
            name: unipi_kit::suite::pkg::INFO_SCREEN.npm.to_owned(),
            version: unipi_kit::suite::UNIPI_VERSION.to_owned(),
            commands: vec![COMMAND.to_owned()],
            tools: Vec::new(),
            load_time_ms: None,
        });
        if ev.get("reason").and_then(Value::as_str) == Some("startup") {
            self.show_splash(cx);
        }
        let _ = self.collect_groups().await;
    }

    fn complete(&self, prefix: &str) -> Vec<CompletionItem> {
        let mut groups = self.groups.borrow().clone();
        if groups.is_empty() {
            groups = builtin_groups();
        }
        groups
            .into_iter()
            .filter(|g| g.id.starts_with(prefix.trim()))
            .map(|g| CompletionItem {
                value: g.id.clone(),
                label: Some(g.id),
                description: Some(g.name),
            })
            .collect()
    }
}

impl Extension for InfoScreen {
    #[allow(clippy::too_many_lines, reason = "flat list of registrations")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        self.started.set(timers::now_ms());
        let infos: Vec<_> = builtin_groups().iter().map(Group::info).collect();
        let ns = config::namespace(&infos);
        self.settings
            .declare(config::NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        {
            let mut c = self.cache.borrow_mut();
            c.dir = Some(PathBuf::from("/cache"));
        }
        cx.command(
            Command::<Self>::new(COMMAND)
                .description("Dashboard: this session, usage, tools, skills, modules")
                .argument_hint("[page]")
                .run(|ext, args, cx| async move {
                    let page = args.trim().to_owned();
                    ext.open_dashboard((!page.is_empty()).then_some(page.as_str()), &cx.cx)
                        .await
                })
                .complete(|ext, prefix, _cx| async move { ext.complete(&prefix) }),
        );
        cx.on(EventKind::SessionStart, |ext, ev, cx| async move {
            ext.on_session_start(&ev, &cx).await;
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.cache.borrow_mut().write();
            ext.dash.remove(cx.session_id());
            Ok(())
        });
        for kind in [EventKind::TurnEnd, EventKind::ModelSelect] {
            cx.on(kind, |ext, _ev, cx| async move {
                ext.cache.borrow_mut().invalidate("session");
                let open = ext.dash.with(cx.session_id(), |d| {
                    d.panel.is_some() && d.active == "session"
                });
                if open {
                    ext.refresh("session", &cx, true).await;
                }
                Ok(())
            });
        }
        cx.on(EventKind::Input, |ext, _ev, cx| async move {
            ext.close_splash(&cx);
            Ok(())
        });
        cx.on(EventKind::OinoEditor, |ext, ev, cx| async move {
            if ev.get("intent").and_then(Value::as_str) == Some("changed") {
                ext.close_splash(&cx);
            }
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            if ev.get("timerId").and_then(Value::as_str)
                == Some(format!("{SPLASH_TIMER}:{}", cx.session_id()).as_str())
            {
                ext.close_splash(&cx);
            }
            Ok(())
        });
        cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
            ext.on_action(&ev, &cx).await;
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        cx.on(EventKind::OinoBus, |ext, ev, _cx| async move {
            let Some(d) = Delivery::from_event(&ev) else {
                return Ok(());
            };
            match d.topic.as_str() {
                topics::MODULE_READY => {
                    if let Some(m) = d.decode::<ModuleReady>().filter(|m| {
                        !m.name.is_empty() && m.name != unipi_kit::suite::pkg::INFO_SCREEN.npm
                    }) {
                        let mut list = ext.modules.borrow_mut();
                        if !list.contains(&m.name) {
                            list.push(m.name);
                        }
                    }
                    let mut c = ext.cache.borrow_mut();
                    c.invalidate("extensions");
                    c.invalidate("tools");
                }
                topics::UPDATE_AVAILABLE => {
                    *ext.update.borrow_mut() = d
                        .data
                        .get("latestVersion")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                }
                topics::UPDATE_APPLIED => *ext.update.borrow_mut() = None,
                _ => {}
            }
            Ok(())
        });
        unipi_kit::bus::subscribe(&[
            topics::MODULE_READY,
            topics::UPDATE_AVAILABLE,
            topics::UPDATE_APPLIED,
        ]);
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(InfoScreen);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_match_unipi() {
        let g = builtin_groups();
        let ids: Vec<&str> = g.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["session", "usage", "tools", "skills", "extensions"]);
        assert_eq!(g[4].short, "Modules");
        assert_eq!(g[1].stats.len(), 4);
        assert_eq!(InfoScreen::keys().as_array().map(Vec::len), Some(12));
        assert_eq!(home_short("/nowhere"), "/nowhere");
    }
}
