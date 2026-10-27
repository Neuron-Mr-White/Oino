//! `unipi.skill-registry` — skill exposure control (unipi `packages/skill-registry`,
//! unipi-port §3.18).
//!
//! | unipi | here |
//! |---|---|
//! | `before_agent_start`: registry (proxy) + exposure `judged\|all\|off`, hidden-skills section | [`Registry::before_agent_start`] → modified `systemPromptOptions` (`skills`, `sections`) |
//! | judged exposure: pins, jev, freeze, later reveals | [`judge`] |
//! | vault `~/.unipi/skill-vault` via `resources_discover` | `~/.oino/skill-vault` (`fs.paths` alias `skill-vault`) → `skillPaths` |
//! | `/skill:name` blocked when off or its condition fails | `input` → `{action: "handled"}` |
//! | core skill conditions | fan-out `skill-condition.check{skill}` → `{hidden, reason?}` |
//! | `unipi:skills:reveal {names}` | bus subscription |
//! | `/skills` → hub on Skills; runner `skills-editor` (skill grid) | `openSettings{namespace:"skills"}`; [`editor`] overlay panel |
//! | `skills` settings | [`settings`] |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod editor;
pub mod judge;
pub mod registry;
pub mod settings;

use std::collections::{BTreeMap, BTreeSet};

use futures::future::join_all;
use oino_extension::prelude::*;
use oino_extension::session::{CustomMessage, Provenance, SendOptions};
use oino_extension::sync::SessionMap;
use unipi_kit::bus::Delivery;
use unipi_kit::services::fanout as fan;
use unipi_kit::settings::Store;
use unipi_kit::topics::{self, ModuleReady};
use unipi_kit::{fanout, jev};

use crate::editor::{Editor, EditorSkill, Outcome, Scope};
use crate::judge::{
    Entry, HIDDEN_SECTION, REVEALED_MESSAGE, SKILLS_JUDGED_ENTRY, SKILLS_REVEALED_ENTRY,
    SessionState, TurnInput,
};
use crate::registry::{
    CatalogSkill, Places, apply_registry, default_must_show, is_bundled_skill_location,
    is_under_dir, skill_command_name, skill_dir, skill_source,
};
use crate::settings::{EDITOR_COMMAND, EXTENSION_ID, ExposureMode, Layer, NAMESPACE};

/// Vault alias (manifest `fs.paths`).
const VAULT_GUEST: &str = "/skill-vault";

#[derive(Default)]
struct Session {
    judge: SessionState,
    catalog: Vec<CatalogSkill>,
    disabled: BTreeSet<String>,
    editor: Option<OpenEditor>,
}

/// `(panel id, editor, global before, project before, proxy before)`.
type OpenEditor = (
    String,
    Editor,
    BTreeMap<String, registry::SkillState>,
    BTreeMap<String, registry::SkillState>,
    bool,
);

/// The extension.
#[derive(Default)]
pub struct Registry {
    settings: Store,
    sessions: SessionMap<Session>,
}

fn host_of(guest: &str) -> Option<String> {
    oino_extension::fs::preopens()
        .into_iter()
        .find(|p| p.guest_path == guest)
        .map(|p| p.host_path)
}

/// Guest path of a host path under one of the preopens.
fn guest_of(host: &str) -> Option<String> {
    oino_extension::fs::preopens().into_iter().find_map(|p| {
        let rest = host.strip_prefix(p.host_path.trim_end_matches('/'))?;
        rest.starts_with('/')
            .then(|| format!("{}{rest}", p.guest_path.trim_end_matches('/')))
    })
}

fn parent(p: &str) -> Option<String> {
    p.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(a, _)| a.to_owned())
}

/// Host places from the preopens.
fn places(cwd: &str) -> Places {
    let agent_dir = ["/agent/skills", "/agent/sessions", "/agent/bundled"]
        .iter()
        .find_map(|g| host_of(g))
        .and_then(|h| parent(&h));
    let vault = host_of(VAULT_GUEST)
        .or_else(|| agent_dir.as_ref().map(|a| format!("{a}/skill-vault")))
        .unwrap_or_else(|| "~/.oino/skill-vault".to_owned());
    let home = agent_dir.as_deref().and_then(parent);
    Places {
        cwd: cwd.to_owned(),
        vault,
        home,
        agent_dir,
    }
}

