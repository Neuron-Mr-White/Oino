//! Page data and renderers (unipi `pages/*.ts`, `core-groups.ts`) as OUS nodes.
//!
//! Built-in pages: `session`, `usage`, `tools`, `skills`, `extensions` (Modules). Module pages
//! (`mcp`, `compactor`, `memory`, `web-api`, `updater`) arrive as data from the
//! `info-screen.groups` fan-out and are drawn here, keyed by page id, like unipi
//! (`MODULE_RENDERERS`); unknown pages fall back to their visible stats as key/value rows.

use serde_json::{Map, Value, json};

use crate::kit::{
    Scope, Tile, ago, bold, compact, dim, duration, empty, health, kv, legend, line, money, page,
    section, share_bar, span, tiles,
};
use crate::registry::StatInfo;
use crate::stats::{PARTS, SessionRaw};
use crate::usage::UsageStats;

/// Wraps a raw payload as unipi group data (`{raw: {value: "", raw}}` + stats).
#[must_use]
pub fn group_data(stats: &[(&str, String)], raw: Value) -> Map<String, Value> {
    let mut m = Map::new();
    for (id, v) in stats {
        m.insert((*id).to_owned(), json!({"value": v}));
    }
    let mut wrapped = json!({"value": ""});
    wrapped["raw"] = raw;
    m.insert("raw".into(), wrapped);
    m
}

fn raw(data: &Map<String, Value>) -> Option<&Value> {
    data.get("raw").and_then(|r| r.get("raw"))
}

fn f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn plural(n: u64, w: &str) -> String {
    format!("{n} {w}{}", if n == 1 { "" } else { "s" })
}

// ── session ─────────────────────────────────────────────────────────────────

/// Session group data.
#[must_use]
pub fn session_data(r: &SessionRaw) -> Map<String, Value> {
    group_data(
        &[
            ("cost", money(r.cost)),
            (
                "tokens",
                compact(r.input + r.output + r.cache_read + r.cache_write),
            ),
            ("replies", r.replies.to_string()),
        ],
        serde_json::to_value(r).unwrap_or(Value::Null),
    )
}

fn pressure(pct: f64) -> &'static str {
    if pct > 90.0 {
        "error"
    } else if pct > 70.0 {
        "warning"
    } else {
        "success"
    }
}

