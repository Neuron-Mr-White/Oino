//! Control-centre data and OUS views (unipi app-bridge `runState`, `buildStats`,
//! `fusionPreset`, `lhMode`/`lhProgress`, `dreamStatus`, `infoGroups`, UNI-160).

use serde::Serialize;
use serde_json::{Value, json};
use unipi_kit::tray::TrayTab;

/// Extension id.
pub(crate) const EXTENSION_ID: &str = "unipi.app-bridge";
/// Panel id.
pub(crate) const PANEL_ID: &str = "unipi.app-bridge/control";
/// Long-horizon modes the chip offers (unipi `set_lh_mode`).
pub(crate) const LH_MODES: [&str; 5] = ["goal", "ralph", "swarm", "graph", "none"];
/// unipi `thinkingLevels`.
pub(crate) const THINKING_LEVELS: [&str; 6] = ["off", "minimal", "low", "medium", "high", "xhigh"];

/// unipi `RunState.context`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Context {
    /// Tokens in context.
    pub tokens: Option<u64>,
    /// Window.
    pub window: Option<u64>,
    /// Percent.
    pub percent: Option<f64>,
}

/// unipi `StatsInfo` (+ run state).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stats {
    /// `idle | running | compacting | retrying | waiting`.
    pub run_state: String,
    /// Model `provider/id`.
    pub model: Option<String>,
    /// Model display name.
    pub model_name: Option<String>,
    /// Reasoning model.
    pub reasoning: bool,
    /// Thinking level.
    pub thinking: String,
    /// Context usage.
    pub context: Context,
    /// Input tokens (branch assistant usage).
    pub tokens_in: u64,
    /// Output tokens.
    pub tokens_out: u64,
    /// Cache reads.
    pub cache_hit: u64,
    /// Cost (USD, 4 decimals).
    pub cost: f64,
    /// Tokens per second (footer).
    pub tps: f64,
}