fn to_entry(s: &CatalogSkill) -> Entry {
    Entry {
        name: s.name.clone(),
        description: s.description.clone(),
        location: skill_dir(s),
    }
}

fn provenance(title: &str, synopsis: &str) -> Provenance {
    Provenance {
        source: "Skills".into(),
        title: Some(title.into()),
        synopsis: Some(synopsis.into()),
        severity: None,
    }
}

impl Registry {
    fn resolved(&self) -> settings::Skills {
        settings::resolve(&self.settings.get(NAMESPACE), settings::stored_proxy())
    }

    fn jev_settings() -> jev::Settings {
        let all = control::settings()
            .ok()
            .and_then(|s| s.get("extensionSettings").cloned())
            .unwrap_or(Value::Null);
        jev::Settings::resolve(&all, Some(NAMESPACE))
    }

    /// Condition providers' verdict per skill: `Some(reason)` = hidden.
    async fn conditions(names: &[String]) -> BTreeMap<String, String> {
        let providers = fanout::providers_of(
            &oino_extension::services::list(),
            fan::SKILL_CONDITION_CHECK,
            Some(EXTENSION_ID),
        );
        if providers.is_empty() {
            return BTreeMap::new();
        }
        let checks = names.iter().map(|n| async move {
            let replies = fanout::call_all(
                fan::SKILL_CONDITION_CHECK,
                &json!({"skill": n}),
                fanout::DEFAULT_TIMEOUT,
                Some(EXTENSION_ID),
            )
            .await;
            replies
                .iter()
                .find(|r| r.value.get("hidden") == Some(&Value::Bool(true)))
                .map(|r| {
                    let reason = r.value.get("reason").and_then(Value::as_str).map_or_else(
                        || format!("Skill \"{n}\" is unavailable right now."),
                        ToOwned::to_owned,
                    );
                    (n.clone(), reason)
                })
        });
        join_all(checks).await.into_iter().flatten().collect()
    }

    fn all_skills(&self, cx: &Ctx, p: &Places) -> Vec<CatalogSkill> {
        let mut by: BTreeMap<String, CatalogSkill> = BTreeMap::new();
        for s in self.sessions.with(cx.session_id(), |s| s.catalog.clone()) {
            by.entry(s.name.clone()).or_insert(s);
        }
        for c in control::commands().unwrap_or_default() {
            if c.get("source").and_then(Value::as_str) != Some("skill") {
                continue;
            }
            let name = c
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim_start_matches("skill:")
                .to_owned();
            by.entry(name.clone()).or_insert_with(|| CatalogSkill {
                name,
                description: c
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                file_path: c
                    .pointer("/sourceInfo/path")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                base_dir: None,
                ..CatalogSkill::default()
            });
        }
        for v in registry::list_vault_skills(VAULT_GUEST, &p.vault) {
            by.entry(v.name.clone()).or_insert(v);
        }
        // Skills only known from the command list (no prompt yet): read `default-enabled`
        // from the file when a preopen reaches it.
        for s in by.values_mut() {
            if s.default_enabled
                && let Some(text) = s
                    .file_path
                    .as_deref()
                    .and_then(guest_of)
                    .and_then(|g| std::fs::read_to_string(g).ok())
            {
                s.default_enabled = !registry::is_opt_in(&text);
            }
        }
        by.into_values().collect()
    }

    fn open_editor(&self, cx: &Ctx) -> Result<()> {
        if !cx.has_ui() {
            return Ok(());
        }
        let p = places(&cx.cwd().display().to_string());
        let mut skills: Vec<EditorSkill> = self
            .all_skills(cx, &p)
            .iter()
            .map(|s| EditorSkill {
                name: s.name.clone(),
                description: s.description.clone(),
                source: skill_source(s, &p),
                default_enabled: s.default_enabled,
            })
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        let global = settings::read_layer(&self.settings, Layer::Global);
        let project = settings::read_layer(&self.settings, Layer::Project);
        let proxy = self.resolved().proxy;
        let ed = Editor {
            skills,
            scope: if project.is_empty() {
                Scope::Global
            } else {
                Scope::Project
            },
            global: global.clone(),
            project: project.clone(),
            proxy,
            filter: String::new(),
        };
        let id = ui::panel_open(&ed.open_request())?;
        self.sessions.with(cx.session_id(), |s| {
            s.editor = Some((id, ed, global, project, proxy));
        });
        Ok(())
    }

