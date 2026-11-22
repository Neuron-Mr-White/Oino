//! Pure `superagent.fleet` merge (oin77 canvas spec §10): session card rows, dispatcher
//! status and agent trees go in, the flat canvas payload comes out. No I/O, no locks —
//! `SessionHub::superagent_fleet` gathers rows and runtime facts (the card index, the
//! board registry, every registered runtime's agent tree) and this module only joins,
//! groups, orders and hashes them.

use oino_core::agents::{AgentInfo, AgentRunState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A dispatcher timestamp (RFC 3339 string) or session time (ms) as epoch ms.
fn epoch_ms_of(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        chrono::DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()
            .and_then(|t| u64::try_from(t.timestamp_millis()).ok())
    })
}

fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// The coarse fleet state of one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Working,
    Waiting,
    Idle,
    Ended,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
            Self::Ended => "ended",
        }
    }
}

/// What the host knows about one claimed session (dispatcher `status` rows; a lead
/// entry wins over the legacy worker rows of the same session).
#[derive(Clone, Debug, Default)]
struct Dossier {
    lead_claim: bool,
    worker_claim: bool,
    /// OIN-80 record state (`working` when absent — a pre-OIN-80 status row).
    lead_state: Option<String>,
    idle_reason: Option<String>,
    health: Option<Value>,
    last_active_at: Option<u64>,
    question: Option<Value>,
    waiting: bool,
    mode: Option<String>,
    model: Option<String>,
    started_at: Option<u64>,
    tickets: Vec<String>,
    step: Option<String>,
    tool: Option<String>,
    project_slug: Option<String>,
}

/// One registered board project (slug → display name + canonical root).
#[derive(Clone, Debug)]
struct Board {
    name: String,
    root: PathBuf,
}

/// One session row the gatherer handed over, with its Superagent-folder membership.
#[derive(Clone, Debug)]
struct Base {
    row: Value,
    router_folder: bool,
}

/// Builds the `superagent.fleet` payload from independently collected snapshots.
/// Feed it in any order, then call [`FleetBuilder::finish`].
#[derive(Clone, Debug, Default)]
pub struct FleetBuilder {
    sessions: BTreeMap<String, Base>,
    dossiers: HashMap<String, Dossier>,
    boards: HashMap<String, Board>,
    live: HashSet<String>,
    waiting: HashSet<String>,
    running: HashSet<String>,
    trees: Vec<(String, Vec<AgentInfo>, HashMap<String, String>)>,
    models: HashMap<String, String>,
    superagent_id: Option<String>,
}

/// A group row of the payload (`groups`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Group {
    key: String,
    name: String,
    has_board: bool,
}

