//! Panels (unipi `src/tui/*-overlay.ts` → OUS, ui-protocol §5/§6) and the settings namespace.

use serde_json::{Value, json};
use unipi_kit::settings::{Choice, Field, Namespace, Section, choice};

use crate::channel::CheckResult;
use crate::readme::Readme;

/// Extension id.
pub(crate) const EXTENSION_ID: &str = "unipi.updater";
/// Settings namespace.
pub(crate) const NAMESPACE: &str = "updater";

/// unipi `DEFAULT_CONFIG`.
pub(crate) fn defaults() -> Value {
    json!({"checkIntervalMs": 3_600_000, "autoUpdate": "notify"})
}

/// `updater` namespace (unipi `registerSettings({namespace: "updater"})`).
pub(crate) fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Updater", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("updates", "Updates")
                .field(
                    Field::enumeration(
                        "checkIntervalMs",
                        "Check interval",
                        [
                            ("1800000", "30 min"),
                            ("3600000", "1 hour"),
                            ("21600000", "6 hours"),
                            ("86400000", "daily"),
                        ]
                        .into_iter()
                        .map(|(value, label)| Choice {
                            value: value.to_owned(),
                            label: label.to_owned(),
                            description: None,
                        })
                        .collect(),
                    )
                    .description("How often to check GitHub for a new Oino release."),
                )
                .field(
                    Field::enumeration(
                        "autoUpdate",
                        "Auto update",
                        vec![
                            choice("disabled", "disabled", "check nothing; update manually"),
                            choice("notify", "notify", "tell you; you install"),
                        ],
                    )
                    .description("What happens when an update is found."),
                ),
        )
}

/// Normalised `autoUpdate`: `disabled`/`off` turn checks off; `notify` and unipi's `auto`
/// (no in-extension install in Oino) notify.
pub(crate) fn checks_enabled(mode: &str) -> bool {
    !matches!(mode, "disabled" | "off")
}

/// `checkIntervalMs` (the hub enum stores strings; numbers are accepted too).
pub(crate) fn interval_ms(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|ms| *ms >= 60_000)
    .unwrap_or(3_600_000)
}

/// unipi `getIntervalLabel`.
pub(crate) fn interval_label(ms: u64) -> String {
    match ms {
        1_800_000 => "30min".into(),
        3_600_000 => "1h".into(),
        21_600_000 => "6h".into(),
        86_400_000 => "1d".into(),
        _ => format!("{}min", (ms + 30_000) / 60_000),
    }
}

/// Update-available screen (unipi `renderUpdateOverlay`): versions, changelog since the
/// installed version, and the actions that replace `[Y] Update / [n] Skip`.
pub(crate) fn update_panel(r: &CheckResult, changelog: &str) -> Value {
    json!({
        "panelId": "unipi.updater/update",
        "placement": "overlay",
        "title": "Update available",
        "spec": update_spec(r, changelog),
        "keys": [
            {"key": "y", "actionId": "how", "label": "How to update"},
            {"key": "o", "actionId": "open", "label": "Open release page"},
            {"key": "n", "actionId": "skip", "label": "Skip this version"},
            {"key": "q", "actionId": "$close", "label": "Later"},
        ],
        "focus": true,
        "closeOnEscape": true,
    })
}

fn update_spec(r: &CheckResult, changelog: &str) -> Value {
    let body = if changelog.trim().is_empty() {
        json!({"t": "empty", "message": "No changelog for this release."})
    } else {
        json!({"t": "markdown", "value": changelog})
    };
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": [
        {"t": "text", "value": [
            {"text": "Oino "}, {"text": r.latest_version, "bold": true, "tone": "success"},
            {"text": " is available (installed "}, {"text": r.current_version, "tone": "muted"}, {"text": ")."}
        ]},
        {"t": "kv", "items": [
            {"key": "Installed", "value": r.current_version},
            {"key": "Latest", "value": r.latest_version},
            {"key": "Channel", "value": r.channel},
        ]},
        {"t": "section", "title": "What's new", "body": body},
        {"t": "stack", "direction": "horizontal", "gap": 1, "children": [
            {"t": "button", "label": "How to update", "actionId": "how", "primary": true},
            {"t": "button", "label": "Release page", "actionId": "open"},
            {"t": "button", "label": "Skip this version", "actionId": "skip"},
            {"t": "button", "label": "Later", "actionId": "$close"},
        ]},
    ]})
}

/// Instructions view after "How to update".
pub(crate) fn instructions_spec(text: &str) -> Value {
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": [
        {"t": "markdown", "value": text},
        {"t": "keyHints", "hints": [{"keys": "b", "label": "back"}, {"keys": "q", "label": "close"}]},
    ]})
}

/// README list (unipi `renderReadmeOverlay`).
pub(crate) fn readme_list_spec(readmes: &[Readme]) -> Value {
    if readmes.is_empty() {
        return json!({"t": "empty", "message": "No bundled READMEs found."});
    }
    let items: Vec<Value> = readmes
        .iter()
        .map(|r| json!({"id": r.name, "label": r.name, "detail": r.id, "badge": r.version}))
        .collect();
    json!({"t": "stack", "direction": "vertical", "children": [
        {"t": "list", "items": items, "selectable": "open"},
        {"t": "keyHints", "hints": [{"keys": "enter", "label": "open"}, {"keys": "q", "label": "close"}]},
    ]})
}