    fn editor_action(&self, cx: &Ctx, panel: &str, action: &str, value: Option<&Value>) {
        let sid = cx.session_id().to_owned();
        let Some((id, mut ed, before_g, before_p, before_proxy)) =
            self.sessions.with(&sid, |s| s.editor.take())
        else {
            return;
        };
        if id != panel && panel != editor::PANEL {
            self.sessions.with(&sid, |s| {
                s.editor = Some((id, ed, before_g, before_p, before_proxy));
            });
            return;
        }
        match ed.act(action, value) {
            Outcome::Save => {
                ui::panel_close(&id);
                let writes =
                    editor::count_changes((&before_g, &before_p), (&ed.global, &ed.project));
                if ed.global != before_g {
                    settings::write_layer(&self.settings, Layer::Global, &ed.global);
                }
                if ed.project != before_p {
                    settings::write_layer(&self.settings, Layer::Project, &ed.project);
                }
                let proxy_changed = ed.proxy != before_proxy;
                if proxy_changed {
                    settings::set_proxy(&self.settings, ed.proxy);
                }
                if writes > 0 || proxy_changed {
                    self.sessions.with(&sid, |s| s.judge.judged = false);
                    let extra = if proxy_changed {
                        format!(", proxy {}", if ed.proxy { "on" } else { "off" })
                    } else {
                        String::new()
                    };
                    let _ = ui::notify(
                        &format!(
                            "Skill settings saved ({writes} change{}{extra}) — applies from the next prompt",
                            if writes == 1 { "" } else { "s" }
                        ),
                        "info",
                    );
                }
            }
            Outcome::Cancel => {
                if action != "$close" {
                    ui::panel_close(&id);
                }
            }
            Outcome::Changed => {
                let _ = ui::panel_update(&id, &json!({"spec": ed.spec()}));
                self.sessions.with(&sid, |s| {
                    s.editor = Some((id, ed, before_g, before_p, before_proxy));
                });
            }
            Outcome::Ignored => self.sessions.with(&sid, |s| {
                s.editor = Some((id, ed, before_g, before_p, before_proxy));
            }),
        }
    }

    async fn on_input(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let name = skill_command_name(ev.get("text").and_then(Value::as_str).unwrap_or_default())?;
        if let Some(reason) = Self::conditions(std::slice::from_ref(&name))
            .await
            .remove(&name)
        {
            if cx.has_ui() {
                let _ = ui::notify(&reason, "warning");
            }
            return Some(json!({"action": "handled"}));
        }
        if !self
            .sessions
            .with(cx.session_id(), |s| s.disabled.contains(&name))
        {
            return None;
        }
        if cx.has_ui() {
            let _ = ui::notify(
                &format!("Skill \"{name}\" is turned off here. Turn it on in /skills."),
                "warning",
            );
        }
        Some(json!({"action": "handled"}))
    }

