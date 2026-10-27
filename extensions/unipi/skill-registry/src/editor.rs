//! The "Skill settings…" editor (unipi `src/editor.ts`) as an OUS overlay panel.
//!
//! Rows: skill, source tag, `E` (enabled) and `M` (must show) toggles showing the effective
//! value (project over global over the source default; inherited values dim), an `inherit`
//! button clearing the edited layer's cell, a filter line, scope and proxy controls, and
//! Save / Cancel. Changes are held until Save.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::registry::{SkillSource, SkillState, default_enabled, default_must_show};

/// Panel id.
pub const PANEL: &str = "unipi.skill-registry/editor";

/// Column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    /// Enabled.
    Enabled,
    /// Must show.
    MustShow,
}

impl Column {
    fn get(self, s: SkillState) -> Option<bool> {
        match self {
            Self::Enabled => s.enabled,
            Self::MustShow => s.must_show,
        }
    }
    fn set(self, s: &mut SkillState, v: Option<bool>) {
        match self {
            Self::Enabled => s.enabled = v,
            Self::MustShow => s.must_show = v,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::MustShow => "mustShow",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "enabled" => Some(Self::Enabled),
            "mustShow" => Some(Self::MustShow),
            _ => None,
        }
    }
}

/// One listed skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSkill {
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Source.
    pub source: SkillSource,
    /// `false` for an opt-in skill (`default-enabled: false`, shown as `opt-in`, off by
    /// default).
    pub default_enabled: bool,
}

/// Edited scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Global layer.
    Global,
    /// Project layer.
    Project,
}

/// Editor state.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    /// Skills (sorted by name).
    pub skills: Vec<EditorSkill>,
    /// Global layer as edited.
    pub global: BTreeMap<String, SkillState>,
    /// Project layer as edited.
    pub project: BTreeMap<String, SkillState>,
    /// Proxy as edited.
    pub proxy: bool,
    /// Edited scope.
    pub scope: Scope,
    /// Filter text.
    pub filter: String,
}

/// What an action did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Re-render.
    Changed,
    /// Save and close.
    Save,
    /// Close without saving.
    Cancel,
    /// Unknown action.
    Ignored,
}

fn default_for(c: Column, s: &EditorSkill) -> bool {
    match c {
        Column::Enabled => default_enabled(s.source, s.default_enabled),
        Column::MustShow => default_must_show(&s.name, s.source),
    }
}

impl Editor {
    fn layer(&self, scope: Scope) -> &BTreeMap<String, SkillState> {
        match scope {
            Scope::Global => &self.global,
            Scope::Project => &self.project,
        }
    }

    fn layer_mut(&mut self) -> &mut BTreeMap<String, SkillState> {
        match self.scope {
            Scope::Global => &mut self.global,
            Scope::Project => &mut self.project,
        }
    }

    /// Effective value of a cell and whether the edited layer sets it.
    #[must_use]
    pub fn cell(&self, skill: &EditorSkill, c: Column) -> (bool, bool) {
        let own = self
            .layer(self.scope)
            .get(&skill.name)
            .and_then(|s| c.get(*s));
        let global = self.global.get(&skill.name).and_then(|s| c.get(*s));
        let project = self.project.get(&skill.name).and_then(|s| c.get(*s));
        let base = match self.scope {
            Scope::Global => global,
            Scope::Project => project.or(global),
        };
        let mut v = base.unwrap_or_else(|| default_for(c, skill));
        if c == Column::Enabled {
            // Must show implies enabled.
            let ms = self.cell(skill, Column::MustShow).0;
            v = v || ms;
        }
        (v, own.is_some())
    }

    fn skill(&self, name: &str) -> Option<EditorSkill> {
        self.skills.iter().find(|s| s.name == name).cloned()
    }