/// One document (README or changelog entry) with a back hint.
pub(crate) fn doc_spec(markdown: &str) -> Value {
    json!({"t": "stack", "direction": "vertical", "children": [
        {"t": "markdown", "value": markdown},
        {"t": "keyHints", "hints": [{"keys": "b", "label": "back"}, {"keys": "q", "label": "close"}]},
    ]})
}

/// Browser panel shell.
pub(crate) fn browser_panel(id: &str, title: &str, spec: &Value) -> Value {
    json!({
        "panelId": format!("{EXTENSION_ID}/{id}"),
        "placement": "overlay",
        "title": title,
        "spec": spec,
        "keys": [{"key": "b", "actionId": "back", "label": "Back"}],
        "focus": true,
        "closeOnEscape": true,
    })
}

/// Info-screen group (unipi `infoRegistry.registerGroup({id: "updater"})`).
pub(crate) fn info_group(
    current: &str,
    last: Option<&CheckResult>,
    checked_at: i64,
    mode: &str,
) -> Value {
    let stats = json!([
        {"id": "current", "label": "Installed", "show": true},
        {"id": "latest", "label": "Latest", "show": true},
        {"id": "status", "label": "Status", "show": true},
        {"id": "lastCheck", "label": "Last check", "show": true},
    ]);
    let data = match last {
        None => json!({
            "current": {"value": current},
            "latest": {"value": "checking…"},
            "status": {"value": "checking"},
            "lastCheck": {"value": "never"},
            "raw": {"value": "", "raw": {"current": current, "latest": null, "available": false, "checkedAt": 0, "mode": mode}},
        }),
        Some(r) => json!({
            "current": {"value": r.current_version},
            "latest": {"value": r.latest_version},
            "status": {"value": if r.update_available { "update available" } else { "up to date" }},
            "lastCheck": {"value": if checked_at > 0 { unipi_kit::fmt::iso_from_ms(checked_at) } else { "never".to_owned() }},
            "raw": {"value": "", "raw": {"current": r.current_version, "latest": r.latest_version, "available": r.update_available, "checkedAt": checked_at, "mode": mode}},
        }),
    };
    json!({"id": "updater", "name": "Updater", "short": "📦", "priority": 20, "config": {"showByDefault": true, "stats": stats}, "data": data})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_unipi() {
        let v = namespace().to_json();
        assert_eq!(v["namespace"], "updater");
        assert_eq!(v["extensionId"], "unipi.updater");
        let f = &v["sections"][0]["fields"];
        assert_eq!(f[0]["key"], "checkIntervalMs");
        assert_eq!(f[0]["options"].as_array().map(Vec::len), Some(4));
        assert_eq!(
            f[0]["options"][1],
            json!({"value": "3600000", "label": "1 hour"})
        );
        assert_eq!(f[1]["key"], "autoUpdate");
        assert_eq!(f[1]["default"], "notify");
        assert_eq!(v["defaults"], defaults());
    }

    #[test]
    fn interval_and_mode() {
        assert_eq!(interval_ms(Some(&json!("21600000"))), 21_600_000);
        assert_eq!(interval_ms(Some(&json!(1_800_000))), 1_800_000);
        assert_eq!(interval_ms(Some(&json!("x"))), 3_600_000);
        assert_eq!(
            interval_ms(Some(&json!(5))),
            3_600_000,
            "floor of one minute"
        );
        assert_eq!(interval_label(3_600_000), "1h");
        assert_eq!(interval_label(86_400_000), "1d");
        assert_eq!(interval_label(600_000), "10min");
        assert!(checks_enabled("notify"));
        assert!(checks_enabled("auto"));
        assert!(!checks_enabled("disabled"));
        assert!(!checks_enabled("off"));
    }

    #[test]
    fn update_panel_shape() {
        let r = CheckResult::new("1.1.0", "1.0.0", "latest", "latest");
        let p = update_panel(&r, "## 1.1.0\n\n### Added\n- x");
        assert_eq!(p["placement"], "overlay");
        assert_eq!(p["keys"][0]["actionId"], "how");
        let s = p["spec"].to_string();
        assert!(s.contains("\"markdown\"") && s.contains("1.1.0"));
        let empty = update_panel(&r, "");
        assert!(empty["spec"].to_string().contains("No changelog"));
    }

    #[test]
    fn info_group_shape() {
        let g = info_group("1.0.0", None, 0, "notify");
        assert_eq!(g["id"], "updater");
        assert_eq!(g["data"]["status"]["value"], "checking");
        let r = CheckResult::new("1.1.0", "1.0.0", "latest", "latest");
        let g = info_group("1.0.0", Some(&r), 1000, "notify");
        assert_eq!(g["data"]["status"]["value"], "update available");
        assert_eq!(g["data"]["raw"]["raw"]["latest"], "1.1.0");
        assert_eq!(g["config"]["stats"].as_array().map(Vec::len), Some(4));
    }
}