/// "This session" page.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
pub fn render_session(data: &Map<String, Value>, now: i64) -> Value {
    let Some(r) = raw(data).and_then(|v| serde_json::from_value::<SessionRaw>(v.clone()).ok())
    else {
        return empty("No session yet", "send a prompt and come back");
    };
    let mut out = Vec::new();
    let title = r.name.as_ref().map_or_else(
        || "this session".to_owned(),
        |n| format!("session  {n}").chars().take(24).collect(),
    );
    let mut model = vec![bold(r.model.clone(), Some("accent"))];
    if !r.thinking.is_empty() && r.thinking != "off" {
        model.push(dim(format!(" · {}", r.thinking)));
    }
    out.push(section(&title, model, None));
    let total = r.input + r.output + r.cache_read + r.cache_write;
    #[allow(clippy::cast_precision_loss, reason = "display")]
    let per = if r.replies > 0 {
        format!("{} / reply", money(r.cost / r.replies as f64))
    } else {
        "nothing spent".into()
    };
    out.push(tiles(&[
        Tile {
            scope: Some(Scope::Session),
            label: "cost".into(),
            value: money(r.cost),
            sub: per,
            tone: "c3",
        },
        Tile {
            scope: Some(Scope::Session),
            label: "tokens".into(),
            value: compact(total),
            sub: format!("{} written", compact(r.output)),
            tone: "info",
        },
        Tile {
            scope: Some(Scope::Session),
            label: "replies".into(),
            value: r.replies.to_string(),
            sub: plural(r.prompts, "prompt"),
            tone: "success",
        },
        Tile {
            scope: Some(Scope::Session),
            label: "time".into(),
            value: duration(now - r.started_at).replace(' ', ""),
            sub: format!(
                "since {}",
                unipi_kit::fmt::iso_from_ms(r.started_at)
                    .get(11..16)
                    .unwrap_or("")
            ),
            tone: "c8",
        },
    ]));
    match &r.context {
        Some(c) if r.ctx_window > 0.0 => {
            let used = r.ctx_tokens.filter(|t| *t > 0.0).unwrap_or(c.used);
            let pct = used / r.ctx_window * 100.0;
            let mut right = vec![
                bold(compact(used), Some(pressure(pct))),
                dim(format!(" of {} · ", compact(r.ctx_window))),
                span(format!("{pct:.0}% full"), Some(pressure(pct))),
            ];
            if c.estimated {
                right.push(dim(" ~"));
            }
            out.push(section("context", right, Some(Scope::Session)));
            out.push(share_bar(
                &PARTS
                    .iter()
                    .map(|(id, label, glyph, tone)| {
                        (
                            format!("{glyph} {label}"),
                            c.parts.get(*id).copied().unwrap_or(0.0),
                            *tone,
                        )
                    })
                    .collect::<Vec<_>>(),
            ));
            out.push(json!({"t": "meter", "value": used, "max": r.ctx_window, "tone": pressure(pct), "label": format!("{pct:.0}%")}));
            let rows: Vec<Value> = PARTS
                .iter()
                .filter_map(|(id, label, glyph, tone)| {
                    let t = c.parts.get(*id).copied().unwrap_or(0.0);
                    (t >= 1.0).then(|| {
                        let share = if c.used > 0.0 { t / c.used * 100.0 } else { 0.0 };
                        json!({"key": [span(*glyph, Some(tone)), span(format!(" {label}"), Some("muted"))], "value": [bold(compact(t), None), dim(format!(" {share:.0}%"))]})
                    })
                })
                .collect();
            out.push(json!({"t": "kv", "items": rows}));
            if c.heavy.first().is_some_and(|h| h.1 >= 500.0) {
                let mut sp = vec![dim("heaviest: ")];
                for (i, (n, t)) in c.heavy.iter().enumerate() {
                    if i > 0 {
                        sp.push(dim("  ·  "));
                    }
                    sp.push(span("▪", Some("c3")));
                    sp.push(span(format!(" {n} "), None));
                    sp.push(bold(compact(*t), None));
                }
                out.push(line(sp));
            }
        }
        _ => out.push(section(
            "context",
            vec![dim("no model selected")],
            Some(Scope::Session),
        )),
    }
    if total > 0.0 {
        let hit = if r.cache_read + r.input > 0.0 {
            r.cache_read / (r.cache_read + r.input) * 100.0
        } else {
            0.0
        };
        out.push(section(
            "billed tokens",
            vec![
                dim("cache hit "),
                bold(format!("{hit:.0}%"), Some("success")),
            ],
            None,
        ));
        let pct = |n: f64| format!("{:.0}%", n / total * 100.0);
        out.push(share_bar(&[
            (
                format!("input {} {}", compact(r.input), pct(r.input)),
                r.input,
                "info",
            ),
            (
                format!("output {} {}", compact(r.output), pct(r.output)),
                r.output,
                "warning",
            ),
            (
                format!("cache read {}", compact(r.cache_read)),
                r.cache_read,
                "success",
            ),
            (
                format!("cache write {}", compact(r.cache_write)),
                r.cache_write,
                "c8",
            ),
        ]));
    }
    if r.per_reply.len() > 1 {
        let peak = r.per_reply.iter().copied().fold(0.0, f64::max);
        out.push(section(
            "per reply",
            vec![dim(format!("peak {}", compact(peak)))],
            None,
        ));
        out.push(json!({"t": "area", "series": [{"label": "tokens", "values": r.per_reply}]}));
        out.push(line(vec![dim(format!(
            "{} replies · last {}",
            r.per_reply.len(),
            compact(r.per_reply.last().copied().unwrap_or(0.0))
        ))]));
    }
    if r.tool_calls > 0 {
        let failed = if r.tool_errors > 0 {
            span(format!("{} failed", r.tool_errors), Some("error"))
        } else {
            span("0 failed", Some("success"))
        };
        out.push(section(
            "tools",
            vec![
                span(r.tool_calls.to_string(), Some("muted")),
                dim(" · "),
                failed,
            ],
            None,
        ));
        #[allow(clippy::cast_precision_loss, reason = "display")]
        let bars: Vec<Value> = r.tools.iter().take(6).map(|(n, c, e)| json!({"label": if *e > 0 { format!("{n} ✗{e}") } else { n.clone() }, "value": *c as f64, "tone": if *e > 0 { "error" } else { "accent" }})).collect();
        out.push(json!({"t": "barChart", "bars": bars}));
        if r.tools.len() > 6 {
            out.push(line(vec![dim(format!("   + {} more", r.tools.len() - 6))]));
        }
    }
    out.push(section("where", vec![], None));
    let home = std::env::var("HOME").unwrap_or_default();
    let cwd = if !home.is_empty() && r.cwd.starts_with(&home) {
        format!("~{}", &r.cwd[home.len()..])
    } else {
        r.cwd.clone()
    };
    out.push(kv(&[
        ("directory", json!(cwd)),
        (
            "branch",
            json!(r.branch.clone().unwrap_or_else(|| "—".into())),
        ),
        ("files edited", json!(r.files.to_string())),
        ("compactions", json!(r.compactions.to_string())),
        (
            "provider",
            json!(if r.provider.is_empty() {
                "—".to_owned()
            } else {
                r.provider.clone()
            }),
        ),
        ("prompts", json!(r.prompts.to_string())),
    ]));
    page(out)
}

// ── usage ───────────────────────────────────────────────────────────────────

/// Usage group data (unipi `usageData`; models of this month, top 8).
#[must_use]
pub fn usage_data(st: &UsageStats) -> Map<String, Value> {
    let mut models: Vec<(String, f64, f64)> = st
        .by_model_month
        .iter()
        .map(|(n, m)| {
            (
                n.strip_prefix("Claude ").unwrap_or(n).to_owned(),
                m.cost,
                m.tokens,
            )
        })
        .collect();
    models.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.total_cmp(&a.2)));
    models.truncate(8);
    let sum = |b: &std::collections::BTreeMap<String, crate::usage::ModelUse>| {
        b.values().map(|m| m.cost).sum::<f64>()
    };
    let raw = json!({
        "tokens": st.tokens, "cost": st.cost, "costWeek": sum(&st.by_model_week), "costMonth": sum(&st.by_model_month),
        "sessions": st.session_count, "sessionsToday": st.sessions_today, "daily": st.daily, "models": models,
        "compaction": st.compaction, "compactionByDir": st.compaction_by_dir,
    });
    group_data(
        &[
            ("tokensToday", compact(st.tokens.today)),
            ("costToday", money(st.cost.today)),
            ("costAllTime", money(st.cost.all_time)),
            ("sessions", st.session_count.to_string()),
        ],
        raw,
    )
}

const MODEL_TONES: &[&str] = &[
    "warning", "info", "c8", "success", "c3", "c4", "c6", "muted",
];

