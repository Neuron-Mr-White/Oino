//! The frontend view of nested agents (`_oino/agents`, GA-44): the agent tree of a root
//! session, live child event forwarding and lookups by id.
//!
//! Every nested agent belongs to the tree of one root session. Tree changes (spawn, close,
//! run state) publish [`OinoEvent::AgentsChanged`] with the whole tree on the root session's
//! hub; while a frontend follows the tree ([`AgentSession::follow_agents`]) every child client
//! event is also published there as [`OinoEvent::AgentEvent`].

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};

use super::ChildAgent;
use crate::events::{ClientEvent, OinoEvent};
use crate::session::{AgentSession, Inner};

/// Run state of a nested agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRunState {
    /// A run is active.
    Running,
    /// Waiting for input.
    Idle,
    /// Closed (final).
    Closed,
}

/// One nested agent as frontends see it (`_oino/agents`, `agents_changed`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    /// Agent id (the child session id).
    pub id: String,
    /// Parent agent id; `None` for children of the root session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Label from the spec (`Subagent …`, `Sidekick`, `btw`, …).
    pub label: String,
    /// Extension that spawned it.
    pub owner: String,
    /// Child role (`AgentSpec.childRole`, else the owner).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Run state.
    pub state: AgentRunState,
    /// Nesting depth (1 = child of the root).
    pub depth: u8,
    /// Child session file (`None` in memory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
}

/// The root session of the tree `inner` belongs to.
pub(crate) fn root_of(inner: &Arc<Inner>) -> Arc<Inner> {
    let mut current = Arc::clone(inner);
    loop {
        let parent = current
            .agents
            .link
            .as_ref()
            .and_then(|link| link.parent.upgrade());
        match parent {
            Some(parent) => current = parent,
            None => return current,
        }
    }
}

impl ChildAgent {
    /// Run state.
    #[must_use]
    pub fn run_state(&self) -> AgentRunState {
        if self.is_closed() {
            AgentRunState::Closed
        } else if self.session().is_idle() {
            AgentRunState::Idle
        } else {
            AgentRunState::Running
        }
    }

    fn info(&self, parent_id: Option<&str>) -> AgentInfo {
        let inner = &self.session().inner;
        AgentInfo {
            id: self.id().to_owned(),
            parent_id: parent_id.map(str::to_owned),
            label: self.label().to_owned(),
            owner: self.owner().to_owned(),
            role: inner.agents.link.as_ref().and_then(|l| l.role.clone()),
            state: self.run_state(),
            depth: inner.agents.depth,
            session_file: self.session_file().map(|p| p.display().to_string()),
        }
    }
}

fn collect(session: &AgentSession, parent_id: Option<&str>, out: &mut Vec<AgentInfo>) {
    for child in session.child_agents() {
        out.push(child.info(parent_id));
        collect(child.session(), Some(child.id()), out);
    }
}

impl AgentSession {
    /// Every open nested agent of this session's subtree, parents before children.
    #[must_use]
    pub fn agent_tree(&self) -> Vec<AgentInfo> {
        let mut out = Vec::new();
        collect(self, None, &mut out);
        out
    }

    /// Finds a nested agent of this session's subtree by id.
    #[must_use]
    pub fn find_agent(&self, id: &str) -> Option<Arc<ChildAgent>> {
        for child in self.child_agents() {
            if child.id() == id {
                return Some(child);
            }
            if let Some(found) = child.session().find_agent(id) {
                return Some(found);
            }
        }
        None
    }

    /// Turns live child event forwarding ([`OinoEvent::AgentEvent`]) on or off for the
    /// whole tree.
    pub fn follow_agents(&self, on: bool) {
        self.inner.agents.tree.follow.store(on, Ordering::SeqCst);
    }

    /// Whether child events are forwarded.
    #[must_use]
    pub fn follows_agents(&self) -> bool {
        self.inner.agents.tree.follow.load(Ordering::SeqCst)
    }

    /// Publishes [`OinoEvent::AgentsChanged`] on the root session of this tree.
    pub(crate) fn publish_agents_changed(&self) {
        let root = AgentSession::from_inner(root_of(&self.inner));
        let agents = root.agent_tree();
        root.hub().publish(OinoEvent::AgentsChanged { agents });
    }
}

/// Forwards one child event to the root session while the tree is followed.
pub(crate) fn forward_to_root(parent: &Arc<Inner>, agent_id: &str, event: &ClientEvent) {
    if !parent.agents.tree.follow.load(Ordering::SeqCst) {
        return;
    }
    let root = root_of(parent);
    root.hub.publish(OinoEvent::AgentEvent {
        agent_id: agent_id.to_owned(),
        event: Box::new(event.clone()),
    });
}

/// Whether `event` changes the child's run state (tree snapshot refresh).
pub(crate) fn changes_state(event: &ClientEvent) -> bool {
    use crate::events::AgentSessionEvent;
    matches!(
        event,
        ClientEvent::Session(
            AgentSessionEvent::AgentStart | AgentSessionEvent::AgentSettled { .. }
        )
    )
}