    /// Applies a panel action (`actionId`, `value`).
    pub fn act(&mut self, action: &str, value: Option<&Value>) -> Outcome {
        match action {
            "save" => return Outcome::Save,
            "cancel" | "$close" => return Outcome::Cancel,
            "filter" => {
                value
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .clone_into(&mut self.filter);
            }
            "scope" => {
                self.scope = if value.and_then(Value::as_str) == Some("global") {
                    Scope::Global
                } else {
                    Scope::Project
                };
            }
            "proxy" => self.proxy = value.and_then(Value::as_bool).unwrap_or(!self.proxy),
            _ => {
                let Some((verb, name)) = action.split_once(':') else {
                    return Outcome::Ignored;
                };
                let Some(skill) = self.skill(name) else {
                    return Outcome::Ignored;
                };
                if verb == "clear" {
                    if let Some(s) = self.layer_mut().get_mut(name) {
                        *s = SkillState::default();
                    }
                    self.layer_mut().retain(|_, s| *s != SkillState::default());
                    return Outcome::Changed;
                }
                let Some(col) = Column::parse(verb) else {
                    return Outcome::Ignored;
                };
                let next = value
                    .and_then(Value::as_bool)
                    .unwrap_or(!self.cell(&skill, col).0);
                let entry = self.layer_mut().entry(name.to_owned()).or_default();
                col.set(entry, Some(next));
            }
        }
        Outcome::Changed
    }

    /// Visible skills (filter on name/description/source).
    #[must_use]
    pub fn visible(&self) -> Vec<&EditorSkill> {
        let f = self.filter.to_lowercase();
        self.skills
            .iter()
            .filter(|s| {
                f.is_empty()
                    || s.name.to_lowercase().contains(&f)
                    || s.description.to_lowercase().contains(&f)
                    || s.source.as_str().contains(&f)
            })
            .collect()
    }

    /// The OUS spec.
    #[must_use]
    pub fn spec(&self) -> Value {
        let scope = match self.scope {
            Scope::Global => "global",
            Scope::Project => "project",
        };
        let mut rows = Vec::new();
        for s in self.visible() {
            let toggle = |c: Column| {
                let (v, own) = self.cell(s, c);
                json!({"t": "toggle", "id": format!("{}:{}", c.key(), s.name), "label": if c == Column::Enabled { "E" } else { "M" }, "value": v, "onChange": format!("{}:{}", c.key(), s.name), "dim": !own})
            };
            let desc: String = s.description.chars().take(80).collect();
            rows.push(json!({"t": "stack", "key": s.name, "direction": "horizontal", "gap": 2, "children": [
                toggle(Column::Enabled),
                toggle(Column::MustShow),
                {"t": "text", "wrap": false, "value": [
                    {"text": s.name.clone(), "bold": true},
                    {"text": format!("  {}{}  ", s.source.as_str(), if s.default_enabled { "" } else { " · opt-in" }), "tone": "muted"},
                    {"text": desc, "dim": true},
                ]},
                {"t": "button", "label": "inherit", "actionId": format!("clear:{}", s.name)},
            ]}));
        }
        if rows.is_empty() {
            rows.push(json!({"t": "empty", "message": "No skills match the filter"}));
        }
        json!({"t": "stack", "direction": "vertical", "gap": 1, "children": [
            {"t": "text", "value": [{"text": "Skill settings", "bold": true}, {"text": format!(" · {} skills · editing {scope} · proxy {}", self.skills.len(), if self.proxy { "on" } else { "off" }), "tone": "muted"}]},
            {"t": "stack", "direction": "horizontal", "gap": 2, "children": [
                {"t": "input", "id": "filter", "value": self.filter, "placeholder": "Search: (type to filter)", "onSubmit": "filter"},
                {"t": "select", "id": "scope", "value": scope, "options": [{"value": "global", "label": "global"}, {"value": "project", "label": "project"}], "onChange": "scope"},
                {"t": "toggle", "id": "proxy", "label": "proxy", "value": self.proxy, "onChange": "proxy"},
            ]},
            {"t": "divider"},
            {"t": "stack", "direction": "vertical", "children": rows},
            {"t": "divider"},
            {"t": "text", "value": "E enabled — the model sees it (judged first when exposure is on); off removes it and blocks /skill:name. opt-in skills are off until enabled here (no proxy needed); /skill:name still runs them", "wrap": true},
            {"t": "text", "value": "M must show — always listed, even when judging would hide it"},
            {"t": "stack", "direction": "horizontal", "gap": 2, "children": [
                {"t": "button", "label": "Save", "actionId": "save", "primary": true},
                {"t": "button", "label": "Cancel", "actionId": "cancel"},
            ]},
        ]})
    }