/// Usage history page.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
pub fn render_usage(data: &Map<String, Value>) -> Value {
    let Some(r) = raw(data) else {
        return empty(
            "No usage yet",
            "Oino has no session history on this machine",
        );
    };
    let all_time = r
        .pointer("/cost/allTime")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let use_cost = all_time > 0.0;
    let mut out = vec![section(
        "spend",
        vec![
            span("g", Some(Scope::Global.tone())),
            dim(" all time "),
            bold(money(all_time), Some("c3")),
            dim(format!(" · {} sessions, every project", f(r, "sessions"))),
        ],
        None,
    )];
    let tok = |k: &str| {
        r.pointer(&format!("/tokens/{k}"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let big = |n: f64| {
        if n >= 100.0 {
            if n >= 1000.0 {
                format!("${}", compact(n))
            } else {
                format!("${}", n.round())
            }
        } else {
            money(n)
        }
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "counts"
    )]
    let today_sessions = f(r, "sessionsToday") as u64;
    out.push(tiles(&if use_cost {
        vec![
            Tile {
                scope: Some(Scope::Global),
                label: "today".into(),
                value: big(r
                    .pointer("/cost/today")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)),
                sub: format!(
                    "{} tok · {}",
                    compact(tok("today")),
                    plural(today_sessions, "session")
                ),
                tone: "c3",
            },
            Tile {
                scope: Some(Scope::Global),
                label: "this week".into(),
                value: big(f(r, "costWeek")),
                sub: format!("{} tokens", compact(tok("week"))),
                tone: "warning",
            },
            Tile {
                scope: Some(Scope::Global),
                label: "this month".into(),
                value: big(f(r, "costMonth")),
                sub: format!("{} tokens", compact(tok("month"))),
                tone: "warning",
            },
        ]
    } else {
        ["today", "week", "month"]
            .iter()
            .map(|k| Tile {
                scope: Some(Scope::Global),
                label: (*k).into(),
                value: compact(tok(k)),
                sub: "tokens".into(),
                tone: "info",
            })
            .collect()
    }));
    let series: Vec<f64> = r
        .get("daily")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|d| f(d, if use_cost { "cost" } else { "tokens" }))
        .collect();
    if !series.is_empty() {
        let fmt = |v: f64| if use_cost { money(v) } else { compact(v) };
        let total: f64 = series.iter().sum();
        #[allow(clippy::cast_precision_loss, reason = "30 days")]
        let avg = total / series.len() as f64;
        out.push(section(
            "last 30 days",
            vec![dim(format!("{} total · {}/day avg", fmt(total), fmt(avg)))],
            Some(Scope::Global),
        ));
        let n = series.len();
        let bars: Vec<Value> = series.iter().enumerate().map(|(i, v)| json!({"label": if i + 1 == n { "today".to_owned() } else { format!("-{}", n - 1 - i) }, "value": v, "tone": if i + 1 == n { "c3" } else { "warning" }})).collect();
        out.push(json!({"t": "barChart", "bars": bars, "columns": true}));
    }
    let models = r
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !models.is_empty() {
        let metric = |m: &Value| {
            m.get(if use_cost { 1 } else { 2 })
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        let total: f64 = models
            .iter()
            .map(metric)
            .sum::<f64>()
            .max(f64::MIN_POSITIVE);
        out.push(section(
            "models this month",
            vec![dim(plural(models.len() as u64, "model"))],
            Some(Scope::Global),
        ));
        let name = |m: &Value| {
            m.get(0)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        if models.len() == 1 {
            let m = &models[0];
            out.push(line(vec![
                span("●", Some(MODEL_TONES[0])),
                span(" ", None),
                bold(name(m), None),
                dim("  every token this month  "),
                bold(
                    if use_cost {
                        money(metric(m))
                    } else {
                        compact(metric(m))
                    },
                    None,
                ),
            ]));
        } else {
            out.push(share_bar(
                &models
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (name(m), metric(m), MODEL_TONES[i % MODEL_TONES.len()]))
                    .collect::<Vec<_>>(),
            ));
            let items: Vec<Value> = models
                .iter()
                .take(6)
                .enumerate()
                .map(|(i, m)| json!({"id": name(m), "label": name(m), "badge": format!("{} {:.0}%", if use_cost { money(metric(m)) } else { compact(metric(m)) }, metric(m) / total * 100.0), "tone": MODEL_TONES[i % MODEL_TONES.len()]}))
                .collect();
            out.push(json!({"t": "list", "items": items}));
            if models.len() > 6 {
                out.push(legend(&[(
                    format!("{} more models", models.len() - 6),
                    "muted",
                    None,
                )]));
            }
        }
    }
    page(out)
}

// ── inventory (tools / skills) ──────────────────────────────────────────────

/// One inventory item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    /// Name.
    pub name: String,
    /// Source group.
    pub group: String,
    /// Active (`None` = unknown).
    pub active: Option<bool>,
}

const GROUP_TONES: &[&str] = &[
    "info", "warning", "c8", "success", "c3", "c4", "c6", "muted",
];

/// unipi `sourceLabel` for an Oino `sourceInfo`.
#[must_use]
pub fn source_label(info: &Value) -> String {
    let src = s(info, "source");
    if src == "builtin" || src == "built-in" {
        return "built-in".into();
    }
    if src == "sdk" {
        return "sdk".into();
    }
    if let Some(id) = src.strip_prefix("extension:") {
        return if id.starts_with("unipi.") {
            "unipi".into()
        } else {
            id.to_owned()
        };
    }
    if let Some(n) = src.strip_prefix("npm:") {
        return n
            .rsplit('/')
            .next()
            .unwrap_or(n)
            .split('@')
            .next()
            .unwrap_or("npm")
            .to_owned();
    }
    if s(info, "scope") == "project" {
        return "project".into();
    }
    "local".into()
}

