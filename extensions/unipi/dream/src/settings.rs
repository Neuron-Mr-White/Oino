//! `dream` settings (unipi `src/settings.ts`).

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, choice};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.dream";
/// Namespace.
pub const NAMESPACE: &str = "dream";

/// Resolved settings.
#[derive(Clone, Debug, PartialEq)]
pub struct DreamSettings {
    /// Background dreaming on.
    pub enabled: bool,
    /// New sessions required.
    pub min_sessions: f64,
    /// Minimum hours between dreams.
    pub min_gap_hours: f64,
    /// `provider/model` for the dream agent (`""` = session default).
    pub model: String,
    /// Thinking level.
    pub thinking: String,
    /// Where approved skills land (project-relative).
    pub skills_target: String,
    /// Hard cap for one run (minutes).
    pub max_runtime_min: f64,
}

impl Default for DreamSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_sessions: 5.0,
            min_gap_hours: 12.0,
            model: String::new(),
            thinking: "medium".into(),
            skills_target: ".agents/skills".into(),
            max_runtime_min: 40.0,
        }
    }
}

/// Defaults JSON.
#[must_use]
pub fn defaults() -> Value {
    json!({"enabled": false, "minSessions": 5, "minGapHours": 12, "model": "", "thinking": "medium", "skillsTarget": ".agents/skills", "maxRuntimeMin": 40})
}

/// unipi `normalizeDream`.
#[must_use]
pub fn normalize(raw: &Value) -> DreamSettings {
    let d = DreamSettings::default();
    let num = |k: &str, dv: f64| {
        raw.get(k)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(dv)
    };
    let s = |k: &str, dv: &str| {
        raw.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(dv)
            .to_owned()
    };
    DreamSettings {
        enabled: raw
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(d.enabled),
        min_sessions: num("minSessions", d.min_sessions).max(1.0),
        min_gap_hours: num("minGapHours", d.min_gap_hours),
        model: raw
            .get("model")
            .and_then(Value::as_str)
            .map_or_else(String::new, |m| m.trim().to_owned()),
        thinking: s("thinking", &d.thinking),
        skills_target: s("skillsTarget", &d.skills_target),
        max_runtime_min: num("maxRuntimeMin", d.max_runtime_min).max(5.0),
    }
}

fn min(v: f64) -> impl FnOnce(&mut Kind) {
    move |k| {
        if let Kind::Number { min, .. } = k {
            *min = Some(v);
        }
    }
}

/// The hub schema.
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Dream", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("dream", "Dream")
                .description("Background retrospection over past sessions: lessons into memory, skills proposed for approval")
                .field(Field::boolean("enabled", "Dream in the background").description("Off by default. On: every time Oino opens, start a background dream when enough new sessions piled up. /unipi:dream run works either way."))
                .field(Field::number("minSessions", "Min new sessions").description("New struggle sessions required since the last dream before another runs.").with_kind(min(1.0)))
                .field(Field::number("minGapHours", "Min gap (hours)").description("Minimum hours between two dreams.").with_kind(min(0.0)))
                .field(Field::string("model", "Dream model").description("Optional provider/model for the dream agent (empty = session default)."))
                .field(
                    Field::enumeration(
                        "thinking",
                        "Thinking level",
                        vec![
                            choice("off", "off", "No thinking."),
                            choice("low", "low", "Brief thinking."),
                            choice("medium", "medium", "Default balance."),
                            choice("high", "high", "Deeper reasoning."),
                        ],
                    )
                    .description("The dream agent's thinking level."),
                )
                .field(Field::string("skillsTarget", "Skills target dir").description("Where approved crafted skills are copied (project-relative)."))
                .field(Field::number("maxRuntimeMin", "Max runtime (min)").description("Hard cap for one dream run.").with_kind(min(5.0))),
        )
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
    fn normalizing() {
        assert_eq!(normalize(&Value::Null), DreamSettings::default());
        let s = normalize(
            &json!({"enabled": true, "minSessions": 0, "maxRuntimeMin": 1, "thinking": " ", "model": " a/b "}),
        );
        assert!(s.enabled);
        assert!((s.min_sessions - 1.0).abs() < f64::EPSILON);
        assert!((s.max_runtime_min - 5.0).abs() < f64::EPSILON);
        assert_eq!(s.thinking, "medium");
        assert_eq!(s.model, "a/b");
        assert_eq!(namespace().to_json()["namespace"], "dream");
    }
}