    /// The `openPanel` request.
    #[must_use]
    pub fn open_request(&self) -> Value {
        json!({"panelId": PANEL, "placement": "overlay", "title": "Skill settings", "focus": true, "closeOnEscape": true,
            "keys": [{"key": "enter", "actionId": "save", "label": "save"}, {"key": "esc", "actionId": "cancel", "label": "cancel"}],
            "spec": self.spec()})
    }
}

/// Number of changed cells between two layer sets.
#[must_use]
pub fn count_changes(
    before: (&BTreeMap<String, SkillState>, &BTreeMap<String, SkillState>),
    after: (&BTreeMap<String, SkillState>, &BTreeMap<String, SkillState>),
) -> usize {
    let mut n = 0;
    for (b, a) in [(before.0, after.0), (before.1, after.1)] {
        let names: std::collections::BTreeSet<&String> = b.keys().chain(a.keys()).collect();
        for name in names {
            for c in [Column::Enabled, Column::MustShow] {
                if b.get(name).and_then(|s| c.get(*s)) != a.get(name).and_then(|s| c.get(*s)) {
                    n += 1;
                }
            }
        }
    }
    n
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

    fn editor() -> Editor {
        Editor {
            skills: vec![
                EditorSkill {
                    name: "aws-deploy".into(),
                    description: "Deploy to AWS".into(),
                    source: SkillSource::Vault,
                    default_enabled: true,
                },
                EditorSkill {
                    name: "sql-review".into(),
                    description: "Review SQL".into(),
                    source: SkillSource::User,
                    default_enabled: true,
                },
                EditorSkill {
                    name: "unipi-migrate".into(),
                    description: "Migrate".into(),
                    source: SkillSource::Unipi,
                    default_enabled: false,
                },
            ],
            global: BTreeMap::new(),
            project: BTreeMap::new(),
            proxy: true,
            scope: Scope::Project,
            filter: String::new(),
        }
    }

    #[test]
    fn cells_and_actions() {
        let mut ed = editor();
        let aws = ed.skills[0].clone();
        assert_eq!(ed.cell(&aws, Column::Enabled), (false, false));
        assert_eq!(ed.act("enabled:aws-deploy", None), Outcome::Changed);
        assert_eq!(ed.cell(&aws, Column::Enabled), (true, true));
        ed.act("scope", Some(&json!("global")));
        assert_eq!(ed.cell(&aws, Column::Enabled), (false, false));
        ed.act("mustShow:aws-deploy", Some(&json!(true)));
        assert!(
            ed.cell(&aws, Column::Enabled).0,
            "must show implies enabled"
        );
        ed.act("clear:aws-deploy", None);
        assert!(ed.global.is_empty());
        ed.act("filter", Some(&json!("sql")));
        assert_eq!(ed.visible().len(), 1);
        assert_eq!(ed.act("save", None), Outcome::Save);
        assert_eq!(ed.act("$close", None), Outcome::Cancel);
        assert_eq!(ed.act("bogus", None), Outcome::Ignored);
        let migrate = ed.skills[2].clone();
        assert_eq!(ed.cell(&migrate, Column::Enabled), (false, false));
        ed.act("enabled:unipi-migrate", None);
        assert_eq!(ed.cell(&migrate, Column::Enabled), (true, true));
        ed.act("filter", Some(&json!("")));
        let spec = ed.spec().to_string();
        assert!(spec.contains("unipi · opt-in"));
        ed.act("filter", Some(&json!("sql")));
        let spec = ed.spec().to_string();
        assert!(spec.contains("editing global"));
        assert!(spec.contains("sql-review"));
    }

    #[test]
    fn changes() {
        let st = |e: Option<bool>, m: Option<bool>| SkillState {
            enabled: e,
            must_show: m,
        };
        let before_p = BTreeMap::from([("sql-review".to_owned(), st(Some(false), None))]);
        let after_g = BTreeMap::from([("aws-deploy".to_owned(), st(Some(true), None))]);
        let after_p = BTreeMap::from([("sql-review".to_owned(), st(None, Some(true)))]);
        assert_eq!(
            count_changes((&BTreeMap::new(), &before_p), (&after_g, &after_p)),
            3
        );
    }
}
