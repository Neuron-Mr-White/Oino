//! Mode registry, owner coordinator and settings (unipi `modes.ts`, `owner.ts`, `settings.ts`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unipi_kit::jev;
use unipi_kit::settings::{Field, Kind, Namespace, Section, Values, choice};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.long-horizon";
/// Settings namespace.
pub const NAMESPACE: &str = "long-horizon";
/// Graph orchestration prompt.
pub const GRAPH_ORCHESTRATION: &str = include_str!("../assets/graph-orchestration.md");
/// Swarm orchestration prompt.
pub const SWARM_ORCHESTRATION: &str = include_str!("../assets/swarm-orchestration.md");

/// unipi `LhMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// One objective.
    Goal,
    /// Checklist loop.
    Ralph,
    /// Parallel graph.
    Swarm,
    /// Dependent graph.
    Graph,
    /// Plain.
    None,
}

/// unipi `OwnerKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerKind {
    /// Goal.
    #[serde(rename = "goal")]
    Goal,
    /// Ralph loop.
    #[serde(rename = "ralph-loop")]
    RalphLoop,
    /// Swarm.
    #[serde(rename = "swarm")]
    Swarm,
    /// Graph.
    #[serde(rename = "graph")]
    Graph,
}

impl OwnerKind {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::RalphLoop => "ralph-loop",
            Self::Swarm => "swarm",
            Self::Graph => "graph",
        }
    }

    /// unipi `modeForOwnerKind`.
    #[must_use]
    pub fn mode(self) -> Mode {
        match self {
            Self::Goal => Mode::Goal,
            Self::RalphLoop => Mode::Ralph,
            Self::Swarm => Mode::Swarm,
            Self::Graph => Mode::Graph,
        }
    }
}

/// All modes.
pub const MODES: [Mode; 5] = [
    Mode::Goal,
    Mode::Ralph,
    Mode::Swarm,
    Mode::Graph,
    Mode::None,
];

impl Mode {
    /// Parses a mode id.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "goal" => Self::Goal,
            "ralph" => Self::Ralph,
            "swarm" => Self::Swarm,
            "graph" => Self::Graph,
            "none" => Self::None,
            _ => return None,
        })
    }
    /// Id.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Ralph => "ralph",
            Self::Swarm => "swarm",
            Self::Graph => "graph",
            Self::None => "none",
        }
    }
    /// Label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Goal => "Goal",
            Self::Ralph => "Ralph",
            Self::Swarm => "Swarm",
            Self::Graph => "Graph",
            Self::None => "Plain",
        }
    }
    /// Routing rubric.
    #[must_use]
    pub fn rubric(self) -> &'static str {
        match self {
            Self::Goal => "One objective pursued across many turns until verifiably true",
            Self::Ralph => "Work through a task file / checklist over many iterations",
            Self::Swarm => "Independent items parallel workers settle, then synthesize",
            Self::Graph => "Multi-step work where later steps depend on earlier results",
            Self::None => "Straight-to-the-point request — no long-horizon machinery",
        }
    }
    /// Mode-exclusive control tools.
    #[must_use]
    pub fn control_tools(self) -> &'static [&'static str] {
        match self {
            Self::Goal => &["create_goal", "get_goal", "update_goal", "todowrite"],
            Self::Ralph => &["ralph_done", "loop_status", "todowrite"],
            Self::Swarm => &[
                "update_agent_graph",
                "yield_agent_graph",
                "agent_swarm_status",
                "agent_output",
                "view_agent_graph",
                "todowrite",
            ],
            Self::Graph => &[
                "view_agent_graph",
                "update_agent_graph",
                "yield_agent_graph",
                "agent_output",
                "agent_swarm_status",
                "todowrite",
            ],
            Self::None => &[],
        }
    }
    /// Delegation tools fully exposed.
    #[must_use]
    pub fn full_delegation(self) -> bool {
        matches!(self, Self::Swarm | Self::Graph)
    }
}