/// Inventory group data (counts per group; items sorted by group size then name).
#[must_use]
pub fn inventory_data(
    items: &[Named],
    active_count: Option<usize>,
    group_scopes: &[(&str, Scope)],
) -> Map<String, Value> {
    let mut groups: Vec<(String, usize)> = Vec::new();
    for i in items {
        match groups.iter_mut().find(|(g, _)| *g == i.group) {
            Some(g) => g.1 += 1,
            None => groups.push((i.group.clone(), 1)),
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    let rank = |g: &str| {
        groups
            .iter()
            .position(|(x, _)| x == g)
            .unwrap_or(usize::MAX)
    };
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| {
        rank(&a.group)
            .cmp(&rank(&b.group))
            .then_with(|| a.name.cmp(&b.name))
    });
    let raw = json!({
        "items": sorted.iter().map(|i| json!({"name": i.name, "group": i.group, "active": i.active})).collect::<Vec<_>>(),
        "groups": groups,
        "activeCount": active_count,
        "groupScopes": group_scopes.iter().map(|(g, s)| ((*g).to_owned(), json!(s.tag()))).collect::<Map<_, _>>(),
    });
    let summary = groups
        .iter()
        .map(|(g, n)| format!("{n} {g}"))
        .collect::<Vec<_>>()
        .join(", ");
    group_data(
        &[("total", items.len().to_string()), ("groups", summary)],
        raw,
    )
}

fn scope_tag(t: &str) -> Option<Scope> {
    match t {
        "s" => Some(Scope::Session),
        "p" => Some(Scope::Project),
        "g" => Some(Scope::Global),
        _ => None,
    }
}

/// Tools / skills page.
#[must_use]
pub fn render_inventory(
    data: &Map<String, Value>,
    noun: &str,
    empty_hint: &str,
    header_scope: Option<Scope>,
) -> Value {
    let Some(r) = raw(data) else {
        return empty(&format!("No {noun} loaded"), empty_hint);
    };
    let items = r
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return empty(&format!("No {noun} loaded"), empty_hint);
    }
    let groups: Vec<(String, f64)> = r
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| Some((g.get(0)?.as_str()?.to_owned(), g.get(1)?.as_f64()?)))
        .collect();
    let tone_of = |g: &str| {
        groups
            .iter()
            .position(|(x, _)| x == g)
            .map_or("muted", |i| GROUP_TONES[i % GROUP_TONES.len()])
    };
    let n = items.len();
    #[allow(clippy::cast_precision_loss, reason = "display")]
    let mut head = vec![Tile {
        scope: None,
        label: noun.into(),
        value: n.to_string(),
        sub: format!("from {}", plural(groups.len() as u64, "source")),
        tone: "accent",
    }];
    let active = r.get("activeCount").and_then(Value::as_u64);
    if let Some(a) = active {
        head.push(Tile {
            scope: None,
            label: "active".into(),
            value: a.to_string(),
            sub: format!("{} switched off", n as u64 - a.min(n as u64)),
            tone: "success",
        });
    }
    for (g, c) in groups.iter().take(if active.is_some() { 2 } else { 3 }) {
        #[allow(clippy::cast_precision_loss, reason = "display")]
        head.push(Tile {
            scope: None,
            label: g.clone(),
            value: compact(*c),
            sub: format!("{:.0}%", c / n as f64 * 100.0),
            tone: tone_of(g),
        });
    }
    let scopes = r.get("groupScopes").cloned().unwrap_or(Value::Null);
    let mut out = vec![tiles(&head)];
    out.push(share_bar(
        &groups
            .iter()
            .map(|(g, c)| (g.clone(), *c, tone_of(g)))
            .collect::<Vec<_>>(),
    ));
    out.push(legend(
        &groups
            .iter()
            .map(|(g, c)| {
                (
                    format!("{g} {c}"),
                    tone_of(g),
                    scopes.get(g).and_then(Value::as_str).and_then(scope_tag),
                )
            })
            .collect::<Vec<_>>(),
    ));
    out.push(section(
        &format!("all {noun}"),
        vec![dim("● active  ○ off")],
        header_scope,
    ));
    let list: Vec<Value> = items
        .iter()
        .map(|i| {
            let off = i.get("active").and_then(Value::as_bool) == Some(false);
            let name = s(i, "name");
            json!({"id": name, "label": [span(if off { "○ " } else { "● " }, Some(tone_of(s(i, "group")))), if off { dim(name) } else { span(name, None) }]})
        })
        .collect();
    out.push(json!({"t": "list", "items": list, "columns": "auto"}));
    page(out)
}

// ── modules ─────────────────────────────────────────────────────────────────

/// One extension row of the Modules page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// Extension id (unipi modules without the `unipi.` prefix).
    pub name: String,
    /// Tools.
    pub tools: u64,
    /// Commands.
    pub commands: u64,
    /// Services (Oino) / settings fields (unipi).
    pub services: u64,
}

/// Modules group data.
#[must_use]
pub fn modules_data(
    unipi: &[Module],
    others: &[Module],
    version: &str,
    boot_ms: i64,
) -> Map<String, Value> {
    let row = |m: &Module| json!({"name": m.name, "tools": m.tools, "commands": m.commands, "services": m.services});
    group_data(
        &[("count", (unipi.len() + others.len()).to_string())],
        json!({"version": version, "unipi": unipi.iter().map(row).collect::<Vec<_>>(), "others": others.iter().map(row).collect::<Vec<_>>(), "totalLoadMs": boot_ms}),
    )
}