impl FleetBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// One registered board project: a session whose cwd is the root belongs to the
    /// board's group, and a claim on it uses its slug as `projectId`.
    pub fn add_board(&mut self, slug: &str, name: &str, root: PathBuf) {
        self.boards.insert(
            slug.to_owned(),
            Board {
                name: name.to_owned(),
                root,
            },
        );
    }

    /// The Superagent router chat the canvas draws as the root (live router first,
    /// else the most recently active one — decided by the gatherer).
    pub fn set_superagent_id(&mut self, id: Option<String>) {
        self.superagent_id = id;
    }

    /// A session with a live runtime in this host: quiet means `idle`, not `ended`.
    pub fn add_live(&mut self, id: &str) {
        self.live.insert(id.to_owned());
    }

    /// A session with a pending owner question (arbiter wait reasons).
    pub fn add_waiting(&mut self, id: &str) {
        self.waiting.insert(id.to_owned());
    }

    /// A session whose run is in flight right now (`GetState` `isStreaming`): the node is
    /// `working` even when the card state has not caught up.
    pub fn set_running(&mut self, id: &str) {
        self.running.insert(id.to_owned());
    }

    /// The model a live session selected (`provider/id`); fills the node even when the
    /// card index has no model (the runtime knows, the index does not). Sessions added
    /// later pick it up in [`FleetBuilder::finish`].
    pub fn set_model(&mut self, id: &str, model: &str) {
        if let Some(base) = self.sessions.get_mut(id) {
            base.row["model"] = json!(model);
        }
        self.models.insert(id.to_owned(), model.to_owned());
    }

    /// One session row (a card row the hub built): creates or replaces the node's
    /// session facts. `router_folder` marks Superagent-folder chats; a row may instead
    /// carry `leadMarker` (the `oino:superagent-worker` entry read by the card index) —
    /// a session known to be or to have been a lead.
    pub fn add_session(&mut self, row: &Value, router_folder: bool) {
        let Some(id) = row["sessionId"].as_str().map(str::to_owned) else {
            return;
        };
        self.sessions.insert(
            id,
            Base {
                row: row.clone(),
                router_folder,
            },
        );
    }

    /// The dispatcher status snapshot: lead/worker claims with mode, model, tickets,
    /// step/tool and needs-you, plus the board registry (`projects` rows carrying
    /// `root`; explicitly unregistered rows are ignored).
    pub fn add_status(&mut self, status: &Value) {
        if let Some(id) = status["superagentId"].as_str() {
            self.superagent_id = Some(id.to_owned());
        }
        for row in rows(&status["projects"]) {
            if row["registered"].as_bool() == Some(false) {
                continue;
            }
            let (Some(slug), Some(name)) = (row["projectId"].as_str(), row["name"].as_str()) else {
                continue;
            };
            let board = self.boards.entry(slug.to_owned()).or_insert_with(|| Board {
                name: name.to_owned(),
                root: PathBuf::new(),
            });
            name.clone_into(&mut board.name);
            if let Some(root) = row["root"].as_str() {
                board.root = PathBuf::from(root);
            }
        }
        let waiting: HashSet<String> = rows(&status["needsYou"])
            .iter()
            .flat_map(|entry| {
                [
                    entry["workerSessionId"].as_str(),
                    entry["sessionId"].as_str(),
                ]
            })
            .flatten()
            .map(str::to_owned)
            .collect();
        for row in rows(&status["leads"]) {
            let Some(id) = row["sessionId"].as_str().map(str::to_owned) else {
                continue;
            };
            let dossier = self.dossiers.entry(id.clone()).or_default();
            dossier.lead_claim = true;
            dossier.lead_state = row["state"].as_str().map(str::to_owned);
            dossier.idle_reason = row["idleReason"].as_str().map(str::to_owned);
            dossier.health = row["health"].as_object().map(|_| row["health"].clone());
            dossier.last_active_at = epoch_ms_of(&row["lastActiveAt"]);
            dossier.question = row["question"].as_object().map(|_| row["question"].clone());
            dossier.mode = Some(
                match row["mode"].as_str() {
                    Some("graph") => "graph",
                    Some("swarm") => "swarm",
                    _ => "plain",
                }
                .to_owned(),
            );
            if let Some(model) = row["model"].as_str().filter(|model| !model.is_empty()) {
                dossier.model = Some(model.to_owned());
            }
            if let Some(at) = epoch_ms_of(&row["startedAt"]) {
                dossier.started_at = Some(at);
            }
            if let Some(step) = row["step"].as_str().filter(|step| !step.is_empty()) {
                dossier.step = Some(step.to_owned());
            }
            if let Some(tool) = row["tool"].as_str().filter(|tool| !tool.is_empty()) {
                dossier.tool = Some(tool.to_owned());
            }
            for ticket in row["ticketIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !dossier.tickets.iter().any(|known| known == ticket) {
                    dossier.tickets.push(ticket.to_owned());
                }
            }
            // An explicit null projectId (dispatcher without the project row) keeps the
            // session's own workspace instead of inventing a board.
            if !row["projectId"].is_null() {
                dossier.project_slug = row["projectId"].as_str().map(str::to_owned);
            }
            dossier.waiting = waiting.contains(&id) || self.waiting.contains(&id);
        }
        for row in rows(&status["workers"]) {
            let Some(id) = row["sessionId"].as_str().map(str::to_owned) else {
                continue;
            };
            let dossier = self.dossiers.entry(id.clone()).or_default();
            if dossier.lead_claim {
                // Legacy overlap: the lead row already describes this session.
                continue;
            }
            dossier.worker_claim = true;
            if !row["projectId"].is_null() {
                dossier.project_slug = row["projectId"].as_str().map(str::to_owned);
            }
            dossier.waiting |= waiting.contains(&id);
        }
    }

    /// One root session's agent subtree (`AgentSession::agent_tree` of every registered
    /// runtime: dedicated Superagent chats, `HostWorkers` leads, chats with open
    /// subagents). `models` carries `provider/id` per live child id (`None` when the
    /// child is hibernated).
    pub fn add_tree(
        &mut self,
        root_id: &str,
        tree: &[AgentInfo],
        models: &HashMap<String, String>,
    ) {
        if tree.is_empty() {
            return;
        }
        self.trees
            .push((root_id.to_owned(), tree.to_vec(), models.clone()));
    }

    fn add_agent_subtrees(&self, out: &mut BTreeMap<String, Value>) {
        // Agent subtrees: children inherit their root's project/group and gain an edge.
        for (root, tree, models) in &self.trees {
            let Some(parent) = out.get(root) else {
                continue;
            };
            let inherited = (
                parent["project"].clone(),
                parent["projectId"].clone(),
                parent["mode"].clone(),
                parent["groupKey"].clone(),
                parent["cwd"].clone(),
            );
            let (project, project_id, mode, group_key, cwd) = inherited;
            for child in tree {
                let state = match child.state {
                    AgentRunState::Running => State::Working,
                    AgentRunState::Idle | AgentRunState::Hibernated => State::Idle,
                    AgentRunState::Closed => State::Ended,
                };
                let node = json!({
                    "id": child.id,
                    "parentId": child.parent_id.clone().unwrap_or_else(|| root.clone()),
                    "project": project,
                    "projectId": project_id,
                    "title": child.label,
                    "mode": mode,
                    "role": "subagent",
                    "status": state.as_str(),
                    "model": models.get(&child.id).map_or(Value::Null, |model| json!(model)),
                    "name": child.label,
                    "cwd": cwd,
                    "ticketIds": Vec::<Value>::new(),
                    "step": Value::Null,
                    "tool": Value::Null,
                    "startedAt": Value::Null,
                    "endedAt": Value::Null,
                    "lastActiveAt": Value::Null,
                    "groupKey": group_key,
                });
                out.entry(child.id.clone()).or_insert(node);
            }
        }
    }

    fn add_claim_only_nodes(&self, out: &mut BTreeMap<String, Value>) {
        // Claim-only nodes (a dispatcher claim whose session row never arrived).
        for (id, dossier) in &self.dossiers {
            if out.contains_key(id) {
                continue;
            }
            let mut node = json!({
                "id": id,
                "parentId": Value::Null,
                "title": id,
                "project": Value::Null,
                "projectId": Value::Null,
                "mode": "plain",
                "role": if dossier.lead_claim { "lead" } else { "worker" },
                "status": if dossier.waiting { "waiting" } else { "working" },
                "model": Value::Null,
                "name": Value::Null,
                "cwd": Value::Null,
                "ticketIds": Vec::<Value>::new(),
                "step": Value::Null,
                "tool": Value::Null,
                "startedAt": dossier.started_at.map_or(Value::Null, |ms| json!(ms)),
                "endedAt": Value::Null,
                "lastActiveAt": Value::Null,
                "groupKey": Value::Null,
            });
            apply_dossier(&mut node, dossier, &self.boards);
            out.insert(id.clone(), node);
        }
    }

    fn node_groups(&self, out: &BTreeMap<String, Value>) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for node in out.values() {
            let Some(key) = node["groupKey"].as_str() else {
                continue;
            };
            if !seen.insert(key.to_owned()) {
                continue;
            }
            if let Some(board) = self.boards.get(key) {
                groups.push(Group {
                    key: key.to_owned(),
                    name: board.name.clone(),
                    has_board: true,
                });
            } else if let Some(cwd) = key.strip_prefix("dir:") {
                groups.push(Group {
                    key: key.to_owned(),
                    name: base_name(cwd),
                    has_board: false,
                });
            }
        }
        groups.sort_by(|a, b| {
            b.has_board
                .cmp(&a.has_board)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.key.cmp(&b.key))
        });
        if out.values().any(|node| node["groupKey"] == "other") {
            groups.push(Group {
                key: "other".into(),
                name: "Other chats".into(),
                has_board: false,
            });
        }
        groups
    }

    fn finish_rows(&self, out: BTreeMap<String, Value>) -> Value {
        let groups = self.node_groups(&out);
        let rank: HashMap<&str, usize> = groups
            .iter()
            .enumerate()
            .map(|(index, group)| (group.key.as_str(), index))
            .collect();
        let mut nodes: Vec<Value> = out.into_values().collect();
        nodes.sort_by(|a, b| {
            let order = |node: &Value| {
                (
                    node["groupKey"] == "other",
                    node["groupKey"]
                        .as_str()
                        .and_then(|key| rank.get(key).copied())
                        .unwrap_or(usize::MAX),
                    node["startedAt"].as_u64().unwrap_or(0),
                    node["id"].as_str().unwrap_or("").to_owned(),
                )
            };
            order(a).cmp(&order(b))
        });
        let ids: HashSet<&str> = nodes
            .iter()
            .filter_map(|node| node["id"].as_str())
            .collect();
        let mut edges: Vec<(String, String)> = nodes
            .iter()
            .filter_map(|node| {
                let parent = node["parentId"].as_str()?;
                ids.contains(parent).then(|| {
                    (
                        parent.to_owned(),
                        node["id"]
                            .as_str()
                            .map_or_else(|| parent.to_owned(), str::to_owned),
                    )
                })
            })
            .collect();
        edges.sort();
        edges.dedup();
        let edges: Vec<Value> = edges
            .into_iter()
            .map(|(from, to)| json!({"from": from, "to": to}))
            .collect();
        let groups: Vec<Value> = groups
            .into_iter()
            .map(|group| json!({"key": group.key, "name": group.name, "hasBoard": group.has_board}))
            .collect();
        let rev = short_digest(&json!({
            "nodes": nodes.iter().map(|node| {
                let mut bare = node.clone();
                if let Some(object) = bare.as_object_mut() {
                    object.remove("lastActiveAt");
                    object.remove("step");
                    object.remove("tool");
                }
                bare
            }).collect::<Vec<_>>(),
            "edges": edges,
            "groups": groups,
            "superagentId": self.superagent_id,
        }));
        json!({
            "nodes": nodes,
            "edges": edges,
            "superagentId": self.superagent_id,
            "groups": groups,
            "rev": rev,
        })
    }

    /// Joins everything into the payload: `nodes`, `edges`, `superagentId`, `groups`
    /// and the content `rev` (stable over structure, status and labels; activity noise
    /// excluded).
    #[must_use]
    pub fn finish(mut self) -> Value {
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
        for (id, base) in &mut self.sessions {
            if base.row["model"].is_null() {
                if let Some(model) = self.models.get(id.as_str()) {
                    base.row["model"] = json!(model);
                }
            }
            let dossier = self.dossiers.get(id);
            let live = self.live.contains(id);
            let claimed = dossier.is_some_and(|d| d.lead_claim || d.worker_claim);
            let marker = base.row["leadMarker"].as_bool().unwrap_or(false);
            let waiting = dossier.is_some_and(|d| d.waiting)
                || self.waiting.contains(id)
                || base.row["needsYou"].as_bool().unwrap_or(false);
            let working = base.row["state"]["kind"] == "working"
                || !base.row["pendingWork"].is_null()
                || self.running.contains(id);
            // A lead without a live runtime is `ended` only when its claim is gone too —
            // a marked session can still be woken (reopen reattaches the board tool).
            let state = if waiting {
                State::Waiting
            } else if working || dossier.is_some_and(|d| d.lead_claim) {
                State::Working
            } else if live || marker {
                State::Idle
            } else {
                State::Ended
            };
            let role = if dossier.is_some_and(|d| d.lead_claim) {
                "lead"
            } else if claimed {
                "worker"
            } else if base.router_folder {
                "superagent"
            } else {
                "chat"
            };
            let updated = base.row["updatedAt"].as_u64();
            let cwd = base.row["cwd"]
                .as_str()
                .filter(|cwd| !cwd.is_empty())
                .map(str::to_owned);
            let mut node = json!({
                "id": id,
                "parentId": Value::Null,
                "title": base.row.get("title").cloned().unwrap_or(Value::Null),
                "project": base.row.get("project").cloned().unwrap_or(Value::Null),
                "projectId": Value::Null,
                "mode": "plain",
                "role": role,
                "status": state.as_str(),
                "model": base.row.get("model").cloned().filter(|m| m.as_str().is_some_and(|s| !s.is_empty())).unwrap_or(Value::Null),
                "name": Value::Null,
                "cwd": cwd.as_ref().map_or(Value::Null, |cwd| json!(cwd)),
                "ticketIds": Vec::<Value>::new(),
                "step": Value::Null,
                "tool": Value::Null,
                "startedAt": epoch_ms_of(&base.row["createdAt"]).map_or(Value::Null, |ms| json!(ms)),
                "endedAt": (state == State::Ended).then_some(updated).flatten().map_or(Value::Null, |ms| json!(ms)),
                "lastActiveAt": updated.map_or(Value::Null, |ms| json!(ms)),
                "groupKey": Value::Null,
            });
            if let Some(dossier) = dossier {
                apply_dossier(&mut node, dossier, &self.boards);
            }
            out.insert(id.clone(), node);
        }
        self.add_claim_only_nodes(&mut out);
        // Session-node groups first: cwd decides — the board project of the folder it
        // sits in, a no-board folder, or the shared "other" group. Superagent chats
        // join none. Trees are applied afterwards so children inherit the root's
        // final groupKey.
        assign_groups(&mut out, &self.boards);
        self.add_agent_subtrees(&mut out);
        self.finish_rows(out)
    }
}