/// Every mode-control tool.
#[must_use]
pub fn all_mode_tools() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for m in MODES {
        for t in m.control_tools() {
            if !out.contains(t) {
                out.push(t);
            }
        }
    }
    out
}

/// unipi `syncModeTools`: the active set with exactly `mode`'s control tools among mode tools.
#[must_use]
pub fn sync_mode_tools(current: &[String], mode: Mode) -> Option<Vec<String>> {
    let all = all_mode_tools();
    let wanted = mode.control_tools();
    if all
        .iter()
        .all(|t| current.iter().any(|c| c == t) == wanted.contains(t))
    {
        return None;
    }
    let mut kept: Vec<String> = current
        .iter()
        .filter(|c| !all.contains(&c.as_str()) || wanted.contains(&c.as_str()))
        .cloned()
        .collect();
    for w in wanted {
        if !kept.iter().any(|k| k == w) {
            kept.push((*w).to_owned());
        }
    }
    Some(kept)
}

/// unipi `hiddenToolNames`.
#[must_use]
pub fn hidden_tools(mode: Mode) -> Vec<&'static str> {
    all_mode_tools()
        .into_iter()
        .filter(|t| !mode.control_tools().contains(t))
        .collect()
}

/// Owner status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OwnerStatus {
    /// Driving.
    Active,
    /// Parked.
    Parked,
}

/// Lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    /// Owner id.
    pub owner_id: String,
    /// Generation.
    pub generation: u64,
}

/// unipi `OwnerState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    /// Id.
    pub owner_id: String,
    /// Kind.
    pub kind: OwnerKind,
    /// Label.
    pub label: String,
    /// Status.
    pub status: OwnerStatus,
    /// Park reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Revision.
    pub revision: u64,
    /// Lease.
    pub lease: Lease,
    /// Updated (ISO).
    pub updated_at: String,
}

/// History entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// Id.
    pub owner_id: String,
    /// Kind.
    pub kind: OwnerKind,
    /// Label.
    pub label: String,
    /// Terminal reason.
    pub terminal_reason: String,
    /// End (ISO).
    pub ended_at: String,
}

/// unipi `CHILD_OWNER_REFUSAL`.
pub const CHILD_OWNER_REFUSAL: &str = "Long-horizon owners are not available inside a child agent — do the work and report back to the lead.";
const HISTORY_LIMIT: usize = 10;

/// unipi `OwnerCoordinator` (persisted as `state.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owners {
    /// Active owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<Owner>,
    /// Parked owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked: Option<Owner>,
    /// Last 10 owners.
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// Last stop `(kind, at)`.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "lastStop")]
    pub last_stop: Option<(String, i64)>,
}

fn renew(o: &Owner) -> Lease {
    Lease {
        owner_id: o.owner_id.clone(),
        generation: o.lease.generation + 1,
    }
}

/// Stop classification for the footer (`complete | paused | budget | other`).
#[must_use]
pub fn stop_kind(reason: &str) -> &'static str {
    if reason.starts_with("complete") {
        "complete"
    } else if reason.starts_with("paused") {
        "paused"
    } else if reason.starts_with("budget_limited") {
        "budget"
    } else {
        "other"
    }
}