/// Modules page.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
pub fn render_modules(data: &Map<String, Value>) -> Value {
    let Some(r) = raw(data) else {
        return empty(
            "No modules announced yet",
            "modules report in as they finish loading",
        );
    };
    let unipi = r
        .get("unipi")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let others = r
        .get("others")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if unipi.is_empty() && others.is_empty() {
        return empty(
            "No modules announced yet",
            "modules report in as they finish loading",
        );
    }
    let sum = |k: &str| unipi.iter().map(|m| f(m, k)).sum::<f64>();
    let ms = f(r, "totalLoadMs");
    let ms_s = if ms >= 1000.0 {
        format!("{:.1}s", ms / 1000.0)
    } else if ms < 1.0 {
        "<1ms".into()
    } else {
        format!("{}ms", ms.round())
    };
    let v = s(r, "version");
    let mut out = vec![tiles(&[
        Tile {
            scope: None,
            label: "modules".into(),
            value: unipi.len().to_string(),
            sub: if v.is_empty() {
                "unipi".into()
            } else {
                format!("unipi v{v}")
            },
            tone: "c7",
        },
        Tile {
            scope: None,
            label: "tools".into(),
            value: sum("tools").to_string(),
            sub: format!("{} commands", sum("commands")),
            tone: "info",
        },
        Tile {
            scope: None,
            label: "services".into(),
            value: sum("services").to_string(),
            sub: "shared between modules".into(),
            tone: "c8",
        },
        Tile {
            scope: None,
            label: "boot".into(),
            value: ms_s,
            sub: "until ready".into(),
            tone: "success",
        },
    ])];
    if !unipi.is_empty() {
        out.push(section(
            "what each module adds",
            vec![
                span("■", Some("info")),
                dim(" tools  "),
                span("■", Some("warning")),
                dim(" commands  "),
                span("■", Some("c8")),
                dim(" services"),
            ],
            None,
        ));
        let mut rows = unipi.clone();
        rows.sort_by(|a, b| {
            (f(b, "tools") + f(b, "commands") + f(b, "services"))
                .total_cmp(&(f(a, "tools") + f(a, "commands") + f(a, "services")))
                .then_with(|| s(a, "name").cmp(s(b, "name")))
        });
        let table_rows: Vec<Value> = rows
            .iter()
            .map(|m| {
                json!([
                    s(m, "name"),
                    f(m, "tools").to_string(),
                    f(m, "commands").to_string(),
                    f(m, "services").to_string()
                ])
            })
            .collect();
        out.push(json!({"t": "table", "columns": [{"title": "module", "width": "fill"}, {"title": "tools", "align": "right"}, {"title": "commands", "align": "right"}, {"title": "services", "align": "right"}], "rows": table_rows}));
    }
    if !others.is_empty() {
        out.push(section(
            "other extensions",
            vec![dim("tools · commands")],
            None,
        ));
        let items: Vec<Value> = others.iter().map(|o| json!({"id": s(o, "name"), "label": [span("◆ ", Some("c7")), span(s(o, "name"), None), dim(format!(" {}·{}", f(o, "tools"), f(o, "commands")))]})).collect();
        out.push(json!({"t": "list", "items": items, "columns": "auto"}));
    }
    page(out)
}

// ── module pages ────────────────────────────────────────────────────────────

fn on(v: bool) -> Value {
    if v {
        span("● on", Some("success"))
    } else {
        span("○ off", Some("muted"))
    }
}