/// Sums assistant usage over entries (unipi `buildStats` + `computeCost`, child usage included).
pub(crate) fn usage_totals(entries: &[Value]) -> (u64, u64, u64, f64) {
    let (mut input, mut output, mut cache, mut cost) = (0, 0, 0, 0.0);
    for e in entries {
        let msg = e.get("message");
        let is_assistant = e.get("type").and_then(Value::as_str) == Some("message")
            && msg.and_then(|m| m.get("role")).and_then(Value::as_str) == Some("assistant");
        if is_assistant {
            let u = msg.and_then(|m| m.get("usage"));
            let n = |k: &str| {
                u.and_then(|u| u.get(k))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            input += n("input");
            output += n("output");
            cache += n("cacheRead");
            cost += u
                .and_then(|u| u.pointer("/cost/total"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
        } else if e.get("type").and_then(Value::as_str) == Some("usage") {
            cost += e
                .pointer("/usage/cost/total")
                .or_else(|| e.pointer("/cost/total"))
                .or_else(|| e.get("cost"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
        }
    }
    (input, output, cache, (cost * 10_000.0).round() / 10_000.0)
}

/// `ContextUsage` JSON → [`Context`].
pub(crate) fn context_of(v: &Value) -> Context {
    Context {
        tokens: v.get("tokens").and_then(Value::as_u64),
        window: v.get("contextWindow").and_then(Value::as_u64),
        percent: v.get("percent").and_then(Value::as_f64),
    }
}

/// unipi `thinkingLevels`.
pub(crate) fn thinking_levels(reasoning: bool) -> Vec<&'static str> {
    if reasoning {
        THINKING_LEVELS.to_vec()
    } else {
        vec!["off"]
    }
}

/// unipi `infoGroups`: title + visible stat rows, ≤ 40 groups, ≤ 24 rows, values ≤ 200 chars.
pub(crate) fn info_rows(groups: &[Value]) -> Vec<(String, Vec<(String, String)>)> {
    groups
        .iter()
        .take(40)
        .filter_map(|g| {
            let name = g.get("name").or_else(|| g.get("id"))?.as_str()?.to_owned();
            let data = g.get("data")?;
            let stats: Vec<(String, String)> =
                match g.pointer("/config/stats").and_then(Value::as_array) {
                    Some(list) if !list.is_empty() => list
                        .iter()
                        .filter(|s| s.get("show").and_then(Value::as_bool) != Some(false))
                        .filter_map(|s| {
                            let id = s.get("id")?.as_str()?;
                            let label = s.get("label").and_then(Value::as_str).unwrap_or(id);
                            Some((
                                label.to_owned(),
                                data.pointer(&format!("/{id}/value"))?.as_str()?.to_owned(),
                            ))
                        })
                        .collect(),
                    _ => data
                        .as_object()
                        .map(|o| {
                            o.iter()
                                .filter(|(k, _)| *k != "raw")
                                .filter_map(|(k, v)| {
                                    Some((k.clone(), v.get("value")?.as_str()?.to_owned()))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                };
            let stats: Vec<(String, String)> = stats
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(l, v)| (l, v.chars().take(200).collect()))
                .take(24)
                .collect();
            (!stats.is_empty()).then_some((name, stats))
        })
        .collect()
}

fn fmt_tokens(n: u64) -> String {
    unipi_kit::fmt::format_tokens(n)
}

/// "Session" tab.
pub(crate) fn session_tab(s: &Stats, models: &[Value], waiting: Option<&str>) -> Value {
    let model_options: Vec<Value> = models
        .iter()
        .filter_map(|m| {
            let key = format!(
                "{}/{}",
                m.get("provider")?.as_str()?,
                m.get("id")?.as_str()?
            );
            let label = m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&key)
                .to_owned();
            Some(json!({"value": key, "label": label}))
        })
        .collect();
    let levels: Vec<Value> = thinking_levels(s.reasoning)
        .into_iter()
        .map(|l| json!({"value": l, "label": l}))
        .collect();
    let ctx_line = match (s.context.tokens, s.context.window) {
        (Some(t), Some(w)) => format!("{} / {}", fmt_tokens(t), fmt_tokens(w)),
        (None, Some(w)) => format!("? / {}", fmt_tokens(w)),
        _ => "unknown".into(),
    };
    let mut children = vec![json!({"t": "kv", "items": [
        {"key": "State", "value": match waiting { Some(w) => format!("{} · waiting on {w}", s.run_state), None => s.run_state.clone() }},
        {"key": "Model", "value": s.model_name.clone().or_else(|| s.model.clone()).unwrap_or_else(|| "none".into())},
        {"key": "Context", "value": ctx_line},
        {"key": "Tokens", "value": format!("↑{} ↓{} · cache {}", fmt_tokens(s.tokens_in), fmt_tokens(s.tokens_out), fmt_tokens(s.cache_hit))},
        {"key": "Cost", "value": format!("${:.4}", s.cost)},
        {"key": "Speed", "value": format!("{:.1} tok/s", s.tps)},
    ]})];
    if let (Some(p), Some(_)) = (s.context.percent, s.context.window) {
        children.push(json!({"t": "meter", "value": p, "max": 100, "label": "context", "tone": if p >= 80.0 { "warning" } else { "accent" }}));
    }
    if !model_options.is_empty() {
        children.push(json!({"t": "select", "id": "model", "options": model_options, "value": s.model, "onChange": "set_model"}));
    }
    children.push(json!({"t": "select", "id": "thinking", "options": levels, "value": s.thinking, "onChange": "set_thinking"}));
    children.push(
        json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
            {"t": "button", "label": "Compact", "actionId": "compact"},
            {"t": "button", "label": "Abort", "actionId": "abort", "tone": "error"},
            {"t": "button", "label": "Refresh", "actionId": "refresh"},
        ]}),
    );
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": children})
}

/// "Work" tab: every work-tray tab's pane in one column.
pub(crate) fn work_tab(tabs: &[TrayTab]) -> Value {
    if tabs.is_empty() {
        return json!({"t": "empty", "message": "No background work."});
    }
    let sections: Vec<Value> = tabs
        .iter()
        .map(|t| {
            let title = if t.counts.running > 0 {
                format!("{} ({} · {} running)", t.label, t.counts.total, t.counts.running)
            } else {
                format!("{} ({})", t.label, t.counts.total)
            };
            json!({"t": "section", "key": t.id, "title": title, "body": t.body.clone().unwrap_or_else(|| json!({"t": "empty", "message": "Nothing here."}))})
        })
        .collect();
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": sections})
}

/// "Modes" tab: long-horizon mode chip, progress, workflow, Fusion pair, dream.
pub(crate) fn modes_tab(
    lh: Option<&Value>,
    progress: Option<&Value>,
    workflow: Option<&Value>,
    fusion: Option<&Value>,
    dream: Option<&Value>,
) -> Value {
    let mode = lh
        .and_then(|v| v.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("none");
    let paused = lh.and_then(|v| v.get("paused")).and_then(Value::as_str);
    let options: Vec<Value> = LH_MODES
        .iter()
        .map(|m| json!({"value": m, "label": if *m == "none" { "regular" } else { m }}))
        .collect();
    let mut children = vec![
        json!({"t": "section", "title": "Long-horizon", "body": {"t": "stack", "direction": "vertical", "children": [
            {"t": "select", "id": "lh", "options": options, "value": mode, "onChange": "set_lh_mode"},
            {"t": "text", "value": match paused { Some(p) => format!("Parked: {p}"), None => String::new() }},
        ]}}),
    ];
    if let Some(p) = progress.filter(|p| !p.is_null()) {
        let md = p
            .get("markdown")
            .or_else(|| p.get("text"))
            .and_then(Value::as_str)
            .map_or_else(
                || {
                    format!(
                        "```json\n{}\n```",
                        serde_json::to_string_pretty(p).unwrap_or_default()
                    )
                },
                ToOwned::to_owned,
            );
        children.push(json!({"t": "section", "title": "Progress", "collapsible": true, "body": {"t": "markdown", "value": md}}));
    }
    if let Some(w) = workflow.filter(|w| !w.is_null()) {
        let plan = w.get("planMode").and_then(Value::as_bool).unwrap_or(false);
        let perm = w
            .get("permissionMode")
            .and_then(Value::as_str)
            .unwrap_or("default");
        children.push(
            json!({"t": "section", "title": "Workflow", "body": {"t": "kv", "items": [
                {"key": "Plan mode", "value": if plan { "on" } else { "off" }},
                {"key": "Permissions", "value": perm},
            ]}}),
        );
    }
    let fusion_body = match fusion.filter(|f| !f.is_null()) {
        Some(f) => json!({"t": "stack", "direction": "vertical", "children": [
            {"t": "kv", "items": [
                {"key": "Lead", "value": format!("{} ({})", f.get("leadName").and_then(Value::as_str).unwrap_or("?"), f.get("leadEffort").and_then(Value::as_str).unwrap_or("?"))},
                {"key": "Sidekick", "value": format!("{} ({})", f.get("sidekickName").and_then(Value::as_str).unwrap_or("?"), f.get("sidekickEffort").and_then(Value::as_str).unwrap_or("?"))},
                {"key": "Saved", "value": f.get("savedUsd").and_then(Value::as_f64).map_or_else(|| "—".to_owned(), |s| format!("${s:.2}"))},
            ]},
            {"t": "button", "label": "Change pair…", "actionId": "fusion_pick"},
        ]}),
        None => json!({"t": "stack", "direction": "vertical", "children": [
            {"t": "empty", "message": "Fusion is off."},
            {"t": "button", "label": "Pick models…", "actionId": "fusion_pick"},
        ]}),
    };
    children.push(json!({"t": "section", "title": "Fusion", "body": fusion_body}));
    if let Some(d) = dream.filter(|d| !d.is_null()) {
        let running = d.get("running").and_then(Value::as_bool).unwrap_or(false);
        let summary = d
            .get("summary")
            .or_else(|| d.get("status"))
            .and_then(Value::as_str)
            .map_or_else(
                || {
                    if running {
                        "dreaming…".to_owned()
                    } else {
                        "idle".to_owned()
                    }
                },
                ToOwned::to_owned,
            );
        children.push(json!({"t": "section", "title": "Dream", "body": {"t": "stack", "direction": "vertical", "children": [
            {"t": "text", "value": summary},
            {"t": "button", "label": if running { "Stop dream" } else { "Dream now" }, "actionId": if running { "dream_stop" } else { "dream_run" }},
        ]}}));
    }
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": children})
}

/// "BTW" tab: side question box + the last answers.
pub(crate) fn btw_tab(history: &[(String, String)], pending: bool) -> Value {
    let mut children = vec![
        json!({"t": "input", "id": "btw", "placeholder": "Ask a side question (does not touch the main session)…", "onSubmit": "btw_ask", "multiline": true}),
    ];
    if pending {
        children.push(json!({"t": "spinner", "label": "thinking…"}));
    }
    for (q, a) in history.iter().rev() {
        children.push(json!({"t": "card", "title": q, "body": {"t": "markdown", "value": a}}));
    }
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": children})
}

/// "Info" tab: info-screen groups as key/value cards.
pub(crate) fn info_tab(rows: &[(String, Vec<(String, String)>)]) -> Value {
    if rows.is_empty() {
        return json!({"t": "empty", "message": "No info groups."});
    }
    let cards: Vec<Value> = rows
        .iter()
        .map(|(name, stats)| {
            let items: Vec<Value> = stats
                .iter()
                .map(|(k, v)| json!({"key": k, "value": v}))
                .collect();
            json!({"t": "card", "title": name, "body": {"t": "kv", "items": items}})
        })
        .collect();
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": cards})
}

/// The whole panel spec.
pub(crate) fn panel_spec(active: &str, tabs: Vec<(&str, &str, Value)>) -> Value {
    let tabs: Vec<Value> = tabs
        .into_iter()
        .map(|(id, title, body)| json!({"id": id, "title": title, "body": body}))
        .collect();
    json!({"t": "tabs", "tabs": tabs, "active": active, "onChange": "$tab"})
}

/// `openPanel` request.
pub(crate) fn open_request(spec: &Value) -> Value {
    json!({
        "panelId": PANEL_ID,
        "placement": "sidePane",
        "title": "Control centre",
        "spec": spec,
        "keys": [
            {"key": "r", "actionId": "refresh", "label": "Refresh"},
            {"key": "c", "actionId": "compact", "label": "Compact"},
        ],
        "focus": false,
        "closeOnEscape": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_like_unipi() {
        let entries = vec![
            json!({"type": "message", "message": {"role": "user", "content": "hello"}}),
            json!({"type": "message", "message": {"role": "assistant", "usage": {"cost": {"total": 0.01}, "input": 100, "output": 50, "cacheRead": 20}}}),
            json!({"type": "message", "message": {"role": "assistant", "usage": {"cost": {"total": 0.000_04}, "input": 1, "output": 2}}}),
            json!({"type": "usage", "usage": {"cost": {"total": 0.5}}}),
        ];
        assert_eq!(usage_totals(&entries), (101, 52, 20, 0.51));
        assert_eq!(thinking_levels(false), ["off"]);
        assert_eq!(thinking_levels(true).len(), 6);
        let c = context_of(&json!({"tokens": 1000, "contextWindow": 10000, "percent": 10.0}));
        assert_eq!(
            c,
            Context {
                tokens: Some(1000),
                window: Some(10000),
                percent: Some(10.0)
            }
        );
    }

    #[test]
    fn info_groups_like_unipi() {
        let groups = vec![
            json!({"id": "mcp", "name": "MCP Servers", "config": {"stats": [{"id": "total", "label": "Total servers", "show": true}, {"id": "hidden", "label": "H", "show": false}, {"id": "empty", "label": "E"}]}, "data": {"total": {"value": "2"}, "hidden": {"value": "x"}, "empty": {"value": ""}}}),
            json!({"id": "bare", "data": {"a": {"value": "1"}, "raw": {"value": "", "raw": {}}}}),
            json!({"id": "nothing", "data": {}}),
        ];
        let rows = info_rows(&groups);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            (
                "MCP Servers".into(),
                vec![("Total servers".into(), "2".into())]
            )
        );
        assert_eq!(rows[1], ("bare".into(), vec![("a".into(), "1".into())]));
    }

    #[test]
    fn views_render() {
        let s = Stats {
            run_state: "running".into(),
            model: Some("p/m".into()),
            model_name: Some("Model M".into()),
            reasoning: true,
            thinking: "medium".into(),
            context: Context {
                tokens: Some(1000),
                window: Some(10_000),
                percent: Some(10.0),
            },
            tokens_in: 100,
            tokens_out: 50,
            cache_hit: 20,
            cost: 0.01,
            tps: 12.3,
        };
        let models = vec![json!({"provider": "p", "id": "m", "name": "Model M"})];
        let tab = session_tab(&s, &models, Some("subagent"));
        let text = tab.to_string();
        assert!(text.contains("running · waiting on subagent"));
        assert!(text.contains("\"onChange\":\"set_model\""));
        assert!(text.contains("12.3 tok/s"));
        let modes_view = modes_tab(
            Some(&json!({"mode": "goal", "paused": "ralph"})),
            None,
            Some(&json!({"planMode": true, "permissionMode": "auto"})),
            None,
            Some(&json!({"running": true})),
        );
        let m = modes_view.to_string();
        assert!(m.contains("\"value\":\"goal\""));
        assert!(m.contains("Parked: ralph"));
        assert!(m.contains("Fusion is off."));
        assert!(m.contains("dream_stop"));
        let work = work_tab(&[]);
        assert_eq!(work["t"], "empty");
        let btw = btw_tab(&[("q?".into(), "a!".into())], true);
        assert_eq!(btw["children"][1]["t"], "spinner");
        assert_eq!(btw["children"][2]["title"], "q?");
        let spec = panel_spec("session", vec![("session", "Session", tab)]);
        assert_eq!(spec["onChange"], "$tab");
        let open = open_request(&spec);
        assert_eq!(open["placement"], "sidePane");
        assert_eq!(open["panelId"], PANEL_ID);
        assert_eq!(info_tab(&[])["t"], "empty");
    }
}
