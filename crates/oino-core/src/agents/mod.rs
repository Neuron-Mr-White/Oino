//! Nested agents (runtime §12, D8): extensions spawn child `AgentSession`s in the same
//! process through `agents.spawn`. Children share the parent's services (settings,
//! resources, models, provider components) and may run with none, all or some of its
//! extensions.
//!
//! | Item | Contents |
//! |---|---|
//! | [`spec`] | [`AgentSpec`] and its policies (camelCase JSON) |
//! | [`ChildAgent`] | handle: prompt, steer, follow-up, abort, events, usage, close |
//! | `AgentsState` | per-session bookkeeping: depth, children, tree-wide limits, host router |
//!
//! Limits: depth ≤ the parent's allowance (child default 2, hard cap 4); concurrent open
//! children per root ≤ `extensionSettings.subagents.maxConcurrent` (default 8, hard cap 16);
//! close grace ≤ 5 s. Child usage is appended to the parent as `usage` entries of kind
//! `child:<label>`. The turn arbiter is disabled in children.

mod child;
pub mod spec;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use oino_agent::StreamFn;

pub use child::ChildAgent;
pub use spec::{
    AgentSpec, ExtensionPolicy, Persistence, Seed, SystemPromptSpec, ToolPolicy, UiPolicy,
};

use crate::extensions::runner::ExtensionSet;
use crate::services::HostRouter;
use crate::session::{Inner, lock};

/// Open children of one session tree (root and descendants).
#[derive(Debug, Default)]
pub(crate) struct Tree {
    live: AtomicUsize,
}

impl Tree {
    /// Reserves a slot when fewer than `max` children are open.
    fn try_acquire(&self, max: usize) -> bool {
        self.live
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < max).then_some(n + 1)
            })
            .is_ok()
    }

    fn release(&self) {
        let _ = self
            .live
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
    }
}

/// What a child session knows about its parent.
#[derive(Clone)]
pub(crate) struct ChildLink {
    pub(crate) parent: Weak<Inner>,
    pub(crate) label: String,
    pub(crate) role: Option<String>,
    pub(crate) ui: UiPolicy,
    pub(crate) prompt: SystemPromptSpec,
}

/// Child inputs of `SessionConfig`.
#[derive(Clone)]
pub struct ChildConfig {
    pub(crate) link: ChildLink,
    pub(crate) all_extensions: Arc<ExtensionSet>,
    pub(crate) base_stream: Arc<dyn StreamFn>,
    pub(crate) tree: Arc<Tree>,
    pub(crate) depth: u8,
    pub(crate) max_depth: u8,
    pub(crate) router: Option<Weak<HostRouter>>,
}

impl std::fmt::Debug for ChildConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildConfig")
            .field("label", &self.link.label)
            .field("depth", &self.depth)
            .finish_non_exhaustive()
    }
}

/// Nested-agent bookkeeping of one session.
pub(crate) struct AgentsState {
    pub(crate) router: OnceLock<Weak<HostRouter>>,
    pub(crate) all_extensions: Arc<ExtensionSet>,
    pub(crate) base_stream: Arc<dyn StreamFn>,
    pub(crate) link: Option<ChildLink>,
    pub(crate) children: Mutex<Vec<Arc<ChildAgent>>>,
    pub(crate) tree: Arc<Tree>,
    pub(crate) depth: u8,
    pub(crate) max_depth: u8,
}

impl AgentsState {
    /// State of a root session.
    pub(crate) fn root(all_extensions: Arc<ExtensionSet>, base_stream: Arc<dyn StreamFn>) -> Self {
        Self {
            router: OnceLock::new(),
            all_extensions,
            base_stream,
            link: None,
            children: Mutex::new(Vec::new()),
            tree: Arc::new(Tree::default()),
            depth: 0,
            max_depth: spec::HARD_MAX_DEPTH,
        }
    }

    /// State of a child session.
    pub(crate) fn child(config: ChildConfig) -> Self {
        let router = config.router;
        let state = Self {
            router: OnceLock::new(),
            all_extensions: config.all_extensions,
            base_stream: config.base_stream,
            link: Some(config.link),
            children: Mutex::new(Vec::new()),
            tree: config.tree,
            depth: config.depth,
            max_depth: config.max_depth,
        };
        if let Some(router) = router {
            let _ = state.router.set(router);
        }
        state
    }

    pub(crate) fn router(&self) -> Option<Arc<HostRouter>> {
        self.router.get().and_then(Weak::upgrade)
    }

    pub(crate) fn remove_child(&self, id: &str) {
        lock(&self.children).retain(|c| c.id() != id);
    }
}