impl Owners {
    /// Activates `kind` (fails when an owner is active).
    pub fn activate(
        &mut self,
        id: String,
        kind: OwnerKind,
        label: &str,
        now: &str,
    ) -> Option<Owner> {
        if self.active.is_some() {
            return None;
        }
        let o = Owner {
            lease: Lease {
                owner_id: id.clone(),
                generation: 0,
            },
            owner_id: id,
            kind,
            label: label.to_owned(),
            status: OwnerStatus::Active,
            reason: None,
            revision: 0,
            updated_at: now.to_owned(),
        };
        self.active = Some(o.clone());
        Some(o)
    }
    /// Parks the active owner (park slot must be free).
    pub fn suspend(&mut self, reason: &str, now: &str) -> Option<Owner> {
        if self.parked.is_some() {
            return None;
        }
        let a = self.active.take()?;
        let p = Owner {
            status: OwnerStatus::Parked,
            reason: Some(reason.to_owned()),
            lease: renew(&a),
            revision: a.revision + 1,
            updated_at: now.to_owned(),
            ..a
        };
        self.parked = Some(p.clone());
        self.last_stop = Some(("paused".into(), 0));
        Some(p)
    }
    /// Reactivates the parked owner.
    pub fn resume(&mut self, now: &str) -> Option<Owner> {
        if self.active.is_some() {
            return None;
        }
        let p = self.parked.take()?;
        let a = Owner {
            status: OwnerStatus::Active,
            reason: None,
            lease: renew(&p),
            revision: p.revision + 1,
            updated_at: now.to_owned(),
            ..p
        };
        self.active = Some(a.clone());
        Some(a)
    }
    /// Ends the active owner.
    pub fn finish(&mut self, reason: &str, now: &str, at_ms: i64) -> Option<Owner> {
        let a = self.active.take()?;
        self.history.insert(
            0,
            HistoryEntry {
                owner_id: a.owner_id.clone(),
                kind: a.kind,
                label: a.label.clone(),
                terminal_reason: reason.to_owned(),
                ended_at: now.to_owned(),
            },
        );
        self.history.truncate(HISTORY_LIMIT);
        self.last_stop = Some((stop_kind(reason).to_owned(), at_ms));
        Some(a)
    }
    /// Drops the parked owner.
    pub fn clear_parked(&mut self) -> Option<Owner> {
        self.parked.take()
    }

    /// unipi `ownerSnapshotText`.
    #[must_use]
    pub fn snapshot_text(&self) -> String {
        let mut lines = vec![match &self.active {
            Some(a) => format!(
                "Active owner: {} \"{}\" (rev {}, lease gen {})",
                a.kind.as_str(),
                a.label,
                a.revision,
                a.lease.generation
            ),
            None => "Active owner: none".to_owned(),
        }];
        if let Some(p) = &self.parked {
            lines.push(format!(
                "Parked owner: {} \"{}\" ({})",
                p.kind.as_str(),
                p.label,
                p.reason.as_deref().unwrap_or("paused")
            ));
        }
        if !self.history.is_empty() {
            lines.push(format!(
                "Recent: {}",
                self.history
                    .iter()
                    .take(3)
                    .map(|h| format!("{}→{}", h.kind.as_str(), h.terminal_reason))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines.join("\n")
    }
}

/// unipi `LongHorizonSettings` (judge transport = Decision Model).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Judge on.
    pub judge_enabled: bool,
    /// Judge threshold.
    pub judge_threshold: f64,
    /// Default mode.
    pub default_mode: Mode,
    /// Verifier model (`""` = session).
    pub verifier_model: String,
    /// `loop | status | off`.
    pub goal_progress: String,
    /// Estimate model.
    pub progress_model: String,
}

impl Settings {
    /// unipi `mergeSettings`.
    #[must_use]
    pub fn load(v: &Values) -> Self {
        let t = v
            .get("judge.threshold")
            .and_then(Value::as_f64)
            .filter(|t| *t > 0.0 && *t <= 1.0)
            .unwrap_or(0.8);
        Self {
            judge_enabled: v.bool("judge.enabled", false),
            judge_threshold: t,
            default_mode: Mode::parse(v.str("defaultMode", "none")).unwrap_or(Mode::None),
            verifier_model: v.str("verifierModel", "").to_owned(),
            goal_progress: match v.str("goalProgress", "loop") {
                s @ ("status" | "off") => s.to_owned(),
                _ => "loop".to_owned(),
            },
            progress_model: v.str("progressModel", "").to_owned(),
        }
    }
}

/// unipi `DEFAULT_SETTINGS` + `decisionModel`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "judge": {"enabled": false, "threshold": 0.8},
        "defaultMode": "none", "verifierModel": "", "goalProgress": "loop", "progressModel": "",
        "decisionModel": jev::consumer_defaults(),
    })
}