fn render_mcp(r: &Value) -> Value {
    let servers = r
        .get("servers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if servers.is_empty() {
        return empty("No MCP servers", "add one in /settings → MCP");
    }
    let running = servers
        .iter()
        .filter(|x| s(x, "status") == "running")
        .count();
    let failed = servers.iter().filter(|x| s(x, "status") == "error").count();
    let tools: f64 = servers.iter().map(|x| f(x, "tools")).sum();
    let h = |st: &str| match st {
        "running" => "ok",
        "starting" => "warn",
        "error" => "bad",
        _ => "off",
    };
    let mut out = vec![tiles(&[
        Tile {
            scope: None,
            label: "servers".into(),
            value: servers.len().to_string(),
            sub: format!("{running} running"),
            tone: "c5",
        },
        Tile {
            scope: None,
            label: "tools".into(),
            value: tools.to_string(),
            sub: "bridged into Oino".into(),
            tone: "info",
        },
        Tile {
            scope: None,
            label: "failed".into(),
            value: failed.to_string(),
            sub: if failed > 0 {
                "see below".into()
            } else {
                "all healthy".into()
            },
            tone: if failed > 0 { "error" } else { "success" },
        },
    ])];
    out.push(section(
        "servers",
        vec![
            span("g", Some(Scope::Global.tone())),
            dim(" ~/.oino  "),
            span("p", Some(Scope::Project.tone())),
            dim(" .oino in project"),
        ],
        None,
    ));
    let items: Vec<Value> = servers
        .iter()
        .map(|x| {
            let scope = if matches!(s(x, "scope"), "" | "global") { Scope::Global } else { Scope::Project };
            let mut v = json!({"id": s(x, "name"), "label": [span("● ", Some(health(h(s(x, "status"))))), span(scope.tag(), Some(scope.tone())), span(" ", None), bold(s(x, "name"), None)], "badge": format!("{} · {} tools", s(x, "status"), f(x, "tools")), "tone": health(h(s(x, "status")))});
            if let Some(e) = x.get("error").and_then(Value::as_str) {
                v["detail"] = json!(e.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            v
        })
        .collect();
    out.push(json!({"t": "list", "items": items}));
    page(out)
}

#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
fn render_compactor(r: &Value, history: &Value, now: i64) -> Value {
    let sess = r.get("session").cloned().unwrap_or_else(|| json!({"count": r.get("history").and_then(Value::as_array).map_or(0, Vec::len), "saved": f(r, "saved"), "avoided": 0, "dollars": 0, "sent": 0}));
    let money_short = |n: f64| {
        if n >= 1000.0 {
            format!("${}", compact(n))
        } else if n >= 100.0 {
            format!("${}", n.round())
        } else if n >= 1.0 {
            format!("${n:.2}")
        } else if n > 0.0 {
            format!("${n:.3}")
        } else {
            "$0".into()
        }
    };
    let tile = |label: &str, v: &Value, scope: Scope, tone: &'static str| Tile {
        scope: Some(scope),
        label: label.into(),
        value: money_short(f(v, "dollars")),
        sub: format!("{} tok · {}×", compact(f(v, "avoided")), f(v, "count")),
        tone,
    };
    let g = history.get("global").filter(|v| f(v, "count") > 0.0);
    let p = history.get("project").filter(|v| f(v, "count") > 0.0);
    let mut row = Vec::new();
    if let Some(v) = g {
        row.push(tile("global", v, Scope::Global, "c3"));
    }
    if let Some(v) = p {
        row.push(tile("project", v, Scope::Project, "success"));
    }
    row.push(tile("session", &sess, Scope::Session, "info"));
    let mut out = vec![
        section(
            "saved by compaction",
            vec![dim("tokens not re-sent · ≈ money not spent")],
            None,
        ),
        tiles(&row),
    ];
    let (best, sc) = g
        .map(|v| (v, Scope::Global))
        .or(p.map(|v| (v, Scope::Project)))
        .unwrap_or((&sess, Scope::Session));
    if f(best, "avoided") > 0.0 {
        let without = f(best, "sent") + f(best, "avoided");
        out.push(section(
            "context sent",
            vec![dim(format!(
                "{:.0}% less",
                f(best, "avoided") / without * 100.0
            ))],
            Some(sc),
        ));
        out.push(json!({"t": "meter", "value": f(best, "sent"), "max": without, "label": format!("with {} · without {}", compact(f(best, "sent")), compact(without)), "tone": "accent"}));
    }
    out.push(section(
        "this session",
        vec![dim("before → after · replies since")],
        Some(Scope::Session),
    ));
    let items = r
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        out.push(line(vec![dim(
            "  No compaction yet — the context has stayed small.",
        )]));
    } else {
        let start = items.len().saturating_sub(6);
        let rows: Vec<Value> = items[start..]
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let after = if f(h, "after") > 0.0 {
                    f(h, "after")
                } else {
                    f(h, "before")
                };
                let saved = if f(h, "context") > 0.0 {
                    (f(h, "before") - after).max(0.0) * f(h, "replies") * f(h, "paid")
                        / f(h, "context")
                } else {
                    0.0
                };
                json!([
                    format!("#{}", start + i + 1),
                    format!("{} → {}", compact(f(h, "before")), compact(after)),
                    format!("×{}", f(h, "replies")),
                    money_short(saved)
                ])
            })
            .collect();
        out.push(json!({"t": "table", "columns": [{"title": "#"}, {"title": "before → after", "width": "fill"}, {"title": "replies", "align": "right"}, {"title": "saved", "align": "right"}], "rows": rows}));
    }
    #[allow(clippy::cast_possible_truncation, reason = "ms")]
    let last = items
        .last()
        .map_or_else(|| "—".into(), |h| ago(f(h, "at") as i64, now));
    out.push(section("settings", vec![], None));
    out.push(kv(&[
        ("method", json!([bold(s(r, "method"), None)])),
        ("runs", json!(s(r, "trigger"))),
        ("Oino's /compact", json!(s(r, "piCompact"))),
        ("last", json!(last)),
    ]));
    page(out)
}

fn render_updater(r: &Value, now: i64) -> Value {
    let latest = r.get("latest").and_then(Value::as_str);
    let available = r.get("available").and_then(Value::as_bool).unwrap_or(false);
    let state = if latest.is_none() || available {
        "warn"
    } else {
        "ok"
    };
    #[allow(clippy::cast_possible_truncation, reason = "ms")]
    let checked = f(r, "checkedAt") as i64;
    page(vec![
        tiles(&[
            Tile {
                scope: None,
                label: "installed".into(),
                value: s(r, "current").to_owned(),
                sub: format!("v{}", s(r, "current")),
                tone: "c7",
            },
            Tile {
                scope: None,
                label: "latest".into(),
                value: latest.unwrap_or("-").to_owned(),
                sub: latest.map_or_else(|| "checking…".into(), |l| format!("v{l}")),
                tone: if available { "c3" } else { "success" },
            },
        ]),
        line(vec![
            span("● ", Some(health(state))),
            if latest.is_none() {
                span("checking", None)
            } else if available {
                bold("update ready", Some("c3"))
            } else {
                span("up to date", Some("success"))
            },
        ]),
        section("details", vec![], None),
        kv(&[
            ("last check", json!(ago(checked, now))),
            (
                "auto update",
                json!(r.get("mode").and_then(Value::as_str).unwrap_or("—")),
            ),
            ("update", json!(if available { "oino update" } else { "—" })),
            ("changelog", json!("/changelog")),
        ]),
    ])
}