/// Applies a dispatcher claim onto a node (role, status, mode/model, tickets, step,
/// tool, startedAt, project/projectId).
fn apply_dossier(node: &mut Value, dossier: &Dossier, boards: &HashMap<String, Board>) {
    node["role"] = json!(if dossier.lead_claim { "lead" } else { "worker" });
    node["status"] = json!(if dossier.lead_claim {
        match dossier.lead_state.as_deref() {
            // The record decides: Unloaded shows idle with its idleReason,
            // Stalled shows stalled; everything else keeps the live shape.
            Some("unloaded") => "idle",
            Some("stalled") => "stalled",
            Some("idle") if !dossier.waiting => "idle",
            _ if dossier.waiting => "waiting",
            _ => "working",
        }
    } else if dossier.waiting {
        "waiting"
    } else {
        "working"
    });
    node["idleReason"] = dossier
        .idle_reason
        .as_ref()
        .map_or(Value::Null, |reason| json!(reason));
    node["health"] = dossier.health.clone().unwrap_or(Value::Null);
    node["question"] = dossier.question.clone().unwrap_or(Value::Null);
    if let Some(at) = dossier.last_active_at {
        node["lastActiveAt"] = json!(at);
    }
    node["endedAt"] = Value::Null;
    node["ticketIds"] = json!(dossier.tickets);
    node["step"] = dossier
        .step
        .as_ref()
        .map_or(Value::Null, |step| json!(step));
    node["tool"] = dossier
        .tool
        .as_ref()
        .map_or(Value::Null, |tool| json!(tool));
    if let Some(mode) = &dossier.mode {
        node["mode"] = json!(mode);
    }
    if let Some(model) = &dossier.model {
        node["model"] = json!(model);
    }
    if let Some(at) = dossier.started_at {
        node["startedAt"] = json!(at);
    }
    if let Some(slug) = &dossier.project_slug {
        node["projectId"] = json!(slug);
        node["project"] = json!(boards
            .get(slug)
            .map_or(slug.as_str(), |board| board.name.as_str()));
    }
}

