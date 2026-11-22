//! Fusion preset store (unipi `preset.ts`): curated lead/sidekick lists, per-model effort,
//! default pair, MRU, badges, price overrides and the active selection.
//!
//! Layers (deep-merged, project on top; arrays replace, objects merge): global
//! `/ext-global/preset.json`, project `.oino/fusion-preset.json`. Runtime state (effort,
//! recent, active, startup) always persists globally.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Schema version.
pub const SCHEMA_VERSION: u32 = 1;
/// pi thinking levels, ascending.
pub const EFFORT_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];
/// unipi `RECENT_LIMIT`.
pub const RECENT_LIMIT: usize = 5;

/// unipi `ActiveSelection`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Active {
    /// One model.
    Single {
        /// `provider/id`.
        model: String,
    },
    /// Lead + sidekick.
    #[serde(rename_all = "camelCase")]
    Fusion {
        /// Lead.
        lead: String,
        /// Sidekick.
        sidekick: String,
        /// Lead effort.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lead_effort: Option<String>,
        /// Sidekick effort.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sidekick_effort: Option<String>,
    },
}

/// Price per 1M tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    /// Input.
    pub input: f64,
    /// Cached input.
    pub cached_input: f64,
    /// Output.
    pub output: f64,
}

/// Default pair.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    /// Lead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    /// Sidekick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidekick: Option<String>,
}

/// unipi `FusionPreset`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// Schema.
    pub schema_version: u32,
    /// Leads.
    pub lead: Vec<String>,
    /// Sidekicks.
    pub sidekick: Vec<String>,
    /// Default pair.
    pub default: Pair,
    /// Per-model effort.
    pub effort: BTreeMap<String, String>,
    /// MRU.
    pub recent: Vec<String>,
    /// Badges (`new|promotion|beta`).
    pub badges: BTreeMap<String, String>,
    /// Price overrides.
    pub prices: BTreeMap<String, Price>,
    /// Active selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<Active>,
}

/// unipi `isEffortLevel`.
#[must_use]
pub fn is_effort(v: &str) -> bool {
    EFFORT_LEVELS.contains(&v)
}

fn string_array(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
        if s.contains('/') && !out.iter().any(|o| o == s) {
            out.push(s.to_owned());
        }
    }
    out
}

/// A validated partial layer (unipi `parsePreset`): `None` fields are absent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    lead: Option<Vec<String>>,
    sidekick: Option<Vec<String>>,
    default: Option<Pair>,
    effort: Option<BTreeMap<String, String>>,
    recent: Option<Vec<String>>,
    badges: Option<BTreeMap<String, String>>,
    prices: Option<BTreeMap<String, Price>>,
    active: Option<Active>,
}

/// unipi `parsePreset`.
#[must_use]
pub fn parse(raw: &Value) -> Layer {
    let Some(r) = raw.as_object() else {
        return Layer::default();
    };
    let obj = |k: &str| r.get(k).and_then(Value::as_object);
    let active = obj("active").and_then(|a| match a.get("kind").and_then(Value::as_str) {
        Some("single") => a
            .get("model")
            .and_then(Value::as_str)
            .map(|m| Active::Single { model: m.into() }),
        Some("fusion") => Some(Active::Fusion {
            lead: a.get("lead")?.as_str()?.into(),
            sidekick: a.get("sidekick")?.as_str()?.into(),
            lead_effort: a
                .get("leadEffort")
                .and_then(Value::as_str)
                .filter(|e| is_effort(e))
                .map(Into::into),
            sidekick_effort: a
                .get("sidekickEffort")
                .and_then(Value::as_str)
                .filter(|e| is_effort(e))
                .map(Into::into),
        }),
        _ => None,
    });
    Layer {
        lead: r.get("lead").map(string_array),
        sidekick: r.get("sidekick").map(string_array),
        default: obj("default").map(|d| Pair {
            lead: d.get("lead").and_then(Value::as_str).map(Into::into),
            sidekick: d.get("sidekick").and_then(Value::as_str).map(Into::into),
        }),
        effort: obj("effort").map(|e| {
            e.iter()
                .filter_map(|(k, v)| {
                    v.as_str()
                        .filter(|v| is_effort(v))
                        .map(|v| (k.clone(), v.to_owned()))
                })
                .collect()
        }),
        recent: r
            .get("recent")
            .map(|v| string_array(v).into_iter().take(RECENT_LIMIT).collect()),
        badges: obj("badges").map(|b| {
            b.iter()
                .filter_map(|(k, v)| {
                    v.as_str()
                        .filter(|v| matches!(*v, "new" | "promotion" | "beta"))
                        .map(|v| (k.clone(), v.to_owned()))
                })
                .collect()
        }),
        prices: obj("prices").map(|p| {
            p.iter()
                .filter_map(|(k, v)| {
                    let n = |f: &str| {
                        v.get(f)
                            .and_then(Value::as_f64)
                            .filter(|x| x.is_finite() && *x >= 0.0)
                    };
                    Some((
                        k.clone(),
                        Price {
                            input: n("input")?,
                            cached_input: n("cachedInput")?,
                            output: n("output")?,
                        },
                    ))
                })
                .collect()
        }),
        active,
    }
}