#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
fn render_web(r: &Value) -> Value {
    let providers = r
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let usable = providers
        .iter()
        .filter(|p| {
            p.get("enabled").and_then(Value::as_bool) == Some(true)
                && p.get("hasKey").and_then(Value::as_bool) == Some(true)
        })
        .count();
    let cache = r.get("cache").cloned().unwrap_or(Value::Null);
    let mut out = vec![tiles(&[
        Tile {
            scope: None,
            label: "providers".into(),
            value: usable.to_string(),
            sub: format!("of {} usable", providers.len()),
            tone: "c4",
        },
        Tile {
            scope: None,
            label: "cache".into(),
            value: compact(f(&cache, "entries")),
            sub: format!(
                "{:.1} MB · {} expired",
                f(&cache, "bytes") / 1024.0 / 1024.0,
                f(&cache, "expired")
            ),
            tone: "info",
        },
    ])];
    out.push(section("tools", vec![], None));
    let chips: Vec<Value> = r
        .get("tools")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .flat_map(|(n, ok)| {
            let ok = ok.as_bool() == Some(true);
            [
                span("● ", Some(health(if ok { "ok" } else { "off" }))),
                if ok {
                    span(format!("{n}    "), None)
                } else {
                    dim(format!("{n}    "))
                },
            ]
        })
        .collect();
    out.push(line(chips));
    out.push(section("providers", vec![], None));
    let rows: Vec<Value> = providers
        .iter()
        .map(|p| {
            let enabled = p.get("enabled").and_then(Value::as_bool) == Some(true);
            let has_key = p.get("hasKey").and_then(Value::as_bool) == Some(true);
            let note = if !enabled {
                "off"
            } else if !has_key {
                "needs key"
            } else if p.get("keyed").and_then(Value::as_bool) == Some(true) {
                "key set"
            } else {
                "free"
            };
            let caps: Vec<&str> = p
                .get("caps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let cell = |c: &str| if caps.contains(&c) { "■" } else { "·" };
            json!([
                s(p, "name"),
                note,
                cell("search"),
                cell("read"),
                cell("summarize")
            ])
        })
        .collect();
    out.push(json!({"t": "table", "columns": [{"title": "provider", "width": "fill"}, {"title": ""}, {"title": "search"}, {"title": "read"}, {"title": "sum"}], "rows": rows}));
    if r.get("smartFetch").is_some_and(|v| !v.is_null()) || r.get("wigolo").is_some() {
        let sf = r.get("smartFetch").and_then(Value::as_array).map_or_else(
            || json!([span("ready", Some("success"))]),
            |m| {
                json!([span(
                    format!(
                        "missing {}",
                        m.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Some("error")
                )])
            },
        );
        out.push(kv(&[
            ("smart-fetch", sf),
            ("wigolo", json!(s(r, "wigolo").trim_start_matches("✓ "))),
        ]));
    }
    page(out)
}

fn render_memory(r: &Value) -> Value {
    let total = f(r, "total");
    let proj = f(r, "projectCount");
    let mut out = vec![tiles(&[
        Tile {
            scope: Some(Scope::Global),
            label: "global".into(),
            value: compact(total),
            sub: "all projects".into(),
            tone: "c3",
        },
        Tile {
            scope: Some(Scope::Project),
            label: "project".into(),
            value: compact(proj),
            sub: s(r, "project").to_owned(),
            tone: "success",
        },
        Tile {
            scope: None,
            label: "share".into(),
            value: format!(
                "{:.0}%",
                if total > 0.0 {
                    proj / total * 100.0
                } else {
                    0.0
                }
            ),
            sub: "project / global".into(),
            tone: "accent",
        },
    ])];
    let mut types: Vec<(String, f64)> = r
        .get("types")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0)))
        .collect();
    types.sort_by(|a, b| b.1.total_cmp(&a.1));
    let tone = |t: &str| match t {
        "decision" => "warning",
        "pattern" => "info",
        "summary" => "c8",
        "preference" => "success",
        _ => "muted",
    };
    if !types.is_empty() {
        out.push(section("by type", vec![], Some(Scope::Project)));
        out.push(share_bar(
            &types
                .iter()
                .map(|(t, n)| (format!("{t} {n}"), *n, tone(t)))
                .collect::<Vec<_>>(),
        ));
    }
    out.push(section("behaviour", vec![], None));
    out.push(kv(&[
        (
            "recall at start",
            json!([on(r.get("recall").and_then(Value::as_bool) == Some(true))]),
        ),
        (
            "save prompts",
            json!([on(r.get("write").and_then(Value::as_bool) == Some(true))]),
        ),
        ("pending writes", json!(f(r, "pending").to_string())),
    ]));
    page(out)
}

/// Generic page: visible stats as key/value rows (unipi default view).
#[must_use]
pub fn render_generic(
    data: &Map<String, Value>,
    stats: &[StatInfo],
    visible: impl Fn(&str) -> bool,
) -> Value {
    let rows: Vec<Value> = stats
        .iter()
        .filter(|st| st.show && visible(&st.id))
        .filter_map(|st| {
            let d = data.get(&st.id)?;
            let mut value = vec![bold(s(d, "value"), None)];
            if let Some(detail) = d.get("detail").and_then(Value::as_str) {
                value.push(dim(format!("  {detail}")));
            }
            Some(json!({"key": st.label, "value": value}))
        })
        .collect();
    if rows.is_empty() {
        return empty("Nothing to show", "this page has no visible stats");
    }
    json!({"t": "kv", "items": rows})
}

