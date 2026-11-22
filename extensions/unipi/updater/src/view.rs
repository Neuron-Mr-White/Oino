//! Panels (unipi `src/tui/*-overlay.ts`, ui-protocol v2 §6) and the settings namespace.

use oino_extension::ui::{self, widgets::prelude::*};
use oino_extension::ui::{KeyBinding, Placement};
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
pub(crate) fn update_panel(r: &CheckResult, changelog: &str) -> ui::Panel {
    ui::Panel::new(PANEL_UPDATE)
        .placement(Placement::Overlay)
        .title("Update available")
        .spec(update_spec(r, changelog))
        .keys([
            key("y", "how", "How to update"),
            key("o", "open", "Open release page"),
            key("n", "skip", "Skip this version"),
            key("q", "close", "Later"),
        ])
        .focus(true)
        .close_on_escape(true)
}

/// One entry key binding.
fn key(key: &str, action: &str, label: &str) -> KeyBinding {
    KeyBinding {
        key: key.to_owned(),
        action: Some(action.to_owned()),
        label: label.to_owned(),
    }
}

/// A section header: a top rule with the title (`section` in the §12.1 mapping).
fn section(title: &str, body: Node) -> Node {
    Block::default()
        .borders(Borders::Sides(vec![BorderSide::TOP]))
        .border_style(Style::new().fg(Color::BorderMuted))
        .title(title)
        .child(body)
        .into()
}

/// Key/value rows (`kv` → 2-column `Table`, §12.1).
fn kv(items: &[(&str, &str)]) -> Node {
    let rows = items.iter().map(|(key, value)| {
        Row::new([
            Cell::new(Line::from(Span::from_string(*key).fg(Color::Muted))),
            Cell::new(*value),
        ])
    });
    Table::new(rows, [Constraint::Max(24), Constraint::Fill(1)]).into()
}

pub(crate) fn update_spec(r: &CheckResult, changelog: &str) -> Node {
    let body: Node = if changelog.trim().is_empty() {
        Empty::new()
            .message("No changelog for this release.")
            .into()
    } else {
        Markdown::new(changelog).into()
    };
    Layout::default()
        .spacing(1)
        .children([
            Paragraph::new(Line::from(vec![
                Span::from("Oino "),
                Span::from_string(r.latest_version.clone())
                    .bold()
                    .fg(Color::Success),
                Span::from(" is available (installed "),
                Span::from_string(r.current_version.clone()).fg(Color::Muted),
                Span::from(")."),
            ]))
            .into(),
            kv(&[
                ("Installed", &r.current_version),
                ("Latest", &r.latest_version),
                ("Channel", &r.channel),
            ]),
            section("What's new", body),
            Layout::horizontal([
                Constraint::Min(0),
                Constraint::Min(0),
                Constraint::Min(0),
                Constraint::Min(0),
            ])
            .spacing(1)
            .children([
                Button::new("How to update", "how")
                    .variant(ButtonVariant::Primary)
                    .into(),
                Button::new("Release page", "open").into(),
                Button::new("Skip this version", "skip").into(),
                Button::new("Later", "close").into(),
            ])
            .into(),
        ])
        .into()
}

/// Instructions view after "How to update".
pub(crate) fn instructions_spec(text: &str) -> Node {
    Layout::default()
        .spacing(1)
        .children([
            Markdown::new(text).into(),
            KeyHints::new([("b", "back"), ("q", "close")]).into(),
        ])
        .into()
}

/// README list (unipi `renderReadmeOverlay`).
pub(crate) fn readme_list_spec(readmes: &[Readme]) -> Node {
    if readmes.is_empty() {
        return Empty::new().message("No bundled READMEs found.").into();
    }
    let items = readmes.iter().map(|r| {
        ListItem::new(r.name.clone())
            .key(r.name.clone())
            .detail(Line::from(r.id.clone()))
            .badge(Line::from(r.version.clone()))
    });
    Layout::default()
        .children([
            List::new(items).on_activate("open").into(),
            KeyHints::new([("enter", "open"), ("q", "close")]).into(),
        ])
        .into()
}

/// One document (README or changelog entry) with a back hint.
pub(crate) fn doc_spec(markdown: &str) -> Node {
    Layout::default()
        .children([
            Markdown::new(markdown).into(),
            KeyHints::new([("b", "back"), ("q", "close")]).into(),
        ])
        .into()
}

/// Changelog version list (unipi `renderChangelogOverlay`; the `/changelog` hidden runner).
pub(crate) fn changelog_list_spec(entries: &[crate::changelog::Entry]) -> Node {
    if entries.is_empty() {
        return Empty::new()
            .message("No changelog found yet — check from Settings → About once online.")
            .into();
    }
    let items = entries.iter().enumerate().map(|(i, e)| {
        let count: usize = e.sections.iter().map(|(_, items)| items.len()).sum();
        let sections: Vec<&str> = e.sections.iter().map(|(s, _)| s.as_str()).collect();
        ListItem::new(e.version.clone())
            .key(i.to_string())
            .detail(Line::from(if e.date.is_empty() {
                sections.join(", ")
            } else {
                format!("{} · {}", e.date, sections.join(", "))
            }))
            .badge(Line::from(count.to_string()))
    });
    Layout::default()
        .children([
            List::new(items).on_activate("open").into(),
            KeyHints::new([("enter", "open"), ("q", "close")]).into(),
        ])
        .into()
}

/// Panel id of the update screen.
pub(crate) const PANEL_UPDATE: &str = "unipi.updater/update";

/// Browser panel shell.
pub(crate) fn browser_panel(id: &str, title: &str, spec: Node) -> ui::Panel {
    ui::Panel::new(format!("{EXTENSION_ID}/{id}"))
        .placement(Placement::Overlay)
        .title(title)
        .spec(spec)
        .keys([key("b", "back", "Back")])
        .focus(true)
        .close_on_escape(true)
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
        let s = serde_json::to_string(&update_spec(&r, "## 1.1.0\n\n### Added\n- x"))
            .unwrap_or_default();
        assert!(s.contains("\"Markdown\"") && s.contains("1.1.0"));
        assert!(s.contains("\"action\":\"how\""));
        let empty = serde_json::to_string(&update_spec(&r, "")).unwrap_or_default();
        assert!(empty.contains("No changelog"));
    }

    /// GA-19: the hidden `/changelog` runner's version list.
    #[test]
    fn changelog_list_shape() {
        let e = crate::changelog::Entry {
            version: "1.1.0".into(),
            date: "2026-10-27".into(),
            sections: vec![("Added".into(), vec!["x".into()])],
            body: "## 1.1.0".into(),
        };
        let spec = serde_json::to_string(&changelog_list_spec(&[e])).unwrap_or_default();
        assert!(spec.contains("\"onActivate\":\"open\"") && spec.contains("1.1.0"));
        let empty: Value = serde_json::to_value(changelog_list_spec(&[])).unwrap_or_default();
        assert!(
            empty["message"]
                .as_str()
                .is_some_and(|m| m.contains("No changelog"))
        );
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