/// Settings schema.
#[must_use]
pub fn namespace() -> Namespace {
    let model = |key: &str, label: &str, desc: &str, empty: &str| {
        Field::model(key, label).description(desc).with_kind(|k| {
            if let Kind::Model {
                filter,
                empty_label,
                empty_option,
                ..
            } = k
            {
                *filter = Some(json!({"capability": "text"}));
                *empty_label = Some(empty.to_owned());
                *empty_option = Some(empty.to_owned());
            }
        })
    };
    Namespace::new(NAMESPACE, "Long-Horizon", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("judge", "Judge")
                .description("Prompt → mode routing with the Decision Model (shared settings: /unipi:settings → Decision Model)")
                .field(Field::boolean("judge.enabled", "Judge enabled").description("Route new prompts automatically; off always uses the default mode."))
                .field(Field::number("judge.threshold", "Confidence threshold").description("Below this the judge abstains and the default mode applies (0–1).").with_kind(|k| {
                    if let Kind::Number { min, max, .. } = k {
                        *min = Some(0.01);
                        *max = Some(1.0);
                    }
                })),
        )
        .section(jev::consumer_section(Some("Judge — Decision model"), false))
        .section(
            Section::new("modes", "Modes")
                .field(Field::enumeration("defaultMode", "Default mode", vec![
                    choice("goal", "goal", "plan worked goal by goal"),
                    choice("ralph", "ralph", "fixed prompt loop"),
                    choice("swarm", "swarm", "parallel sessions on one goal"),
                    choice("graph", "graph", "dependency graph of tasks"),
                    choice("none", "none", "no long-horizon machinery"),
                ]).description("Used when the judge is off or abstains."))
                .field(model("verifierModel", "Verifier model", "Checks goal completion after each turn.", "inherit (session model)")),
        )
        .section(
            Section::new("progress", "Progress")
                .description("Progress bars shown only to you — never sent to the model")
                .field(Field::enumeration("goalProgress", "Goal progress estimate", vec![
                    choice("loop", "every loop", "estimate after every goal turn and on /goal status"),
                    choice("status", "status only", "only when you run /goal status"),
                    choice("off", "off", "never estimate"),
                ]).description("A one-off side call estimates % done and summarizes the work so far."))
                .field(model("progressModel", "Estimate model", "Model for the progress estimate; runs once per goal turn, cheaper is fine.", "inherit (verifier → session model)")),
        )
}

/// The judge question set (unipi `buildQuestions`).
#[must_use]
pub fn judge_questions() -> Value {
    json!({
        "mode": {"type": "choice", "instructions": "Which execution mode fits this request? Route by task shape and complexity.", "criteria": {
            "goal": "One objective pursued across many turns until verifiably true",
            "ralph": "Work through a task file or checklist over many iterations",
            "swarm": "Several independent items parallel workers can settle, then synthesize",
            "graph": "Multi-step work where later steps depend on earlier results",
            "none": "Straight-to-the-point request — a rename, small edit, quick question",
        }},
        "decomposable": {"type": "noul", "instructions": "Can this be split into independent parallel items?"},
    })
}