    async fn on_reveal(&self, d: &Delivery, cx: &Ctx) {
        let names: Vec<String> = d
            .data
            .get("names")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|n| n.as_str().map_or_else(|| n.to_string(), ToOwned::to_owned))
            .collect();
        let entries: Vec<Entry> = self.sessions.with(cx.session_id(), |s| {
            let fresh: Vec<Entry> = s
                .judge
                .hidden
                .iter()
                .filter(|h| names.contains(&h.name) && !s.judge.revealed.contains(&h.name))
                .cloned()
                .collect();
            for e in &fresh {
                s.judge.revealed.insert(e.name.clone());
            }
            fresh
        });
        if entries.is_empty() {
            return;
        }
        let revealed: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
        let home = places(&cx.cwd().display().to_string()).home;
        let _ = session::append_entry(SKILLS_REVEALED_ENTRY, Some(&json!({"names": revealed})));
        let _ = session::send_message(
            &CustomMessage {
                custom_type: REVEALED_MESSAGE.into(),
                content: json!(judge::reveal_message(&entries, home.as_deref())),
                display: true,
                details: Some(json!({"names": revealed, "lines": entries.iter().map(|e| format!("{} — {}", e.name, e.description)).collect::<Vec<_>>()})),
            },
            SendOptions {
                trigger_turn: false,
                deliver_as: None,
                provenance: Some(provenance("Skill reveal", &revealed.join(", "))),
            },
        )
        .await;
    }

    #[allow(
        clippy::too_many_lines,
        reason = "registry, exposure and judging of one hook"
    )]
    async fn before_agent_start(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let cwd = cx.cwd().display().to_string();
        let p = places(&cwd);
        let mut options = ev.get("systemPromptOptions").cloned()?;
        let raw_skills: Vec<Value> = options
            .get("skills")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let full: Vec<CatalogSkill> = raw_skills.iter().map(CatalogSkill::from_value).collect();
        self.sessions
            .with(cx.session_id(), |s| s.catalog.clone_from(&full));
        let cfg = self.resolved();
        let hidden_by_condition =
            Self::conditions(&full.iter().map(|s| s.name.clone()).collect::<Vec<_>>()).await;
        let catalog: Vec<usize> = (0..full.len())
            .filter(|i| !hidden_by_condition.contains_key(&full[*i].name))
            .collect();
        let mut disabled = BTreeSet::new();
        let mut must_show = BTreeSet::new();
        // Opt-in skills core reports as off are unlisted (never judged or indexed) but stay
        // invocable by `/skill:name`.
        let catalog: Vec<usize> = catalog
            .into_iter()
            .filter(|i| full[*i].enabled || cfg.proxy)
            .collect();
        let pool: Vec<usize> = if cfg.proxy {
            let sub: Vec<CatalogSkill> = catalog.iter().map(|i| full[*i].clone()).collect();
            let r = apply_registry(&sub, &cfg.states, &p);
            disabled = r.disabled;
            must_show = r.must_show;
            r.listed.iter().map(|j| catalog[*j]).collect()
        } else {
            let mut pool = Vec::new();
            for i in catalog {
                if is_under_dir(&skill_dir(&full[i]), &p.vault) {
                    disabled.insert(full[i].name.clone());
                } else {
                    if default_must_show(&full[i].name, skill_source(&full[i], &p)) {
                        must_show.insert(full[i].name.clone());
                    }
                    pool.push(i);
                }
            }
            pool
        };
        self.sessions
            .with(cx.session_id(), |s| s.disabled.clone_from(&disabled));
        let set_skills = |options: &mut Value, keep: &dyn Fn(usize) -> bool| {
            let list: Vec<Value> = pool
                .iter()
                .filter(|i| keep(**i))
                .map(|i| raw_skills[*i].clone())
                .collect();
            options["skills"] = Value::Array(list);
        };
        let sections = |options: &mut Value, index: Option<&str>| {
            if !options.get("sections").is_some_and(Value::is_object) {
                options["sections"] = json!({});
            }
            if let Some(map) = options["sections"].as_object_mut() {
                match index {
                    Some(t) if !t.is_empty() => {
                        map.insert(HIDDEN_SECTION.into(), json!(t));
                    }
                    _ => {
                        map.remove(HIDDEN_SECTION);
                    }
                }
            }
        };
        if cfg.exposure.mode != ExposureMode::Judged {
            if cfg.exposure.mode == ExposureMode::Off {
                set_skills(&mut options, &|i| {
                    must_show.contains(&full[i].name)
                        || !is_bundled_skill_location(&skill_dir(&full[i]))
                });
            } else {
                set_skills(&mut options, &|_| true);
            }
            sections(&mut options, None);
            return Some(json!({"systemPromptOptions": options}));
        }
        let entries: Vec<Entry> = pool.iter().map(|i| to_entry(&full[*i])).collect();
        let prompt = ev
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let cwd_base = cwd.rsplit('/').next().unwrap_or_default().to_owned();
        let mut state = self
            .sessions
            .with(cx.session_id(), |s| std::mem::take(&mut s.judge));
        let js = Self::jev_settings();
        let out = judge::decide_turn(
            &TurnInput {
                prompt: &prompt,
                catalog: &entries,
                settings: &cfg.exposure,
                cwd_base: &cwd_base,
                must_show: &must_show,
                home: p.home.as_deref(),
            },
            &mut state,
            |s, q| {
                let js = js.clone();
                async move { jev::ask(&js, &s, &q).await }
            },
        )
        .await;
        self.sessions.with(cx.session_id(), |s| s.judge = state);
        set_skills(&mut options, &|i| out.listed.contains(&full[i].name));
        sections(&mut options, Some(&out.index));
        if let Some(freeze) = &out.freeze {
            let _ = session::append_entry(SKILLS_JUDGED_ENTRY, Some(freeze));
        }
        if let Some(st) = &out.status
            && cx.has_ui()
        {
            let _ = ui::status("skills", Some(st));
        }
        let mut result = json!({"systemPromptOptions": options});
        if !out.reveal.is_empty() {
            let names: Vec<String> = out.reveal.iter().map(|r| r.name.clone()).collect();
            let _ = session::append_entry(SKILLS_REVEALED_ENTRY, Some(&json!({"names": names})));
            result["message"] = json!({
                "customType": REVEALED_MESSAGE,
                "content": judge::reveal_message(&out.reveal, p.home.as_deref()),
                "display": true,
                "details": {"names": names, "oinoProvenance": {"source": "Skills", "title": "Skill reveal", "synopsis": names.join(", ")}},
            });
        }
        Some(result)
    }
}