/// unipi `mergePresets`.
#[must_use]
pub fn merge(base: Preset, over: Layer) -> Preset {
    let mut effort = base.effort;
    effort.extend(over.effort.unwrap_or_default());
    let mut badges = base.badges;
    badges.extend(over.badges.unwrap_or_default());
    let mut prices = base.prices;
    prices.extend(over.prices.unwrap_or_default());
    let d = over.default.unwrap_or_default();
    Preset {
        schema_version: SCHEMA_VERSION,
        lead: over.lead.unwrap_or(base.lead),
        sidekick: over.sidekick.unwrap_or(base.sidekick),
        default: Pair {
            lead: d.lead.or(base.default.lead),
            sidekick: d.sidekick.or(base.default.sidekick),
        },
        effort,
        recent: over.recent.unwrap_or(base.recent),
        badges,
        prices,
        active: over.active.or(base.active),
    }
}

/// unipi `pushRecent`.
#[must_use]
pub fn push_recent(recent: &[String], key: &str) -> Vec<String> {
    std::iter::once(key.to_owned())
        .chain(recent.iter().filter(|k| *k != key).cloned())
        .take(RECENT_LIMIT)
        .collect()
}

/// unipi `stepEffort`.
#[must_use]
pub fn step_effort(current: &str, delta: i32) -> String {
    let i = EFFORT_LEVELS
        .iter()
        .position(|e| *e == current)
        .unwrap_or(3);
    let next = i32::try_from(i)
        .unwrap_or(3)
        .saturating_add(delta)
        .clamp(0, 6);
    EFFORT_LEVELS[usize::try_from(next).unwrap_or(3)].to_owned()
}

/// unipi `effortLabel`.
#[must_use]
pub fn effort_label(level: &str) -> String {
    match level {
        "off" => "None".into(),
        "xhigh" => "XHigh".into(),
        "max" => "Max".into(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    }
}

/// unipi `splitModelKey`.
#[must_use]
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    let i = key.find('/')?;
    (i > 0 && i + 1 < key.len()).then(|| (&key[..i], &key[i + 1..]))
}

/// The file JSON of `saveCuration` (existing layer + new lists/default).
#[must_use]
pub fn with_curation(
    existing: &Value,
    lead: &[String],
    sidekick: &[String],
    default: &Pair,
) -> Value {
    let mut out = existing.as_object().cloned().unwrap_or_else(Map::new);
    out.insert("schema_version".into(), json!(SCHEMA_VERSION));
    out.insert("lead".into(), json!(lead));
    out.insert("sidekick".into(), json!(sidekick));
    out.insert(
        "default".into(),
        serde_json::to_value(default).unwrap_or_default(),
    );
    Value::Object(out)
}

/// The file JSON of `saveRuntimeState`.
#[must_use]
pub fn with_runtime(
    existing: &Value,
    effort: &BTreeMap<String, String>,
    recent: &[String],
    active: Option<&Active>,
) -> Value {
    let mut out = existing.as_object().cloned().unwrap_or_else(Map::new);
    out.insert("schema_version".into(), json!(SCHEMA_VERSION));
    for k in ["lead", "sidekick"] {
        out.entry(k).or_insert_with(|| json!([]));
    }
    out.entry("default").or_insert_with(|| json!({}));
    out.insert("effort".into(), json!(effort));
    out.insert(
        "recent".into(),
        json!(recent.iter().take(RECENT_LIMIT).collect::<Vec<_>>()),
    );
    match active {
        Some(a) => out.insert("active".into(), serde_json::to_value(a).unwrap_or_default()),
        None => out.remove("active"),
    };
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::many_single_char_names, reason = "short test bindings")]
    fn parse_merge_and_helpers() {
        let g = parse(
            &json!({"lead": ["a/x", "bad", "a/x"], "effort": {"a/x": "high", "b/y": "nope"}, "recent": ["a/1","a/2","a/3","a/4","a/5","a/6"], "prices": {"a/x": {"input": 1, "cachedInput": 0.1, "output": 2}, "b/y": {"input": -1}}, "active": {"kind": "fusion", "lead": "a/x", "sidekick": "b/y", "leadEffort": "max"}}),
        );
        let p = merge(Preset::default(), g);
        assert_eq!(p.lead, ["a/x"]);
        assert_eq!(p.effort.len(), 1);
        assert_eq!(p.recent.len(), 5);
        assert_eq!(p.prices.len(), 1);
        assert!(
            matches!(&p.active, Some(Active::Fusion { lead_effort: Some(e), .. }) if e == "max")
        );
        let proj = parse(&json!({"sidekick": ["b/y"], "default": {"lead": "a/x"}}));
        let m = merge(p, proj);
        assert_eq!((m.lead.len(), m.sidekick.len()), (1, 1));
        assert_eq!(m.default.lead.as_deref(), Some("a/x"));
        assert_eq!(push_recent(&["a".into(), "b".into()], "b"), ["b", "a"]);
        assert_eq!(step_effort("max", 1), "max");
        assert_eq!(step_effort("off", -1), "off");
        assert_eq!(step_effort("medium", 1), "high");
        assert_eq!(effort_label("off"), "None");
        assert_eq!(effort_label("xhigh"), "XHigh");
        assert_eq!(effort_label("medium"), "Medium");
        assert_eq!(split_key("openrouter/a/b"), Some(("openrouter", "a/b")));
        assert_eq!(split_key("/x"), None);
        let a = Active::Single {
            model: "a/x".into(),
        };
        let rt = with_runtime(
            &json!({"lead": ["k/l"]}),
            &BTreeMap::new(),
            &["z/z".into()],
            Some(&a),
        );
        assert_eq!(rt["active"], json!({"kind": "single", "model": "a/x"}));
        assert_eq!(rt["lead"], json!(["k/l"]));
        let c = with_curation(&rt, &["n/n".into()], &[], &Pair::default());
        assert_eq!(c["lead"], json!(["n/n"]));
        assert_eq!(c["recent"], json!(["z/z"]));
    }
}