/// The group key of one folder: the board whose canonical root is the cwd, else
/// `dir:<absolute path>`.
fn folder_group(cwd: &str, board_roots: &[(String, PathBuf)]) -> String {
    let folder = canonical(Path::new(cwd));
    for (slug, root) in board_roots {
        if *root == folder {
            return slug.clone();
        }
    }
    format!("dir:{}", folder.display())
}

/// Sets every non-Superagent node's `groupKey` from its cwd (a claim's board slug is
/// the fallback for claim-only rows without a session folder), defaulting to `other`.
fn assign_groups(out: &mut BTreeMap<String, Value>, boards: &HashMap<String, Board>) {
    let board_roots: Vec<(String, PathBuf)> = boards
        .iter()
        .map(|(slug, board)| (slug.clone(), canonical(&board.root)))
        .collect();
    let mut folder_keys: HashMap<String, String> = HashMap::new();
    for node in out.values_mut() {
        if node["role"] == "superagent" {
            continue;
        }
        let key = node["cwd"]
            .as_str()
            .filter(|cwd| !cwd.is_empty())
            .map(|cwd| {
                folder_keys
                    .entry(cwd.to_owned())
                    .or_insert_with(|| folder_group(cwd, &board_roots))
                    .clone()
            })
            .or_else(|| {
                node["projectId"]
                    .as_str()
                    .filter(|slug| boards.contains_key(*slug))
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "other".to_owned());
        node["groupKey"] = json!(key);
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn base_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| path.to_owned(), str::to_owned)
}

/// First 16 hex characters of the canonical payload digest.
fn short_digest(payload: &Value) -> String {
    // One `to_string` is fine here: the serializer would need an io::Write hasher
    // wrapper for streaming, and the canonical string is what defines the digest.
    let mut hasher = Sha256::new();
    hasher.update(payload.to_string().as_bytes());
    let digest = hasher.finalize();
    digest.iter().take(8).fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Test entry point (the hub gathers through [`FleetBuilder`] directly): join raw
/// session rows, dispatcher status and agent trees in one call (rows may carry a
/// `superagentFolder` marker for Superagent-folder chats).
#[cfg(test)]
pub(crate) fn merge(sessions: &Value, status: &Value, trees: &[(String, Vec<AgentInfo>)]) -> Value {
    let mut fleet = FleetBuilder::new();
    fleet.add_status(status);
    for row in rows(sessions) {
        fleet.add_session(row, row["superagentFolder"].as_bool().unwrap_or(false));
    }
    for (root, tree) in trees {
        fleet.add_tree(root, tree, &HashMap::new());
    }
    fleet.finish()
}

#[cfg(test)]
mod fleet_builder_tests {
    use super::*;

    fn info(id: &str, parent: Option<&str>, label: &str, state: AgentRunState) -> AgentInfo {
        AgentInfo {
            id: id.into(),
            parent_id: parent.map(str::to_owned),
            label: label.into(),
            owner: "test".into(),
            role: None,
            state,
            depth: 1,
            session_file: None,
        }
    }

    #[test]
    fn groups_follow_boards_folders_and_other() {
        let dir = tempfile::tempdir().unwrap();
        let board_root = dir.path().join("alpha");
        let folder_root = dir.path().join("plain");
        std::fs::create_dir_all(&board_root).unwrap();
        std::fs::create_dir_all(&folder_root).unwrap();
        let board_cwd = std::fs::canonicalize(&board_root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let folder_cwd = std::fs::canonicalize(&folder_root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut fleet = FleetBuilder::new();
        fleet.add_board("alpha-abc123", "Alpha", board_root);
        fleet.add_session(&json!({"sessionId":"in-board","title":"In board","cwd":board_cwd,"updatedAt":10,"createdAt":1}), false);
        fleet.add_session(&json!({"sessionId":"no-board","title":"No board","cwd":folder_cwd,"updatedAt":11,"createdAt":2}), false);
        fleet.add_session(
            &json!({"sessionId":"nowhere","title":"Nowhere","cwd":"","updatedAt":12,"createdAt":3}),
            false,
        );
        let fleet = fleet.finish();
        let groups = fleet["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups[0],
            json!({"key":"alpha-abc123","name":"Alpha","hasBoard":true})
        );
        assert_eq!(
            groups[1],
            json!({"key":format!("dir:{folder_cwd}"),"name":"plain","hasBoard":false})
        );
        assert_eq!(
            groups[2],
            json!({"key":"other","name":"Other chats","hasBoard":false})
        );
        let get = |id: &str| {
            fleet["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == id)
                .unwrap()
        };
        assert_eq!(get("in-board")["groupKey"], "alpha-abc123");
        assert_eq!(get("in-board")["role"], "chat");
        assert_eq!(get("no-board")["groupKey"], format!("dir:{folder_cwd}"));
        assert_eq!(get("nowhere")["groupKey"], "other");
    }

    #[test]
    fn subagents_inherit_the_root_group_and_gain_edges() {
        let mut fleet = FleetBuilder::new();
        fleet.add_board("alpha-abc123", "Alpha", PathBuf::from("/boards/alpha"));
        fleet.add_session(&json!({"sessionId":"root","title":"Root","cwd":"/boards/alpha","updatedAt":10,"createdAt":1}), false);
        fleet.add_tree(
            "root",
            &[
                info("kid", None, "Explore", AgentRunState::Running),
                info("grand", Some("kid"), "Deep", AgentRunState::Closed),
            ],
            &HashMap::new(),
        );
        let fleet = fleet.finish();
        let get = |id: &str| {
            fleet["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == id)
                .unwrap()
        };
        assert_eq!(get("kid")["role"], "subagent");
        assert_eq!(get("kid")["groupKey"], "alpha-abc123");
        assert_eq!(get("kid")["name"], "Explore");
        assert_eq!(get("kid")["parentId"], "root");
        assert_eq!(get("grand")["parentId"], "kid");
        let edges = fleet["edges"].as_array().unwrap();
        assert!(edges.contains(&json!({"from":"root","to":"kid"})));
        assert!(edges.contains(&json!({"from":"kid","to":"grand"})));
    }

    #[test]
    fn rev_ignores_activity_noise_but_tracks_status() {
        let build = |status: i64| {
            let mut fleet = FleetBuilder::new();
            fleet.add_session(
                &json!({"sessionId":"s","title":"S","cwd":"/w","updatedAt":5,"createdAt":1}),
                false,
            );
            if status == 1 {
                fleet.add_waiting("s");
            }
            let first = fleet.clone().finish();
            let second = fleet.finish();
            (first["rev"].clone(), second["rev"].clone())
        };
        let (quiet_a, quiet_b) = build(0);
        assert_eq!(
            quiet_a, quiet_b,
            "activity-only changes must not change the rev"
        );
        let (waiting_a, waiting_b) = build(1);
        assert_eq!(waiting_a, waiting_b);
        assert_ne!(quiet_a, waiting_b, "a status change must change the rev");
    }

    #[test]
    fn merge_stays_fast_for_500_sessions_and_200_runs() {
        let sessions: Vec<Value> = (0..500)
            .map(|i| {
                json!({"sessionId":format!("s{i}"),"title":format!("Chat {i}"),"cwd":"/w","updatedAt":i,"createdAt":i,
                       "state":{"kind":"idle"},"pendingWork":null,"needsYou":false})
            })
            .collect();
        let tree: Vec<AgentInfo> = (0..200)
            .map(|i| {
                info(
                    &format!("c{i}"),
                    None,
                    "Explore",
                    if i % 3 == 0 {
                        AgentRunState::Running
                    } else {
                        AgentRunState::Closed
                    },
                )
            })
            .collect();
        let tree_count = tree.len();
        let trees = vec![("root".to_owned(), tree)];
        let sessions = json!([
            sessions.as_slice(),
            &[json!({"sessionId":"root","title":"Root","cwd":"/w","updatedAt":500,"createdAt":0})]
        ]
        .concat());
        let status = json!({"projects":[{"projectId":"w","name":"W","root":"/w","registered":true}],"leads":[],"workers":[],"needsYou":[]});
        // One warm-up merge first (allocator and code paths); the measured run is the
        // steady state the poll loop actually sees.
        let _ = merge(&sessions, &status, &trees);
        let start = std::time::Instant::now();
        let fleet = merge(&sessions, &status, &trees);
        let elapsed = start.elapsed();
        let node_count = fleet["nodes"].as_array().unwrap().len();
        assert_eq!(node_count, sessions.as_array().unwrap().len() + tree_count);
        assert!(elapsed.as_millis() < 50, "merge took {elapsed:?}");
    }
}

#[cfg(test)]
mod fleet_tests {
    use crate::superagent_fleet::{merge, FleetBuilder};
    use oino_core::agents::{AgentInfo, AgentRunState};
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn child(id: &str, parent: Option<&str>, state: AgentRunState) -> AgentInfo {
        AgentInfo {
            id: id.into(),
            parent_id: parent.map(str::to_owned),
            label: id.into(),
            owner: "test".into(),
            role: None,
            state,
            depth: 1,
            session_file: None,
        }
    }

    fn get<'a>(fleet: &'a Value, id: &str) -> &'a Value {
        fleet["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("no node {id}"))
    }

    fn edge(fleet: &Value, from: &str, to: &str) -> bool {
        fleet["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["from"] == from && e["to"] == to)
    }

    // 1: two board projects, each with a lead and two subagents; roles, parents, edges,
    // projectId slugs, group keys and the A-Z board group order.
    #[test]
    fn board_projects_group_leads_and_subagents() {
        let mut fleet = FleetBuilder::new();
        let oino_dir = tempfile::tempdir().unwrap();
        let alpha_dir = tempfile::tempdir().unwrap();
        let oino_cwd = std::fs::canonicalize(oino_dir.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let alpha_cwd = std::fs::canonicalize(alpha_dir.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        fleet.add_board("oino-f6a9c2", "Oino", oino_dir.path().to_path_buf());
        fleet.add_board("alpha-aaa111", "Alpha", alpha_dir.path().to_path_buf());
        for (slug, lead, cwd, kids) in [
            ("oino-f6a9c2", "lead-oino", &oino_cwd, ["sub-1", "sub-2"]),
            ("alpha-aaa111", "lead-alpha", &alpha_cwd, ["sub-3", "sub-4"]),
        ] {
            fleet.add_session(&json!({"sessionId": lead, "title": format!("Lead · {slug}"), "cwd": cwd, "createdAt": 1, "updatedAt": 100, "state": {"kind": "idle"}}), false);
            let tree: Vec<_> = kids
                .iter()
                .map(|kid| child(kid, None, AgentRunState::Running))
                .collect();
            fleet.add_tree(lead, &tree, &std::collections::HashMap::new());
        }
        fleet.add_status(&json!({"projects": [
            {"projectId": "oino-f6a9c2", "name": "Oino", "root": oino_cwd, "registered": true},
            {"projectId": "alpha-aaa111", "name": "Alpha", "root": alpha_cwd, "registered": true}
        ], "leads": [
            {"sessionId": "lead-alpha", "projectId": "alpha-aaa111", "mode": "graph", "model": "prov/alpha-model", "startedAt": 10, "ticketIds": ["A-1"]},
            {"sessionId": "lead-oino", "projectId": "oino-f6a9c2", "mode": "swarm", "model": "prov/oino-model", "startedAt": 5, "ticketIds": ["O-1", "O-2"]}
        ]}));
        let fleet = fleet.finish();
        let groups = fleet["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "groups: {groups:?}");
        assert_eq!(groups[0]["key"], "alpha-aaa111");
        assert_eq!(groups[0]["name"], "Alpha");
        assert_eq!(groups[0]["hasBoard"], true);
        assert_eq!(groups[1]["key"], "oino-f6a9c2");
        let lead = get(&fleet, "lead-oino");
        assert_eq!(lead["role"], "lead");
        assert_eq!(lead["parentId"], Value::Null);
        assert_eq!(lead["projectId"], "oino-f6a9c2");
        assert_eq!(lead["groupKey"], "oino-f6a9c2");
        assert_eq!(lead["mode"], "swarm");
        assert_eq!(lead["status"], "working");
        assert_eq!(lead["ticketIds"], json!(["O-1", "O-2"]));
        let sub = get(&fleet, "sub-1");
        assert_eq!(sub["role"], "subagent");
        assert_eq!(sub["parentId"], "lead-oino");
        assert_eq!(sub["groupKey"], "oino-f6a9c2");
        assert_eq!(sub["projectId"], "oino-f6a9c2");
        assert!(edge(&fleet, "lead-oino", "sub-1"));
        assert!(edge(&fleet, "lead-oino", "sub-2"));
        assert!(edge(&fleet, "lead-alpha", "sub-3"));
        assert!(!edge(&fleet, "lead-oino", "sub-3"));
    }

    // 2: an ordinary chat lands in the board group, a folder without a board in a `dir:`
    // group, and a folder-less chat in `other`.
    #[test]
    fn chats_group_by_board_folder_and_other() {
        let dir = tempfile::tempdir().unwrap();
        let board_dir = dir.path().join("board");
        let plain_dir = dir.path().join("plain");
        std::fs::create_dir_all(&board_dir).unwrap();
        std::fs::create_dir_all(&plain_dir).unwrap();
        let board_cwd = std::fs::canonicalize(&board_dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let plain_cwd = std::fs::canonicalize(&plain_dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut fleet = FleetBuilder::new();
        fleet.add_board("oino-f6a9c2", "Oino", board_dir);
        fleet.add_session(&json!({"sessionId": "chat-board", "title": "In project", "cwd": board_cwd, "updatedAt": 10, "createdAt": 1, "state": {"kind": "idle"}}), false);
        fleet.add_session(&json!({"sessionId": "chat-plain", "title": "No board", "cwd": plain_cwd, "updatedAt": 11, "createdAt": 2, "state": {"kind": "idle"}}), false);
        fleet.add_session(&json!({"sessionId": "chat-nowhere", "title": "No folder", "cwd": null, "updatedAt": 12, "createdAt": 3, "state": {"kind": "idle"}}), false);
        let fleet = fleet.finish();
        assert_eq!(get(&fleet, "chat-board")["role"], "chat");
        assert_eq!(get(&fleet, "chat-board")["groupKey"], "oino-f6a9c2");
        assert_eq!(get(&fleet, "chat-board")["parentId"], Value::Null);
        assert_eq!(
            get(&fleet, "chat-plain")["groupKey"],
            format!("dir:{plain_cwd}")
        );
        let groups = fleet["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups[0],
            json!({"key": "oino-f6a9c2", "name": "Oino", "hasBoard": true})
        );
        assert_eq!(
            groups[1],
            json!({"key": format!("dir:{plain_cwd}"), "name": "plain", "hasBoard": false})
        );
        assert_eq!(
            groups[2],
            json!({"key": "other", "name": "Other chats", "hasBoard": false})
        );
        assert_eq!(get(&fleet, "chat-nowhere")["groupKey"], "other");
    }

    // 3: more than 40 ended and older than 24h sessions all stay; there is no cutoff.
    #[test]
    fn history_has_no_caps() {
        let sessions: Vec<Value> = (0..60)
            .map(|i| json!({"sessionId": format!("old-{i}"), "title": "Old", "updatedAt": i, "createdAt": i, "state": {"kind": "idle"}}))
            .collect();
        let fleet = merge(&json!(sessions), &Value::Null, &[]);
        let nodes = fleet["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 60);
        assert!(nodes.iter().all(|n| n["status"] == "ended"));
    }

    // 4: a HostWorkers lead's tree arrives through the same runtime registry as any
    // session: children get subagent roles and edges.
    #[test]
    fn host_workers_lead_tree_is_included() {
        let mut fleet = FleetBuilder::new();
        fleet.add_board("oino-f6a9c2", "Oino", PathBuf::from("/ws/oino"));
        fleet.add_session(&json!({"sessionId": "lead-hw", "title": "Lead · Oino", "cwd": "/ws/oino", "updatedAt": 100, "createdAt": 1, "state": {"kind": "idle"}}), false);
        fleet.add_status(&json!({"leads": [{"sessionId": "lead-hw", "projectId": "oino-f6a9c2", "mode": "graph", "ticketIds": ["O-9"]}]}));
        fleet.add_tree(
            "lead-hw",
            &[
                child("hw-sub-1", None, AgentRunState::Running),
                child("hw-sub-2", Some("hw-sub-1"), AgentRunState::Closed),
            ],
            &std::collections::HashMap::new(),
        );
        let fleet = fleet.finish();
        let sub = get(&fleet, "hw-sub-2");
        assert_eq!(sub["role"], "subagent");
        assert_eq!(sub["parentId"], "hw-sub-1");
        assert_eq!(sub["status"], "ended");
        assert!(edge(&fleet, "lead-hw", "hw-sub-1"));
        assert!(edge(&fleet, "hw-sub-1", "hw-sub-2"));
    }

    // 5: superagentId is set, and a second Superagent chat keeps role superagent.
    #[test]
    fn superagent_chats_get_their_own_role() {
        let mut fleet = FleetBuilder::new();
        fleet.add_session(&json!({"sessionId": "router-live", "title": "Superagent", "cwd": null, "updatedAt": 20, "createdAt": 1, "state": {"kind": "idle"}}), true);
        fleet.add_session(&json!({"sessionId": "router-old", "title": "Superagent", "cwd": null, "updatedAt": 10, "createdAt": 2, "state": {"kind": "idle"}}), true);
        fleet.set_superagent_id(Some("router-live".into()));
        let fleet = fleet.finish();
        assert_eq!(fleet["superagentId"], "router-live");
        assert_eq!(get(&fleet, "router-live")["role"], "superagent");
        assert_eq!(get(&fleet, "router-old")["role"], "superagent");
        assert_eq!(get(&fleet, "router-live")["groupKey"], Value::Null);
        // Superagent chats join no group, so no group exists at all.
        assert_eq!(
            fleet["groups"].as_array().unwrap().as_slice(),
            [] as [serde_json::Value; 0]
        );
    }

    // 6: model, name, cwd, ticketIds and lastActiveAt are filled where the host knows.
    #[test]
    fn node_facts_are_filled() {
        let mut fleet = FleetBuilder::new();
        fleet.add_session(&json!({"sessionId": "lead-x", "title": "Lead", "cwd": "/ws/x", "updatedAt": 77, "createdAt": 5, "model": "prov/card-model", "state": {"kind": "idle"}}), false);
        fleet.add_status(&json!({"leads": [{"sessionId": "lead-x", "projectId": "p-1", "mode": "graph", "model": "prov/lead-model", "startedAt": 42, "step": "running", "tool": "edit", "ticketIds": ["P-1"]}]}));
        fleet.add_tree(
            "lead-x",
            &[child("spec-kid", None, AgentRunState::Idle)],
            &std::collections::HashMap::new(),
        );
        let fleet = fleet.finish();
        let lead = get(&fleet, "lead-x");
        assert_eq!(lead["model"], "prov/lead-model");
        assert_eq!(lead["ticketIds"], json!(["P-1"]));
        assert_eq!(lead["step"], "running");
        assert_eq!(lead["tool"], "edit");
        assert_eq!(lead["startedAt"], 42);
        assert_eq!(lead["lastActiveAt"], 77);
        assert_eq!(lead["cwd"], "/ws/x");
        let kid = get(&fleet, "spec-kid");
        assert_eq!(kid["name"], "spec-kid");
        assert_eq!(kid["cwd"], "/ws/x");
        assert_eq!(kid["lastActiveAt"], Value::Null);
    }

    // 7: the rev is stable for identical input, changes when a status changes, and
    // ignores lastActiveAt/step/tool churn.
    #[test]
    fn rev_tracks_structure_and_status_not_activity() {
        let build = |waiting: bool, step: &str| {
            let mut fleet = FleetBuilder::new();
            fleet.add_session(&json!({"sessionId": "s", "title": "S", "cwd": "/w", "updatedAt": 5, "createdAt": 1, "state": {"kind": "idle"}}), false);
            let mut status =
                json!({"leads": [{"sessionId": "s", "projectId": "p", "mode": "plain"}]});
            status["leads"][0]["step"] = json!(step);
            if waiting {
                status["needsYou"] = json!([{"sessionId": "s"}]);
            }
            fleet.add_status(&status);
            fleet.finish()["rev"].clone()
        };
        assert_eq!(build(true, "running"), build(true, "running"));
        assert_eq!(
            build(false, "running"),
            build(false, "chirp"),
            "step churn must not change the rev"
        );
        assert_ne!(
            build(false, "running"),
            build(true, "running"),
            "a status change must change the rev"
        );
    }
}
