//! `skills` settings namespace (unipi `src/settings.ts`).
//!
//! Per-skill states edited in the skill editor are stored by the extension in
//! `/ext-global/states.json` (global) and `/ext-workspace/<wsid>/states.json` (project) and
//! layered over `extensionSettings.skills.states` (no guest settings write path, deviations).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use unipi_kit::jev;
use unipi_kit::settings::{Field, Kind, Namespace, Section, Values, choice};

use crate::registry::{SkillState, normalize_states};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.skill-registry";
/// Namespace.
pub const NAMESPACE: &str = "skills";
/// Editor runner.
pub const EDITOR_COMMAND: &str = "unipi:skills-editor";

/// Exposure mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExposureMode {
    /// jev picks the listed set.
    #[default]
    Judged,
    /// List everything.
    All,
    /// Strip bundled skills.
    Off,
}

/// `skills.exposure`.
#[derive(Clone, Debug, PartialEq)]
pub struct Exposure {
    /// Mode.
    pub mode: ExposureMode,
    /// Minimum jev relevance.
    pub threshold: f64,
    /// Listed cap.
    pub max_skills: usize,
    /// Announce newly relevant hidden skills.
    pub recheck: bool,
}

impl Default for Exposure {
    fn default() -> Self {
        Self {
            mode: ExposureMode::Judged,
            threshold: 0.8,
            max_skills: 12,
            recheck: true,
        }
    }
}

/// unipi `normalizeExposure`.
#[must_use]
pub fn normalize_exposure(raw: &Value) -> Exposure {
    let d = Exposure::default();
    Exposure {
        mode: match raw.get("mode").and_then(Value::as_str) {
            Some("all") => ExposureMode::All,
            Some("off") => ExposureMode::Off,
            Some("judged") => ExposureMode::Judged,
            _ => d.mode,
        },
        threshold: raw
            .get("threshold")
            .and_then(Value::as_f64)
            .filter(|t| (0.0..=1.0).contains(t))
            .unwrap_or(d.threshold),
        max_skills: raw
            .get("maxSkills")
            .and_then(Value::as_f64)
            .filter(|m| *m >= 1.0)
            .map_or(d.max_skills, |m| {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "floored, ≥ 1"
                )]
                let n = m.floor().min(1e6) as usize;
                n
            }),
        recheck: raw
            .get("recheck")
            .and_then(Value::as_bool)
            .unwrap_or(d.recheck),
    }
}

/// Resolved `skills` settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Skills {
    /// Apply per-skill states and include the vault.
    pub proxy: bool,
    /// Merged states (settings ⊕ stored global ⊕ stored project).
    pub states: BTreeMap<String, SkillState>,
    /// Exposure.
    pub exposure: Exposure,
}

/// Defaults.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "proxy": false,
        "states": {},
        "exposure": {"mode": "judged", "threshold": 0.8, "maxSkills": 12, "recheck": true},
        "decisionModel": jev::consumer_defaults(),
    })
}

/// The hub schema.
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Skills", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("registry", "Skill registry")
                .description("Turn skills on or off per project, and keep a vault of extra skills")
                .field(Field::boolean("proxy", "Skill proxy").description(
                    "Apply the per-skill choices below and include ~/.oino/skill-vault. Off leaves the discovered skills as-is and hides the vault.",
                ))
                .field(
                    Field::action("states", "Skill settings…", EDITOR_COMMAND).description(
                        "Per skill Enabled / Must show, per global or project scope.",
                    ),
                ),
        )
        .section(
            Section::new("exposure", "Exposure")
                .description("Which skills are listed in the system prompt")
                .field(
                    Field::enumeration(
                        "exposure.mode",
                        "Skill exposure",
                        vec![
                            choice(
                                "judged",
                                "judged",
                                "jev picks the skills listed for the session",
                            ),
                            choice("all", "all", "list everything, no judging"),
                            choice(
                                "off",
                                "off",
                                "strip bundled skills from the system prompt",
                            ),
                        ],
                    )
                    .description("How the session's skill list is picked."),
                )
                .field(
                    Field::number("exposure.threshold", "Relevance threshold")
                        .description("Minimum jev relevance for a skill to stay listed.")
                        .with_kind(|k| {
                            if let Kind::Number { min, max, .. } = k {
                                *min = Some(0.0);
                                *max = Some(1.0);
                            }
                        }),
                )
                .field(
                    Field::number("exposure.maxSkills", "Max skills listed")
                        .description("At or under this size the catalog is never judged.")
                        .with_kind(|k| {
                            if let Kind::Number { min, .. } = k {
                                *min = Some(1.0);
                            }
                        }),
                )
                .field(
                    Field::boolean("exposure.recheck", "Announce new skills").description(
                        "Tell the agent when a hidden skill becomes relevant on a later prompt.",
                    ),
                ),
        )
        .section(jev::consumer_section(Some("Skills — Decision model"), false))
}