/// Module page renderer by id (`None` = generic view).
#[must_use]
pub fn render_module(
    id: &str,
    data: &Map<String, Value>,
    compactor_history: &Value,
    now: i64,
) -> Option<Value> {
    let r = raw(data)?;
    Some(match id {
        "mcp" => render_mcp(r),
        "compactor" => render_compactor(r, compactor_history, now),
        "updater" => render_updater(r, now),
        "web-api" => render_web(r),
        "memory" => render_memory(r),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::collect;

    fn texts(v: &Value) -> String {
        v.to_string()
    }

    #[test]
    fn session_page() {
        assert_eq!(render_session(&Map::new(), 0)["t"], "empty");
        let mut r = collect(&[], Some(0), 0);
        r.model = "Opus".into();
        r.cost = 1.5;
        r.replies = 3;
        let t = texts(&render_session(&session_data(&r), 95_000));
        assert!(
            t.contains("$1.50")
                && t.contains("$0.50 / reply")
                && t.contains("1m35s")
                && t.contains("no model selected")
        );
    }

    #[test]
    fn usage_page_tiles_and_models() {
        let mut st = UsageStats {
            daily: vec![crate::usage::Day::default(); 30],
            ..UsageStats::default()
        };
        st.cost.all_time = 10.0;
        st.by_model_month.insert(
            "Claude Opus".into(),
            crate::usage::ModelUse {
                tokens: 5.0,
                cost: 4.0,
                sessions: 1,
            },
        );
        st.by_model_month.insert(
            "haiku".into(),
            crate::usage::ModelUse {
                tokens: 9.0,
                cost: 1.0,
                sessions: 1,
            },
        );
        let d = usage_data(&st);
        assert_eq!(d["costAllTime"]["value"], "$10.00");
        let t = texts(&render_usage(&d));
        assert!(
            t.contains("THIS WEEK")
                && t.contains("models this month".to_uppercase().as_str())
                && t.contains("\"Opus\"")
        );
        assert!(t.contains("80%"), "{t}");
    }

    #[test]
    fn inventory_and_scopes() {
        let items = vec![
            Named {
                name: "read".into(),
                group: "built-in".into(),
                active: Some(true),
            },
            Named {
                name: "ask_user".into(),
                group: "unipi".into(),
                active: Some(false),
            },
            Named {
                name: "bash".into(),
                group: "built-in".into(),
                active: Some(true),
            },
        ];
        let d = inventory_data(
            &items,
            Some(2),
            &[("project", Scope::Project), ("user", Scope::Global)],
        );
        assert_eq!(d["groups"]["value"], "2 built-in, 1 unipi");
        let r = raw(&d).cloned().unwrap_or_default();
        assert_eq!(r["items"][0]["name"], "bash");
        assert_eq!(r["groupScopes"]["project"], "p");
        let t = texts(&render_inventory(&d, "tools", "none", Some(Scope::Session)));
        assert!(t.contains("ALL TOOLS s") && t.contains("1 switched off") && t.contains("○ "));
        assert_eq!(
            source_label(&json!({"source": "extension:unipi.memory"})),
            "unipi"
        );
        assert_eq!(source_label(&json!({"source": "builtin"})), "built-in");
        assert_eq!(
            source_label(&json!({"source": "extension:acme.tools"})),
            "acme.tools"
        );
        assert_eq!(
            source_label(&json!({"source": "local", "scope": "project"})),
            "project"
        );
    }

    #[test]
    #[allow(clippy::many_single_char_names, reason = "tile positions g/p/s")]
    fn modules_and_module_pages() {
        let d = modules_data(
            &[Module {
                name: "footer".into(),
                tools: 0,
                commands: 1,
                services: 0,
            }],
            &[Module {
                name: "acme".into(),
                tools: 2,
                commands: 0,
                services: 0,
            }],
            "3.0.0",
            412,
        );
        let t = texts(&render_modules(&d));
        assert!(
            t.contains("unipi v3.0.0")
                && t.contains("412ms")
                && t.contains("other extensions".to_uppercase().as_str())
        );
        let mem = group_data(
            &[],
            json!({"total": 10, "projectCount": 4, "project": "p", "types": {"decision": 3}, "recall": true, "write": false, "pending": 0}),
        );
        let t = texts(&render_module("memory", &mem, &Value::Null, 0).unwrap_or_default());
        assert!(
            t.find("GLOBAL").unwrap_or(9999) < t.find("PROJECT").unwrap_or(0)
                && t.contains("project / global")
                && t.contains("pending writes")
        );
        let comp = group_data(
            &[],
            json!({"method": "vcc", "trigger": "70%", "piCompact": "vcc", "history": [], "items": []}),
        );
        let hist = json!({"global": {"count": 2, "dollars": 1.0, "avoided": 100, "sent": 50}, "project": {"count": 1, "dollars": 0.5, "avoided": 10, "sent": 5}});
        let t = texts(&render_module("compactor", &comp, &hist, 0).unwrap_or_default());
        let (g, p, s) = (
            t.find("GLOBAL").unwrap_or(0),
            t.find("PROJECT").unwrap_or(0),
            t.find("SESSION").unwrap_or(0),
        );
        assert!(g < p && p < s, "tile order global < project < session");
        assert!(render_module("zzz", &mem, &Value::Null, 0).is_none());
        let generic = render_generic(
            &group_data(&[("a", "1".into())], json!({})),
            &[StatInfo {
                id: "a".into(),
                label: "A".into(),
                show: true,
            }],
            |_| true,
        );
        assert_eq!(generic["items"][0]["key"], "A");
    }
}