impl Extension for Registry {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        self.settings.declare(NAMESPACE, settings::defaults());
        self.settings.load(cx.settings());
        cx.settings_namespace(&settings::namespace().to_json());

        cx.on(EventKind::ResourcesDiscover, |_ext, _ev, _cx| async move {
            let vault = host_of(VAULT_GUEST);
            Ok(vault.map(|v| json!({"skillPaths": [v]})))
        });
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.settings.refresh();
            if !cx.is_child() {
                settings::import_legacy(&ext.settings);
            }
            let entries = session::entries().unwrap_or_default();
            ext.sessions.with(cx.session_id(), |s| {
                *s = Session {
                    judge: judge::restore_state(&entries),
                    ..Session::default()
                };
            });
            unipi_kit::bus::emit(&ModuleReady {
                name: "@pi-unipi/skill-registry".into(),
                version: host::info().extension_version,
                commands: vec!["skills".into()],
                tools: Vec::new(),
                load_time_ms: None,
            });
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::Input, |ext, ev, cx| async move {
            Ok(ext.on_input(&ev, &cx).await)
        });
        cx.on(EventKind::BeforeAgentStart, |ext, ev, cx| async move {
            Ok(ext.before_agent_start(&ev, &cx).await)
        });
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            if let Some(d) = Delivery::from_event(&ev)
                && d.topic == topics::SKILLS_REVEAL
            {
                ext.on_reveal(&d, &cx).await;
            }
            Ok(())
        });
        cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
            let panel = ev
                .get("panelId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let action = ev
                .get("actionId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            ext.editor_action(&cx, &panel, &action, ev.get("value"));
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, cx| async move {
            if ext.settings.on_changed(&ev.payload).as_deref() == Some(NAMESPACE) {
                ext.sessions
                    .with(cx.session_id(), |s| s.judge.judged = false);
            }
            Ok(())
        });
        bus::subscribe(topics::SKILLS_REVEAL);

        cx.command(
            Command::<Self>::new("skills")
                .description("Skill settings — proxy, exposure, and per-skill Enabled / Must show (opens /settings on Skills)")
                .run(|_ext, _args, cx| async move {
                    if cx.has_ui() {
                        ui::fire(&json!({"method": "openSettings", "namespace": NAMESPACE}))?;
                    }
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new(EDITOR_COMMAND)
                .description("Per-skill Enabled / Must show editor")
                .hidden()
                .run(|ext, _args, cx| async move { ext.open_editor(&cx.cx) }),
        );
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Registry);