/// unipi `renderModeFragment` (inner text; the `long-horizon` section body).
#[must_use]
pub fn mode_fragment(mode: Mode, owner: Option<&Owner>, parked: Option<&Owner>) -> String {
    let mut lines = vec![format!("Mode: {} — {}.", mode.label(), mode.rubric())];
    if mode != Mode::None {
        lines.push(format!(
            "Available long-horizon tools: {}.{}",
            mode.control_tools().join(", "),
            if mode.full_delegation() {
                " Delegation tools are available for parallel work."
            } else {
                ""
            }
        ));
    }
    let mut parts = Vec::new();
    if let Some(o) = owner {
        parts.push(format!(
            "an active {} owner is driving this session",
            o.kind.as_str()
        ));
    }
    if let Some(p) = parked {
        parts.push(format!(
            "a parked {} owner exists (see /unipi:continue)",
            p.kind.as_str()
        ));
    }
    if !parts.is_empty() {
        lines.push(parts.join("; "));
    }
    let body = lines.join("\n");
    match mode {
        Mode::Swarm => format!("{body}\n{SWARM_ORCHESTRATION}"),
        Mode::Graph => format!("{body}\n{GRAPH_ORCHESTRATION}"),
        _ => body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_tool_sync() {
        assert_eq!(all_mode_tools().len(), 11);
        let cur: Vec<String> = ["read", "create_goal", "ralph_done"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let next = sync_mode_tools(&cur, Mode::Ralph).unwrap();
        assert_eq!(next, ["read", "ralph_done", "loop_status", "todowrite"]);
        assert!(sync_mode_tools(&next, Mode::Ralph).is_none());
        assert!(hidden_tools(Mode::None).contains(&"get_goal"));
        assert!(!hidden_tools(Mode::Goal).contains(&"todowrite"));
        assert_eq!(Mode::parse("swarm"), Some(Mode::Swarm));
        assert_eq!(OwnerKind::RalphLoop.mode(), Mode::Ralph);
    }

    #[test]
    fn owners_lifecycle() {
        let mut o = Owners::default();
        assert!(
            o.activate("a".into(), OwnerKind::Goal, "obj", "t")
                .is_some()
        );
        assert!(o.activate("b".into(), OwnerKind::Graph, "x", "t").is_none());
        let p = o.suspend("paused(user_requested)", "t").unwrap();
        assert_eq!((p.status, p.lease.generation), (OwnerStatus::Parked, 1));
        assert!(o.activate("c".into(), OwnerKind::Graph, "g", "t").is_some());
        assert!(o.suspend("x", "t").is_none(), "park slot busy");
        o.finish("complete(finish)", "t", 5);
        assert_eq!(o.last_stop, Some(("complete".into(), 5)));
        let r = o.resume("t").unwrap();
        assert_eq!(r.lease.generation, 2);
        assert!(o.snapshot_text().starts_with(
            "Active owner: goal \"obj\" (rev 2, lease gen 2)\nRecent: graph→complete(finish)"
        ));
        assert_eq!(stop_kind("budget_limited(token)"), "budget");
    }

    #[test]
    fn settings_and_fragment() {
        let s = Settings::load(&Values(
            json!({"judge": {"threshold": 3}, "defaultMode": "graph", "goalProgress": "x"}),
        ));
        assert!((s.judge_threshold - 0.8).abs() < 1e-9);
        assert_eq!(
            (s.default_mode, s.goal_progress.as_str()),
            (Mode::Graph, "loop")
        );
        let f = mode_fragment(Mode::None, None, None);
        assert_eq!(
            f,
            "Mode: Plain — Straight-to-the-point request — no long-horizon machinery."
        );
        let o = Owner {
            owner_id: "i".into(),
            kind: OwnerKind::Graph,
            label: "l".into(),
            status: OwnerStatus::Active,
            reason: None,
            revision: 0,
            lease: Lease {
                owner_id: "i".into(),
                generation: 0,
            },
            updated_at: String::new(),
        };
        let g = mode_fragment(Mode::Graph, Some(&o), None);
        assert!(g.contains("an active graph owner is driving this session\n<orchestration_mode>"));
        assert_eq!(
            namespace().to_json()["sections"].as_array().map(Vec::len),
            Some(4)
        );
        assert_eq!(judge_questions()["mode"]["type"], "choice");
    }
}