/// Stored per-skill state layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Every project.
    Global,
    /// This project.
    Project,
}

fn layer_file(l: Layer) -> PathBuf {
    match l {
        Layer::Global => unipi_kit::paths::global("states.json"),
        Layer::Project => unipi_kit::paths::workspace("states.json"),
    }
}

/// Reads a stored layer.
#[must_use]
pub fn read_layer(l: Layer) -> BTreeMap<String, SkillState> {
    unipi_kit::paths::read_json::<Value>(&layer_file(l))
        .map(|v| normalize_states(&v))
        .unwrap_or_default()
}

/// Writes a stored layer.
pub fn write_layer(l: Layer, states: &BTreeMap<String, SkillState>) {
    let _ = unipi_kit::paths::write_json(&layer_file(l), states);
}

/// Stored proxy override (`/ext-global/proxy.json`), set from the editor.
#[must_use]
pub fn stored_proxy() -> Option<bool> {
    unipi_kit::paths::read_json::<Value>(&unipi_kit::paths::global("proxy.json"))
        .and_then(|v| v.get("proxy").and_then(Value::as_bool))
}

/// Stores the proxy override.
pub fn set_stored_proxy(on: bool) {
    let _ = unipi_kit::paths::write_json(
        &unipi_kit::paths::global("proxy.json"),
        &json!({"proxy": on}),
    );
}

/// Merges per option: project over global over settings.
#[must_use]
pub fn merge_states(
    base: &BTreeMap<String, SkillState>,
    global: &BTreeMap<String, SkillState>,
    project: &BTreeMap<String, SkillState>,
) -> BTreeMap<String, SkillState> {
    let mut out = base.clone();
    for layer in [global, project] {
        for (name, s) in layer {
            let e = out.entry(name.clone()).or_default();
            if s.enabled.is_some() {
                e.enabled = s.enabled;
            }
            if s.must_show.is_some() {
                e.must_show = s.must_show;
            }
        }
    }
    out
}

/// Resolves the namespace values plus the stored layers. Legacy `utility.skills` (in the
/// same settings object) migrates into `exposure` on read.
#[must_use]
pub fn resolve(v: &Values, utility: Option<&Value>, stored: Option<bool>) -> Skills {
    let mut exposure_raw = v.get("exposure").cloned().unwrap_or(Value::Null);
    if let Some(old) = utility
        .and_then(|u| u.get("skills"))
        .filter(|o| o.is_object())
    {
        let mut legacy = serde_json::Map::new();
        match old.get("mode").and_then(Value::as_str) {
            Some(m @ ("judged" | "all" | "off")) => {
                legacy.insert("mode".into(), json!(m));
            }
            _ if old.get("discovery") == Some(&Value::Bool(false)) => {
                legacy.insert("mode".into(), json!("off"));
            }
            _ => {}
        }
        for k in ["threshold", "maxSkills", "recheck"] {
            if let Some(x) = old.get(k) {
                legacy.insert(k.into(), x.clone());
            }
        }
        exposure_raw = unipi_kit::settings::deep_merge(&Value::Object(legacy), &exposure_raw);
    }
    Skills {
        proxy: stored.unwrap_or_else(|| v.bool("proxy", false)),
        states: merge_states(
            &normalize_states(v.get("states").unwrap_or(&Value::Null)),
            &read_layer(Layer::Global),
            &read_layer(Layer::Project),
        ),
        exposure: normalize_exposure(&exposure_raw),
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn exposure_normalization() {
        assert_eq!(
            normalize_exposure(&json!({"mode": "bogus", "threshold": 2})),
            Exposure::default()
        );
        assert_eq!(
            normalize_exposure(&json!({"mode": "off", "maxSkills": 8.7})).max_skills,
            8
        );
    }

    #[test]
    fn layer_merge() {
        let st = |e: Option<bool>, m: Option<bool>| SkillState {
            enabled: e,
            must_show: m,
        };
        let base = BTreeMap::from([("a".to_owned(), st(Some(false), None))]);
        let global = BTreeMap::from([("a".to_owned(), st(None, Some(true)))]);
        let project = BTreeMap::from([("a".to_owned(), st(Some(true), None))]);
        assert_eq!(
            merge_states(&base, &global, &project)["a"],
            st(Some(true), Some(true))
        );
    }

    #[test]
    fn schema() {
        let ns = namespace().to_json();
        assert_eq!(ns["namespace"], "skills");
        assert!(ns.to_string().contains(EDITOR_COMMAND));
    }
}
