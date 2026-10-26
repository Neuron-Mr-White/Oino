//! Namespace `info-screen` (unipi `config.ts`, `types.ts`).

use serde_json::{Map, Value, json};
use unipi_kit::settings::{Choice, Field, Kind, Namespace, Section, Unit, choice};

use crate::registry::GroupInfo;

/// Namespace.
pub const NAMESPACE: &str = "info-screen";

/// Startup splash behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    /// Shown until a key is pressed.
    On,
    /// Not shown.
    Off,
    /// Shown, then closes after `bootTimeoutMs` without taking keys.
    AutoClose,
}

/// Resolved settings.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoSettings {
    /// Splash mode.
    pub boot_mode: BootMode,
    /// Auto-close time.
    pub boot_timeout_ms: u64,
    /// Raw `groups` object (`{<id>: {show, stats: {<stat>: bool}}}`).
    pub groups: Map<String, Value>,
    /// Tab order override.
    pub group_order: Vec<String>,
}

/// Defaults.
#[must_use]
pub fn defaults() -> Value {
    json!({"bootMode": "auto-close", "bootTimeoutMs": 2500, "groups": {}, "groupOrder": []})
}

impl InfoSettings {
    /// From the namespace object; migrates legacy `showOnBoot` (`true` → on, `false` → off).
    #[must_use]
    pub fn from_json(v: &Value) -> Self {
        let boot_mode = match v.get("bootMode").and_then(Value::as_str) {
            Some("on") => BootMode::On,
            Some("off") => BootMode::Off,
            Some("auto-close") => BootMode::AutoClose,
            _ => match v.get("showOnBoot").and_then(Value::as_bool) {
                Some(true) => BootMode::On,
                Some(false) => BootMode::Off,
                None => BootMode::AutoClose,
            },
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "ms setting"
        )]
        let timeout = v
            .get("bootTimeoutMs")
            .and_then(Value::as_f64)
            .map_or(2500, |t| t.max(0.0) as u64);
        Self {
            boot_mode,
            boot_timeout_ms: timeout,
            groups: v
                .get("groups")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            group_order: v
                .get("groupOrder")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// `groups.<id>.show` (default on).
    #[must_use]
    pub fn group_shown(&self, id: &str) -> bool {
        self.groups
            .get(id)
            .and_then(|g| g.get("show"))
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    /// `groups.<id>.stats.<stat>` (default on).
    #[must_use]
    pub fn stat_enabled(&self, id: &str, stat: &str) -> bool {
        self.groups
            .get(id)
            .and_then(|g| g.get("stats"))
            .and_then(|s| s.get(stat))
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }
}

/// Schema; the "Pages & stats" page and "Page order" items come from the live groups.
#[must_use]
pub fn namespace(groups: &[GroupInfo]) -> Namespace {
    let pages: Vec<Section> = groups
        .iter()
        .map(|g| {
            Section::new(&g.id, &g.name)
                .field(
                    Field::boolean(
                        &format!("groups.{}.show", g.id),
                        &format!("Show {}", g.name),
                    )
                    .description(format!("Show the {} group on the dashboard.", g.name))
                    .default_value(true),
                )
                .fields(g.stats.iter().map(|s| {
                    Field::boolean(&format!("groups.{}.stats.{}", g.id, s.id), &s.label)
                        .description(format!(
                            "Show the {} stat in the {} group.",
                            s.label, g.name
                        ))
                        .default_value(true)
                }))
        })
        .collect();
    let order: Vec<Choice> = groups
        .iter()
        .map(|g| Choice {
            value: g.id.clone(),
            label: g.name.clone(),
            description: None,
        })
        .collect();
    Namespace::new(NAMESPACE, "Info Screen", crate::EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("startup", "Startup")
                .field(
                    Field::enumeration(
                        "bootMode",
                        "Unicrab splash",
                        vec![
                            choice("auto-close", "auto-close", "show it, then fade out; never takes your keys"),
                            choice("on", "on", "keep it up until you press a key"),
                            choice("off", "off", "no splash"),
                        ],
                    )
                    .description("The Unicrab card shown at startup. /unipi:info opens the dashboard any time."),
                )
                .field(
                    Field::number("bootTimeoutMs", "Splash time")
                        .description("How long the splash stays up in auto-close mode.")
                        .with_kind(|k| {
                            if let Kind::Number { min, unit, .. } = k {
                                *min = Some(500.0);
                                *unit = Some(Unit::Ms);
                            }
                        }),
                ),
        )
        .section(
            Section::new("pages", "Pages")
                .field(Field::page("groups-page", "Pages & stats…", pages).description("Choose which pages appear on the dashboard."))
                .field(Field::order("groupOrder", "Page order", order).description("Tab order of the dashboard pages.")),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::StatInfo;

    #[test]
    fn boot_mode_and_legacy_migration() {
        assert_eq!(
            InfoSettings::from_json(&defaults()).boot_mode,
            BootMode::AutoClose
        );
        assert_eq!(
            InfoSettings::from_json(&json!({"showOnBoot": true})).boot_mode,
            BootMode::On
        );
        assert_eq!(
            InfoSettings::from_json(&json!({"showOnBoot": false})).boot_mode,
            BootMode::Off
        );
        assert_eq!(
            InfoSettings::from_json(&json!({"bootMode": "on", "showOnBoot": false})).boot_mode,
            BootMode::On
        );
        assert_eq!(InfoSettings::from_json(&json!({})).boot_timeout_ms, 2500);
    }

    #[test]
    fn visibility() {
        let s = InfoSettings::from_json(
            &json!({"groups": {"usage": {"show": false, "stats": {"costToday": false}}}}),
        );
        assert!(!s.group_shown("usage"));
        assert!(s.group_shown("tools"));
        assert!(!s.stat_enabled("usage", "costToday"));
        assert!(s.stat_enabled("usage", "sessions"));
    }

    #[test]
    fn dynamic_page_and_order() {
        let g = vec![GroupInfo {
            id: "usage".into(),
            name: "Usage history".into(),
            stats: vec![StatInfo {
                id: "costToday".into(),
                label: "Cost today".into(),
                show: true,
            }],
        }];
        let v = namespace(&g).to_json();
        let page = &v["sections"][1]["fields"][0];
        assert_eq!(page["type"], "page");
        assert_eq!(page["sections"][0]["fields"][0]["key"], "groups.usage.show");
        assert_eq!(page["sections"][0]["fields"][1]["label"], "Cost today");
        assert_eq!(v["sections"][1]["fields"][1]["items"][0]["value"], "usage");
        assert_eq!(v["sections"][0]["fields"][1]["unit"], "ms");
    }
}
