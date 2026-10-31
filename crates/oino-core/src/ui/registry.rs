//! The entry registry (§6): one per scope, owned by the session (or the host
//! catalog for host-scope entries).
//!
//! Every retained producer call of §6.3 becomes an [`UiEntry`]; mutations are
//! queued as [`RegistryOp`]s and flushed as one [`UiPatch`] with a monotonic
//! `rev` (§9.2). Entries are owned by an extension id and are disposed with it
//! (§6.6). Reserved ids are gated to their owner and component (§8).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

use indexmap::IndexMap;
use oino_ai::{JsonObject, JsonValue};
use oino_ui::extras::icon::Icon;
use oino_ui::node::Node;
use oino_ui::patch::{NodeOp, apply, diff};
use oino_ui::slot::{
    DismissReason, EntryMetaPatch, EntrySize, KeyBinding, RegistryOp, ReservedComponent, Scope,
    Slot, UiEntry, UiMeta, UiPatch, UiSnapshot, UiView,
};
use oino_ui::style::Color;
use oino_ui::text::Line;

/// A dialog request in registry form (`UiSnapshot.dialogs`, `op:"dialog"`).
pub type DialogRequest = oino_ui::slot::UiRequest;

/// One reserved first-party panel the embedding host registers (§8): the SDK ships an empty
/// table — every reserved id, its owner and its component key are embedder registration
/// ([`register_reserved`]), so a bare `oino` knows no first-party panels.
#[derive(Clone, Debug, PartialEq)]
pub struct ReservedPanelSpec {
    /// Entry id (`"<extensionId>/<name>"`, or `host/<name>` for host-created ids).
    pub id: String,
    /// Opaque component key the frontend renders (wire value, e.g. `"board"`).
    pub component: ReservedComponent,
    /// The extension id that owns (and gates) it; `host` for host-created ids.
    pub owner: String,
    /// Default slot.
    pub slot: Slot,
    /// Slots the owner may request instead (the default is always allowed).
    pub allowed: Vec<Slot>,
    /// Default title.
    pub title: String,
    /// Default `order` (§6.4) per slot; a slot without an entry defaults to 0.
    pub orders: Vec<(Slot, i32)>,
}

impl ReservedPanelSpec {
    /// A spec with no alternate slots and one default order for every slot.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        owner: impl Into<String>,
        component: impl Into<String>,
        slot: Slot,
        title: impl Into<String>,
        order: i32,
    ) -> Self {
        let slot_key = slot;
        Self {
            id: id.into(),
            component: ReservedComponent::new(component),
            owner: owner.into(),
            slot,
            allowed: Vec::new(),
            title: title.into(),
            orders: vec![(slot_key, order)],
        }
    }

    /// The default `order` of this panel in `slot` (§6.4).
    #[must_use]
    pub fn order_for(&self, slot: Slot) -> i32 {
        self.orders
            .iter()
            .find(|(key, _)| *key == slot)
            .map_or(0, |(_, order)| *order)
    }
}

fn reserved_registry() -> &'static RwLock<Vec<Arc<ReservedPanelSpec>>> {
    static RESERVED: OnceLock<RwLock<Vec<Arc<ReservedPanelSpec>>>> = OnceLock::new();
    RESERVED.get_or_init(|| RwLock::new(Vec::new()))
}

/// Registers a reserved panel for every registry of the process (§8): the embedding host
/// does this once at startup, before sessions open. Re-registering an id replaces its spec;
/// the manifest-side owner table (extension-api) is kept in sync. Idempotent and cheap —
/// embedders may call it from every entry point.
///
/// The panel is gated to `owner`, rendered as `component`, placed in `slot` (alternates in
/// `allowed`) and titled `title`.
pub fn register_reserved(spec: ReservedPanelSpec) {
    let spec = Arc::new(spec);
    let mut table = reserved_registry()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match table.iter_mut().find(|entry| entry.id == spec.id) {
        Some(slot) => *slot = Arc::clone(&spec),
        None => table.push(Arc::clone(&spec)),
    }
    drop(table);
    oino_extension_api::manifest::register_reserved_owner(&spec.id, &spec.owner);
}

/// Drops every registered reserved panel (tests and embedder restarts).
pub fn clear_reserved_panels() {
    reserved_registry()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    oino_extension_api::manifest::clear_reserved_owners();
}

/// Every registered reserved panel, in registration order (§8).
#[must_use]
pub fn reserved_panels() -> Vec<Arc<ReservedPanelSpec>> {
    reserved_registry()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The registered reserved panel with this id, if any (§8).
#[must_use]
pub fn reserved(id: &str) -> Option<Arc<ReservedPanelSpec>> {
    reserved_registry()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
}

/// The default `order` of a registered reserved panel in `slot` (§6.4); 0 for anything else.
#[must_use]
pub fn default_order(id: &str, slot: Slot) -> i32 {
    reserved(id).map_or(0, |spec| spec.order_for(slot))
}

/// Built-in Settings order of §6.4 (`core` first, host/app next).
#[must_use]
pub fn settings_order(extension_id: &str) -> i32 {
    match extension_id {
        "oino" | "core" => 1000,
        "host" | "app" => 900,
        _ => 0,
    }
}

/// Third-party `order` is clamped to this range (§6.4).
const ORDER_RANGE: std::ops::RangeInclusive<i32> = -100..=100;

/// Entries per extension (§4.8).
const MAX_ENTRIES: usize = 64;
/// `sidePane` entries per extension (§4.8).
const MAX_SIDE_PANES: usize = 8;
/// `bottomPane` entries per extension (§4.8).
const MAX_BOTTOM_PANES: usize = 8;

/// Why a registry mutation was rejected.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// An extension tried to write an id outside its own namespace.
    #[error("entry id `{id}` is not owned by `{extension}`")]
    ForeignId {
        /// The requested id.
        id: String,
        /// The requesting extension.
        extension: String,
    },
    /// A reserved id was claimed by another extension.
    #[error("reserved id `{id}` is owned by `{owner}`")]
    ReservedOwned {
        /// The reserved id.
        id: String,
        /// Its owner.
        owner: String,
    },
    /// A non-reserved id was opened as a reserved component.
    #[error("`{0}` is not a reserved id")]
    NotReserved(String),
    /// The extension is at its entry limit.
    #[error("entry limit reached for `{0}`")]
    Limit(String),
    /// The entry does not exist.
    #[error("unknown entry `{0}`")]
    NotFound(String),
    /// The node tree or patch failed validation (§4.8).
    #[error("invalid tree: {0}")]
    Invalid(String),
}

/// Where flushed patches go (the session's event hub).
pub trait RegistrySink: Send + Sync {
    /// Publishes one coalesced patch in order.
    fn publish(&self, patch: UiPatch);
}

/// One producer call's effect on the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProduceOutcome {
    /// Not a registry producer (a one-shot effect, §2 L0).
    Effect,
    /// An entry was created (`created`) or replaced.
    Entry {
        /// Entry id.
        id: String,
        /// Whether the entry is new.
        created: bool,
    },
    /// An entry was deleted.
    Deleted(String),
}

/// Result of [`UiRegistry::put`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PutOutcome {
    /// Entry id.
    pub id: String,
    /// Whether the entry is new.
    pub created: bool,
}

/// An entry to add or replace (§6.1).
#[derive(Clone, Debug, PartialEq)]
pub struct EntryRequest {
    /// Entry id (`"<extensionId>/<name>"`, or a reserved id).
    pub id: String,
    /// Owning extension.
    pub extension_id: String,
    /// Where it renders (`openPanel` placement).
    pub slot: Slot,
    /// Sort weight (§6.4).
    pub order: Option<i32>,
    /// Tab title.
    pub title: Option<String>,
    /// Tab icon.
    pub icon: Option<Icon>,
    /// Tab badge.
    pub badge: Option<Line>,
    /// Tab tone.
    pub tone: Option<Color>,
    /// What it renders.
    pub view: UiView,
    /// Key bindings.
    pub keys: Vec<KeyBinding>,
    /// Activate on open.
    pub focus: Option<bool>,
    /// Esc closes.
    pub close_on_escape: Option<bool>,
    /// Overlay width.
    pub size: Option<EntrySize>,
    /// Fallback tree of a reserved entry (§8 rule 3).
    pub fallback: Option<Node>,
    /// Reserved-component parameters.
    pub params: Option<JsonValue>,
}

/// A partial update of an existing entry (`updatePanel`, §9.1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryUpdate {
    /// Entry id.
    pub id: String,
    /// Whole new tree (core diffs it).
    pub spec: Option<Node>,
    /// Explicit node ops.
    pub patch: Option<Vec<NodeOp>>,
    /// New title.
    pub title: Option<String>,
    /// New icon.
    pub icon: Option<Icon>,
    /// New badge.
    pub badge: Option<Line>,
    /// New sort weight.
    pub order: Option<i32>,
    /// New key bindings.
    pub keys: Option<Vec<KeyBinding>>,
    /// New fallback tree (reserved entries, §8 rule 3).
    pub fallback: Option<Node>,
}

/// The entry's aggregate visibility changed in the attached frontends (§6.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibilityChange {
    /// Entry id.
    pub entry_id: String,
    /// Visible in at least one frontend now.
    pub visible: bool,
}

/// The per-scope entry registry (§6).
pub struct UiRegistry {
    scope: Scope,
    session_id: Option<String>,
    rev: u64,
    seq: u64,
    panels: u64,
    entries: IndexMap<String, UiEntry>,
    meta: UiMeta,
    dialogs: Vec<DialogRequest>,
    settings: HashMap<String, String>,
    visible: HashMap<String, HashSet<String>>,
    pending: Vec<RegistryOp>,
    sink: Option<Arc<dyn RegistrySink>>,
}

impl std::fmt::Debug for UiRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiRegistry")
            .field("scope", &self.scope)
            .field("rev", &self.rev)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl UiRegistry {
    /// An empty session-scope registry.
    #[must_use]
    pub fn new(scope: Scope, session_id: Option<String>) -> Self {
        Self {
            scope,
            session_id,
            rev: 0,
            seq: 0,
            panels: 0,
            entries: IndexMap::new(),
            meta: UiMeta::default(),
            dialogs: Vec::new(),
            settings: HashMap::new(),
            visible: HashMap::new(),
            pending: Vec::new(),
            sink: None,
        }
    }

    /// The emitted patch revision (a frontend at another revision resyncs, §9.2).
    #[must_use]
    pub const fn rev(&self) -> u64 {
        self.rev
    }

    /// Registry scope.
    #[must_use]
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// Where flushed patches go.
    pub fn set_sink(&mut self, sink: Arc<dyn RegistrySink>) {
        self.sink = Some(sink);
    }

    /// Entry ids in registration order.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// One entry by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&UiEntry> {
        self.entries.get(id)
    }

    /// Entries of one slot in registry order (`order` desc, then `seq` asc, §6.4).
    #[must_use]
    pub fn slot_entries(&self, slot: Slot) -> Vec<&UiEntry> {
        let mut entries: Vec<&UiEntry> = self
            .entries
            .values()
            .filter(|entry| entry.slot == slot)
            .collect();
        entries.sort_by(|a, b| b.order.cmp(&a.order).then(a.seq.cmp(&b.seq)));
        entries
    }

    /// The top entry of a stack slot (`overlay`, `editorReplace`): the most
    /// recently opened or focused entry (§6.4).
    #[must_use]
    pub fn stack_top(&self, slot: Slot) -> Option<&UiEntry> {
        self.entries
            .values()
            .filter(|entry| entry.slot == slot)
            .max_by_key(|entry| {
                let opened = if entry.focus == Some(true) {
                    u64::MAX
                } else {
                    entry.seq
                };
                (opened, entry.seq)
            })
    }

    /// Whether a frontend at `rev` must request a resync (§9.2).
    #[must_use]
    pub const fn needs_resync(&self, rev: u64) -> bool {
        rev != self.rev
    }

    /// A full snapshot; drops queued state ops (the snapshot supersedes them) but keeps
    /// one-shot `effect` ops, which are never part of a snapshot (§9.2).
    pub fn snapshot(&mut self) -> UiSnapshot {
        self.pending
            .retain(|op| matches!(op, RegistryOp::Effect { .. }));
        UiSnapshot {
            scope: self.scope,
            session_id: self.session_id.clone(),
            rev: self.rev,
            entries: self.entries.values().cloned().collect(),
            dialogs: self.dialogs.clone(),
            meta: self.meta.clone(),
        }
    }

    /// Takes the queued ops as one patch (`rev` = previous + 1, §9.2).
    pub fn take_patch(&mut self) -> Option<UiPatch> {
        if self.pending.is_empty() {
            return None;
        }
        self.rev += 1;
        Some(UiPatch {
            scope: self.scope,
            session_id: self.session_id.clone(),
            rev: self.rev,
            ops: std::mem::take(&mut self.pending),
        })
    }

    /// Emits the queued ops through the sink, if any.
    pub fn flush(&mut self) {
        if let Some(patch) = self.take_patch()
            && let Some(sink) = &self.sink
        {
            sink.publish(patch);
        }
    }

    /// Whether ops are waiting for a flush.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Queues an op, coalescing consecutive ops for the same entry (§4.7): a later
    /// whole-entry `put` replaces the earlier one, consecutive `tree` ops compose.
    fn push_op(&mut self, op: RegistryOp) {
        match (&op, self.pending.last_mut()) {
            (
                RegistryOp::Put { entry },
                Some(RegistryOp::Put {
                    entry: pending_entry,
                }),
            ) if pending_entry.id == entry.id => *pending_entry = entry.clone(),
            (
                RegistryOp::Tree { id, rev, ops },
                Some(RegistryOp::Tree {
                    id: pending_id,
                    rev: pending_rev,
                    ops: pending_ops,
                }),
            ) if pending_id == id => {
                *pending_rev = *rev;
                pending_ops.extend(ops.iter().cloned());
            }
            _ => self.pending.push(op),
        }
    }
}

/// Why a producer call could not change the registry is reported as
/// [`RegistryError`]; the caller turns it into `host-error{code:"invalid"}` and
/// an `extension_error` diagnostic (§9.1).
impl UiRegistry {
    /// Adds or replaces an entry (`openPanel`, `setWidget`, …).
    ///
    /// # Errors
    ///
    /// Foreign id, reserved-ownership, limit or validation failures.
    pub fn put(&mut self, request: EntryRequest) -> Result<PutOutcome, RegistryError> {
        let id = request.id.clone();
        let extension = request.extension_id.clone();
        check_id(&id)?;
        Self::check_ownership(&extension, &id)?;
        let (slot, view, order) = Self::gate(&request)?;
        let existing = self.entries.get(&id);
        if existing.is_none() {
            self.check_limits(&id, &extension, slot)?;
        }
        let seq = if let Some(entry) = existing {
            entry.seq
        } else {
            self.seq += 1;
            self.seq
        };
        let rev = existing.map_or(1, |entry| entry.rev + 1);
        let title = request
            .title
            .clone()
            .or_else(|| existing.and_then(|entry| entry.title.clone()))
            .or_else(|| reserved(&id).map(|reserved| reserved.title.clone()));
        let entry = UiEntry {
            id: id.clone(),
            extension_id: extension,
            scope: self.scope,
            slot,
            order,
            seq,
            title,
            icon: request.icon,
            badge: request.badge,
            tone: request.tone,
            view,
            keys: request.keys,
            focus: request.focus,
            close_on_escape: request.close_on_escape,
            size: request.size,
            rev,
        };
        let created = existing.is_none();
        match existing {
            Some(old) if only_tree_changed(old, &entry) => {
                if let (UiView::Tree { root: old }, UiView::Tree { root: new }) =
                    (&old.view, &entry.view)
                {
                    let ops = diff(old, new);
                    self.push_op(RegistryOp::Tree {
                        id: entry.id.clone(),
                        rev,
                        ops,
                    });
                }
            }
            _ => self.push_op(RegistryOp::Put {
                entry: entry.clone(),
            }),
        }
        let id = entry.id.clone();
        self.entries.insert(id.clone(), entry);
        Ok(PutOutcome { id, created })
    }

    /// Applies a partial update (`updatePanel`, §9.1).
    ///
    /// # Errors
    ///
    /// [`RegistryError::NotFound`] for an unknown id, plus the [`put`](Self::put)
    /// failures.
    pub fn update(&mut self, update: EntryUpdate) -> Result<(), RegistryError> {
        let Some(old) = self.entries.get(&update.id).cloned() else {
            return Err(RegistryError::NotFound(update.id.clone()));
        };
        Self::check_ownership(&old.extension_id, &update.id)?;
        let mut next = old.clone();
        let mut meta_changed = false;
        if let Some(title) = update.title {
            next.title = Some(title);
            meta_changed = true;
        }
        if let Some(icon) = update.icon {
            next.icon = Some(icon);
            meta_changed = true;
        }
        if let Some(badge) = update.badge {
            next.badge = Some(badge);
            meta_changed = true;
        }
        if let Some(keys) = update.keys {
            next.keys = keys;
            meta_changed = true;
        }
        if let Some(order) = update.order
            && reserved(&update.id).is_none()
        {
            next.order = order.clamp(*ORDER_RANGE.start(), *ORDER_RANGE.end());
            meta_changed = true;
        }
        if let UiView::Reserved { fallback, .. } = &mut next.view
            && let Some(tree) = update.fallback
        {
            *fallback = Some(Box::new(tree));
            meta_changed = true;
        }
        let mut explicit: Option<Vec<NodeOp>> = None;
        if let Some(ops) = update.patch {
            oino_ui::limits::validate_patch(&ops)
                .map_err(|diagnostics| RegistryError::Invalid(diagnostics_text(&diagnostics)))?;
            let UiView::Tree { root } = &next.view else {
                return Err(RegistryError::Invalid(format!(
                    "{} is not a tree entry",
                    update.id
                )));
            };
            let applied =
                apply(root, &ops).map_err(|error| RegistryError::Invalid(error.to_string()))?;
            next.view = UiView::Tree { root: applied };
            explicit = Some(ops);
        }
        if let Some(tree) = update.spec {
            oino_ui::limits::validate(&tree)
                .map_err(|diagnostics| RegistryError::Invalid(diagnostics_text(&diagnostics)))?;
            next.view = UiView::Tree { root: tree };
            explicit = None;
        }
        let view_changed = next.view != old.view;
        if !view_changed && !meta_changed {
            return Ok(());
        }
        next.rev += 1;
        let rev = next.rev;
        let id = next.id.clone();
        let tree_ops = match (&old.view, &next.view) {
            (UiView::Tree { root: old_root }, UiView::Tree { root: new_root }) => {
                Some(explicit.unwrap_or_else(|| diff(old_root, new_root)))
            }
            _ => None,
        };
        match (view_changed, tree_ops) {
            (true, Some(ops)) => {
                self.push_op(RegistryOp::Tree {
                    id: id.clone(),
                    rev,
                    ops,
                });
                if meta_changed {
                    self.pending.push(RegistryOp::Meta {
                        id: id.clone(),
                        set: meta_patch(&old, &next),
                    });
                }
            }
            (true, None) => self.push_op(RegistryOp::Put {
                entry: next.clone(),
            }),
            (false, _) => self.pending.push(RegistryOp::Meta {
                id: id.clone(),
                set: meta_patch(&old, &next),
            }),
        }
        self.entries.insert(id, next);
        Ok(())
    }

    /// Deletes an entry owned by `extension` (`closePanel`, clearing producer, `$close`).
    ///
    /// # Errors
    ///
    /// Foreign id or [`RegistryError::NotFound`].
    pub fn remove(&mut self, extension: &str, id: &str) -> Result<(), RegistryError> {
        Self::check_ownership(extension, id)?;
        if !self.entries.contains_key(id) {
            return Err(RegistryError::NotFound(id.to_owned()));
        }
        self.delete(id);
        Ok(())
    }

    /// Deletes every entry of one extension instance (dispose, §6.6).
    pub fn remove_owner(&mut self, extension: &str) -> Vec<String> {
        let ids: Vec<String> = self
            .entries
            .values()
            .filter(|entry| entry.extension_id == extension)
            .map(|entry| entry.id.clone())
            .collect();
        for id in &ids {
            self.delete(id);
        }
        ids
    }

    /// Deletes every entry (session close / reload, §6.6).
    pub fn clear(&mut self) -> Vec<String> {
        let ids = self.ids();
        for id in &ids {
            self.delete(id);
        }
        ids
    }

    fn delete(&mut self, id: &str) {
        if let Some(entry) = self.entries.shift_remove(id) {
            self.settings.retain(|_, entry_id| entry_id != id);
            for set in self.visible.values_mut() {
                set.remove(id);
            }
            self.pending.push(RegistryOp::Del {
                id: entry.id.clone(),
            });
        }
    }

    /// Creates the turn-footer entry of a renderer registration (§6.7).
    ///
    /// # Errors
    ///
    /// Ownership/limit failures (the entry id is derived, not supplied).
    pub fn register_turn_footer(
        &mut self,
        extension: &str,
        key: &str,
    ) -> Result<String, RegistryError> {
        let id = format!("{extension}/turnFooter:{key}");
        self.put(EntryRequest {
            id: id.clone(),
            extension_id: extension.to_owned(),
            slot: Slot::TurnFooter,
            order: Some(0),
            title: None,
            icon: None,
            badge: None,
            tone: None,
            view: UiView::Renderer {
                key: key.to_owned(),
            },
            keys: Vec::new(),
            focus: None,
            close_on_escape: None,
            size: None,
            fallback: None,
            params: None,
        })?;
        Ok(id)
    }

    /// Invalidates turn-footer caches (`invalidateTurnFooter`, §6.7).
    ///
    /// # Errors
    ///
    /// [`RegistryError::NotFound`] when the owner has no such renderer.
    pub fn invalidate_turn_footer(
        &mut self,
        extension: &str,
        key: &str,
        turns: Option<Vec<String>>,
    ) -> Result<String, RegistryError> {
        let id = format!("{extension}/turnFooter:{key}");
        let Some(entry) = self.entries.get_mut(&id) else {
            return Err(RegistryError::NotFound(id));
        };
        entry.rev += 1;
        self.pending.push(RegistryOp::Invalidate {
            id: id.clone(),
            turns,
        });
        Ok(id)
    }

    /// Registers a Settings namespace entry (§6.3, §7). Collisions are rejected.
    ///
    /// # Errors
    ///
    /// A namespace may only be registered once per scope; plus the [`put`](Self::put)
    /// failures.
    pub fn register_settings(
        &mut self,
        namespace: oino_ui::settings::SettingsNamespace,
    ) -> Result<String, RegistryError> {
        if let Some(existing) = self.settings.get(&namespace.namespace) {
            return Err(RegistryError::Invalid(format!(
                "settings namespace `{}` is already registered by `{existing}`",
                namespace.namespace
            )));
        }
        let extension = namespace.extension_id.clone();
        let order = namespace
            .order
            .unwrap_or_else(|| settings_order(&extension));
        let namespace_key = namespace.namespace.clone();
        let id = format!("{extension}/settings:{}", namespace.namespace);
        let title = namespace.title.clone();
        self.put(EntryRequest {
            id: id.clone(),
            extension_id: extension,
            slot: Slot::Settings,
            order: Some(order),
            title: Some(title),
            icon: namespace.icon,
            badge: None,
            tone: None,
            view: UiView::Settings { namespace },
            keys: Vec::new(),
            focus: None,
            close_on_escape: None,
            size: None,
            fallback: None,
            params: None,
        })?;
        self.settings.insert(namespace_key, id.clone());
        Ok(id)
    }

    /// Sets the registry meta (`setTitle`, `setWorkingMessage`, `setShortcuts`).
    pub fn set_meta(&mut self, set: oino_ui::slot::UiMetaPatch) {
        if let Some(title) = &set.title {
            self.meta.title = Some(title.clone());
        }
        if let Some(message) = &set.working_message {
            self.meta.working_message = Some(message.clone());
        }
        if let Some(visible) = set.working_visible {
            self.meta.working_visible = Some(visible);
        }
        if let Some(shortcuts) = &set.shortcuts {
            self.meta.shortcuts.clone_from(shortcuts);
        }
        self.pending.push(RegistryOp::SessionMeta { set });
    }

    /// Records a dialog (also sent as a pi record to pi clients, §9.2).
    pub fn dialog_opened(&mut self, request: DialogRequest) {
        if !self.dialogs.iter().any(|dialog| dialog.id == request.id) {
            self.dialogs.push(request.clone());
        }
        self.pending.push(RegistryOp::Dialog { request });
    }

    /// Records a dialog dismissal (§9.2).
    pub fn dialog_dismissed(&mut self, id: &str, reason: DismissReason) {
        self.dialogs.retain(|dialog| dialog.id != id);
        self.pending.push(RegistryOp::Dismiss {
            id: id.to_owned(),
            reason,
        });
    }

    /// Records a one-shot effect (§9.2 `effect`; never in snapshots).
    pub fn effect(&mut self, request: DialogRequest) {
        self.pending.push(RegistryOp::Effect { request });
    }

    /// Aggregates a frontend's visibility report (§6.5); `Some` when the
    /// aggregate changed (core then sends `$show`/`$hide`).
    pub fn set_visible(
        &mut self,
        frontend: &str,
        entry_id: &str,
        visible: bool,
    ) -> Option<VisibilityChange> {
        if !self.entries.contains_key(entry_id) {
            return None;
        }
        let was = self.is_visible(entry_id);
        if visible {
            self.visible
                .entry(frontend.to_owned())
                .or_default()
                .insert(entry_id.to_owned());
        } else if let Some(set) = self.visible.get_mut(frontend) {
            set.remove(entry_id);
        }
        let now = self.is_visible(entry_id);
        (was != now).then(|| VisibilityChange {
            entry_id: entry_id.to_owned(),
            visible: now,
        })
    }

    /// Whether any attached frontend reported the entry visible.
    #[must_use]
    pub fn is_visible(&self, entry_id: &str) -> bool {
        self.visible.values().any(|set| set.contains(entry_id))
    }

    /// Deletes an entry by id (user `$close`, §6.6); no ownership check.
    pub fn close(&mut self, id: &str) {
        self.delete(id);
    }

    fn check_ownership(extension: &str, id: &str) -> Result<(), RegistryError> {
        if reserved(id).is_some() {
            return Ok(());
        }
        let owned = id
            .strip_prefix(extension)
            .is_some_and(|rest| rest.starts_with('/'))
            || (extension == "host" && id.starts_with("host/"));
        if owned {
            Ok(())
        } else {
            Err(RegistryError::ForeignId {
                id: id.to_owned(),
                extension: extension.to_owned(),
            })
        }
    }

    fn gate(request: &EntryRequest) -> Result<(Slot, UiView, i32), RegistryError> {
        let fallback = request.fallback.clone().or_else(|| match &request.view {
            UiView::Reserved { fallback, .. } => fallback.as_deref().cloned(),
            _ => None,
        });
        let params = request.params.clone().or_else(|| match &request.view {
            UiView::Reserved { params, .. } => params.clone(),
            _ => None,
        });
        if let Some(entry) = reserved(&request.id) {
            if entry.owner != request.extension_id {
                return Err(RegistryError::ReservedOwned {
                    id: request.id.clone(),
                    owner: entry.owner.clone(),
                });
            }
            let slot = if request.slot == entry.slot || entry.allowed.contains(&request.slot) {
                request.slot
            } else {
                entry.slot
            };
            let order = request
                .order
                .unwrap_or_else(|| default_order(&entry.id, slot));
            return Ok((
                slot,
                UiView::Reserved {
                    component: entry.component.clone(),
                    fallback: fallback.map(Box::new),
                    params,
                },
                order,
            ));
        }
        if matches!(request.view, UiView::Reserved { .. }) {
            return Err(RegistryError::NotReserved(request.id.clone()));
        }
        if let UiView::Tree { root } = &request.view {
            oino_ui::limits::validate(root)
                .map_err(|diagnostics| RegistryError::Invalid(diagnostics_text(&diagnostics)))?;
        }
        // Third-party order is clamped (§6.4); the host owns the built-in Settings
        // constants of core/app namespaces.
        let built_in_settings = request.slot == Slot::Settings
            && matches!(
                request.extension_id.as_str(),
                "oino" | "core" | "host" | "app"
            );
        let order = match request.order {
            Some(order) if built_in_settings => order,
            Some(order) => order.clamp(*ORDER_RANGE.start(), *ORDER_RANGE.end()),
            None => 0,
        };
        Ok((request.slot, request.view.clone(), order))
    }

    fn check_limits(&self, id: &str, extension: &str, slot: Slot) -> Result<(), RegistryError> {
        let owned = self
            .entries
            .values()
            .filter(|entry| entry.extension_id == extension);
        let (mut total, mut side, mut bottom, mut full) = (0_usize, 0_usize, 0_usize, 0_usize);
        for entry in owned {
            total += 1;
            match entry.slot {
                Slot::SidePane => side += 1,
                Slot::BottomPane => bottom += 1,
                Slot::Fullscreen => full += 1,
                _ => {}
            }
        }
        let over = match slot {
            Slot::SidePane => side >= MAX_SIDE_PANES,
            Slot::BottomPane => bottom >= MAX_BOTTOM_PANES,
            Slot::Fullscreen => full >= 1,
            _ => total >= MAX_ENTRIES,
        };
        if over && reserved(id).is_none() {
            return Err(RegistryError::Limit(extension.to_owned()));
        }
        Ok(())
    }
}

/// `true` when only the tree differs (then a `Tree` op is enough).
fn only_tree_changed(old: &UiEntry, new: &UiEntry) -> bool {
    matches!(
        (&old.view, &new.view),
        (UiView::Tree { .. }, UiView::Tree { .. })
    ) && old.title == new.title
        && old.icon == new.icon
        && old.badge == new.badge
        && old.tone == new.tone
        && old.order == new.order
        && old.slot == new.slot
        && old.keys == new.keys
        && old.focus == new.focus
        && old.close_on_escape == new.close_on_escape
        && old.size == new.size
        && old.extension_id == new.extension_id
}

/// The `meta` op for changed entry meta (§9.2).
fn meta_patch(old: &UiEntry, new: &UiEntry) -> EntryMetaPatch {
    EntryMetaPatch {
        title: if old.title == new.title {
            None
        } else {
            new.title.clone()
        },
        icon: if old.icon == new.icon { None } else { new.icon },
        badge: if old.badge == new.badge {
            None
        } else {
            new.badge.clone()
        },
        order: (old.order != new.order).then_some(new.order),
        keys: (old.keys != new.keys).then(|| new.keys.clone()),
        tone: if old.tone == new.tone {
            None
        } else {
            new.tone.clone()
        },
        focus: if old.focus == new.focus {
            None
        } else {
            new.focus
        },
    }
}

/// Renders validation diagnostics as one `extension_error` line (§4.8).
#[must_use]
pub fn diagnostics_text(diagnostics: &[oino_ui::limits::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            if diagnostic.path.is_empty() {
                diagnostic.message.clone()
            } else {
                format!("{}: {}", diagnostic.path.join("."), diagnostic.message)
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Entry ids are `"<extensionId>/<name>"` (1–160 chars) or a reserved id.
fn check_id(id: &str) -> Result<(), RegistryError> {
    if id.is_empty() || id.len() > 160 || !id.contains('/') {
        return Err(RegistryError::Invalid(format!("invalid entry id `{id}`")));
    }
    Ok(())
}

/// Whether `method` is a registry producer (§6.3) rather than a one-shot effect.
#[must_use]
pub fn is_producer(method: &str) -> bool {
    matches!(
        method,
        "setStatus"
            | "setWidget"
            | "setFrame"
            | "setHeader"
            | "setFooter"
            | "openPanel"
            | "updatePanel"
            | "closePanel"
            | "invalidateTurnFooter"
    )
}

/// Whether `method` changes registry **meta** instead of creating an entry (§6.3).
#[must_use]
pub fn is_meta_method(method: &str) -> bool {
    matches!(method, "setTitle" | "setWorkingMessage" | "setShortcuts")
}

fn string(params: &JsonObject, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn line(params: &JsonObject, key: &str) -> Option<Line> {
    let value = params.get(key)?;
    if value.is_null() {
        return None;
    }
    Some(
        serde_json::from_value(value.clone())
            .unwrap_or_else(|_| Line::from(value.as_str().map_or_else(String::new, str::to_owned))),
    )
}

fn color(params: &JsonObject, key: &str) -> Option<Color> {
    serde_json::from_value(params.get(key)?.clone()).ok()
}

fn order(params: &JsonObject) -> Option<i32> {
    params
        .get("priority")
        .or_else(|| params.get("order"))
        .and_then(JsonValue::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn widget_placement(params: &JsonObject) -> Slot {
    match params.get("widgetPlacement").and_then(JsonValue::as_str) {
        Some("belowEditor") => Slot::WidgetBelow,
        _ => Slot::Widget,
    }
}

fn widget_view(params: &JsonObject) -> Result<Option<UiView>, RegistryError> {
    if let Some(spec) = params.get("widgetSpec").filter(|value| !value.is_null()) {
        let root: Node = serde_json::from_value(spec.clone())
            .map_err(|error| RegistryError::Invalid(format!("widgetSpec: {error}")))?;
        oino_ui::limits::validate(&root)
            .map_err(|diagnostics| RegistryError::Invalid(diagnostics_text(&diagnostics)))?;
        return Ok(Some(UiView::Tree { root }));
    }
    match params.get("widgetLines") {
        Some(JsonValue::Array(lines)) => Ok(Some(UiView::Lines {
            lines: lines
                .iter()
                .map(|line| line.as_str().unwrap_or_default().to_owned())
                .collect(),
        })),
        Some(JsonValue::Null) | None => Ok(None),
        Some(_) => Err(RegistryError::Invalid(
            "widgetLines must be an array of strings".to_owned(),
        )),
    }
}
// APPEND

/// Producer calls of §6.3: one retained producer call becomes one registry
/// mutation. Everything else is a one-shot [`ProduceOutcome::Effect`] (or an
/// unknown method the bridge ignores, §3.1).
impl UiRegistry {
    /// Applies one `ui.fire` producer call of `extension`.
    ///
    /// # Errors
    ///
    /// Invalid payloads, foreign ids and limit/validation failures (§9.1).
    pub fn produce(
        &mut self,
        extension: &str,
        method: &str,
        params: &JsonObject,
    ) -> Result<ProduceOutcome, RegistryError> {
        match method {
            "setStatus" => self.produce_status(extension, params),
            "setWidget" => self.produce_widget(extension, params),
            "setFrame" => self.produce_frame(extension, params),
            "setHeader" | "setFooter" => self.produce_header_footer(extension, method, params),
            "setTitle" => {
                let title = string(params, "title");
                self.set_meta(oino_ui::slot::UiMetaPatch {
                    title,
                    ..oino_ui::slot::UiMetaPatch::default()
                });
                Ok(ProduceOutcome::Effect)
            }
            "setWorkingMessage" => {
                let set = oino_ui::slot::UiMetaPatch {
                    working_message: string(params, "message"),
                    working_visible: params.get("visible").and_then(JsonValue::as_bool),
                    ..oino_ui::slot::UiMetaPatch::default()
                };
                self.set_meta(set);
                Ok(ProduceOutcome::Effect)
            }
            "setShortcuts" => {
                let shortcuts = params
                    .get("shortcuts")
                    .and_then(JsonValue::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let keys = item.get("keys")?.as_array()?;
                                Some(KeyBinding {
                                    key: keys.first()?.as_str()?.to_owned(),
                                    action: item
                                        .get("id")
                                        .and_then(JsonValue::as_str)
                                        .map(str::to_owned),
                                    label: item
                                        .get("description")
                                        .and_then(JsonValue::as_str)
                                        .unwrap_or_default()
                                        .to_owned(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.set_meta(oino_ui::slot::UiMetaPatch {
                    shortcuts: Some(shortcuts),
                    ..oino_ui::slot::UiMetaPatch::default()
                });
                Ok(ProduceOutcome::Effect)
            }
            "openPanel" => self.produce_open(extension, params),
            "updatePanel" => self.produce_update(extension, params),
            "closePanel" => {
                let id = string(params, "panelId")
                    .ok_or_else(|| RegistryError::Invalid("closePanel without panelId".into()))?;
                self.remove(extension, &id)?;
                Ok(ProduceOutcome::Deleted(id))
            }
            "invalidateTurnFooter" => {
                let key = string(params, "key").ok_or_else(|| {
                    RegistryError::Invalid("invalidateTurnFooter without key".into())
                })?;
                let turns = params
                    .get("turnEntryIds")
                    .and_then(JsonValue::as_array)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(|id| id.as_str().map(str::to_owned))
                            .collect()
                    });
                self.invalidate_turn_footer(extension, &key, turns)?;
                Ok(ProduceOutcome::Effect)
            }
            _ => Ok(ProduceOutcome::Effect),
        }
    }

    fn produce_status(
        &mut self,
        extension: &str,
        params: &JsonObject,
    ) -> Result<ProduceOutcome, RegistryError> {
        let key = string(params, "statusKey")
            .ok_or_else(|| RegistryError::Invalid("setStatus without statusKey".into()))?;
        let id = format!("{extension}/status:{key}");
        let Some(text) = line(params, "statusText").or_else(|| line(params, "spans")) else {
            if self.entries.contains_key(&id) {
                self.remove(extension, &id)?;
            }
            return Ok(ProduceOutcome::Deleted(id));
        };
        let out = self.put(EntryRequest {
            id,
            extension_id: extension.to_owned(),
            slot: Slot::Status,
            order: order(params),
            title: None,
            icon: params.get("icon").and_then(icon_value),
            badge: None,
            tone: color(params, "tone"),
            view: UiView::Status { text },
            keys: Vec::new(),
            focus: None,
            close_on_escape: None,
            size: None,
            fallback: None,
            params: None,
        })?;
        Ok(ProduceOutcome::Entry {
            id: out.id,
            created: out.created,
        })
    }

    fn produce_widget(
        &mut self,
        extension: &str,
        params: &JsonObject,
    ) -> Result<ProduceOutcome, RegistryError> {
        let key = string(params, "widgetKey")
            .ok_or_else(|| RegistryError::Invalid("setWidget without widgetKey".into()))?;
        let id = format!("{extension}/widget:{key}");
        let Some(view) = widget_view(params)? else {
            if self.entries.contains_key(&id) {
                self.remove(extension, &id)?;
            }
            return Ok(ProduceOutcome::Deleted(id));
        };
        let out = self.put(EntryRequest {
            id,
            extension_id: extension.to_owned(),
            slot: widget_placement(params),
            order: order(params),
            title: None,
            icon: None,
            badge: None,
            tone: None,
            view,
            keys: Vec::new(),
            focus: None,
            close_on_escape: None,
            size: None,
            fallback: None,
            params: None,
        })?;
        Ok(ProduceOutcome::Entry {
            id: out.id,
            created: out.created,
        })
    }

    fn produce_frame(
        &mut self,
        extension: &str,
        params: &JsonObject,
    ) -> Result<ProduceOutcome, RegistryError> {
        let key = string(params, "frameKey")
            .ok_or_else(|| RegistryError::Invalid("setFrame without frameKey".into()))?;
        let prefix = format!("{extension}/frame:{key}:");
        let stale: Vec<String> = self
            .entries
            .keys()
            .filter(|id| id.starts_with(&prefix))
            .cloned()
            .collect();
        let segments = |field: &str| {
            params
                .get(field)
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let segments: Vec<JsonValue> = segments("top")
            .into_iter()
            .chain(segments("bottom"))
            .collect();
        for id in stale {
            self.delete(&id);
        }
        let mut last = None;
        for (index, segment) in segments.iter().enumerate() {
            let Some(object) = segment.as_object() else {
                return Err(RegistryError::Invalid(
                    "setFrame segments must be objects".into(),
                ));
            };
            let Some(text) = line(object, "text") else {
                continue;
            };
            let id = format!("{prefix}{index}");
            let out = self.put(EntryRequest {
                id: id.clone(),
                extension_id: extension.to_owned(),
                slot: Slot::Status,
                order: order(object),
                title: None,
                icon: object.get("icon").and_then(icon_value),
                badge: None,
                tone: color(object, "tone"),
                view: UiView::Status { text },
                keys: Vec::new(),
                focus: None,
                close_on_escape: None,
                size: None,
                fallback: None,
                params: None,
            })?;
            last = Some(out.id);
        }
        Ok(match last {
            Some(id) => ProduceOutcome::Entry { id, created: false },
            None => ProduceOutcome::Deleted(format!("{extension}/frame:{key}")),
        })
    }

    fn produce_header_footer(
        &mut self,
        extension: &str,
        method: &str,
        params: &JsonObject,
    ) -> Result<ProduceOutcome, RegistryError> {
        let key = string(params, "key")
            .ok_or_else(|| RegistryError::Invalid(format!("{method} without key")))?;
        let slot = if method == "setHeader" {
            Slot::Header
        } else {
            Slot::Footer
        };
        let id = format!("{extension}/{method}:{key}");
        if let Some(spec) = params.get("spec").filter(|value| !value.is_null()) {
            let root: Node = serde_json::from_value(spec.clone())
                .map_err(|error| RegistryError::Invalid(format!("{method} spec: {error}")))?;
            let out = self.put(EntryRequest {
                id,
                extension_id: extension.to_owned(),
                slot,
                order: order(params),
                title: None,
                icon: None,
                badge: None,
                tone: None,
                view: UiView::Tree { root },
                keys: Vec::new(),
                focus: None,
                close_on_escape: None,
                size: None,
                fallback: None,
                params: None,
            })?;
            return Ok(ProduceOutcome::Entry {
                id: out.id,
                created: out.created,
            });
        }
        if self.entries.contains_key(&id) {
            self.remove(extension, &id)?;
        }
        Ok(ProduceOutcome::Deleted(id))
    }

    fn produce_open(
        &mut self,
        extension: &str,
        params: &JsonObject,
    ) -> Result<ProduceOutcome, RegistryError> {
        let id = if let Some(id) = string(params, "panelId") {
            id
        } else {
            self.panels += 1;
            format!("{extension}/panel-{}", self.panels)
        };
        let request = panel_request(extension, &id, params)?;
        let out = self.put(request)?;
        Ok(ProduceOutcome::Entry {
            id: out.id,
            created: out.created,
        })
    }

    fn produce_update(
        &mut self,
        extension: &str,
        params: &JsonObject,
    ) -> Result<ProduceOutcome, RegistryError> {
        let id = string(params, "panelId")
            .ok_or_else(|| RegistryError::Invalid("updatePanel without panelId".into()))?;
        // The caller must own the entry; `update` trusts the stored owner.
        Self::check_ownership(extension, &id)?;
        let spec = params
            .get("spec")
            .filter(|value| !value.is_null())
            .map(|value| {
                serde_json::from_value::<Node>(value.clone())
                    .map_err(|error| RegistryError::Invalid(format!("updatePanel spec: {error}")))
            })
            .transpose()?;
        let patch = params
            .get("patch")
            .filter(|value| !value.is_null())
            .map(|value| {
                serde_json::from_value::<Vec<NodeOp>>(value.clone())
                    .map_err(|error| RegistryError::Invalid(format!("updatePanel patch: {error}")))
            })
            .transpose()?;
        let fallback = params
            .get("fallback")
            .filter(|value| !value.is_null())
            .map(|value| {
                serde_json::from_value::<Node>(value.clone()).map_err(|error| {
                    RegistryError::Invalid(format!("updatePanel fallback: {error}"))
                })
            })
            .transpose()?;
        let keys = params
            .get("keys")
            .and_then(|value| serde_json::from_value::<Vec<KeyBinding>>(value.clone()).ok());
        self.update(EntryUpdate {
            id: id.clone(),
            spec,
            patch,
            title: string(params, "title"),
            icon: params.get("icon").and_then(icon_value),
            badge: params
                .get("badge")
                .and_then(|value| serde_json::from_value(value.clone()).ok()),
            order: order(params),
            keys,
            fallback,
        })?;
        Ok(ProduceOutcome::Entry { id, created: false })
    }
}

fn icon_value(value: &JsonValue) -> Option<Icon> {
    serde_json::from_value(value.clone()).ok()
}

fn panel_request(
    extension: &str,
    id: &str,
    params: &JsonObject,
) -> Result<EntryRequest, RegistryError> {
    let slot = match params.get("placement") {
        Some(value) if !value.is_null() => serde_json::from_value::<Slot>(value.clone())
            .map_err(|error| RegistryError::Invalid(format!("openPanel placement: {error}")))?,
        _ => Slot::Overlay,
    };
    let spec = params
        .get("spec")
        .filter(|value| !value.is_null())
        .map(|value| {
            serde_json::from_value::<Node>(value.clone())
                .map_err(|error| RegistryError::Invalid(format!("openPanel spec: {error}")))
        })
        .transpose()?;
    let keys = params
        .get("keys")
        .and_then(|value| serde_json::from_value::<Vec<KeyBinding>>(value.clone()).ok())
        .unwrap_or_default();
    let view = match spec {
        Some(root) => UiView::Tree { root },
        None => UiView::Lines { lines: Vec::new() },
    };
    Ok(EntryRequest {
        id: id.to_owned(),
        extension_id: extension.to_owned(),
        slot,
        order: order(params),
        title: string(params, "title"),
        icon: params.get("icon").and_then(icon_value),
        badge: params
            .get("badge")
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        tone: None,
        view,
        keys,
        focus: params.get("focus").and_then(JsonValue::as_bool),
        close_on_escape: params.get("closeOnEscape").and_then(JsonValue::as_bool),
        size: params
            .get("size")
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        fallback: None,
        params: params.get("params").cloned(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    use super::*;
    use oino_ui::widgets::paragraph::Paragraph;

    fn tree(text: &str) -> Node {
        Paragraph::new(text).into()
    }

    fn request(extension: &str, id: &str, slot: Slot, root: Node) -> EntryRequest {
        EntryRequest {
            id: id.to_owned(),
            extension_id: extension.to_owned(),
            slot,
            order: None,
            title: None,
            icon: None,
            badge: None,
            tone: None,
            view: UiView::Tree { root },
            keys: Vec::new(),
            focus: None,
            close_on_escape: None,
            size: None,
            fallback: None,
            params: None,
        }
    }

    fn registry() -> UiRegistry {
        UiRegistry::new(Scope::Session, Some("s1".to_owned()))
    }

    fn params(value: JsonValue) -> JsonObject {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn entries_render_in_order_then_seq() {
        let mut registry = registry();
        registry
            .put(request("a", "a/one", Slot::SidePane, tree("1")))
            .unwrap();
        registry
            .put(request("a", "a/two", Slot::SidePane, tree("2")))
            .unwrap();
        let mut later = request("b", "b/three", Slot::SidePane, tree("3"));
        later.order = Some(5);
        registry.put(later).unwrap();
        let ids: Vec<&str> = registry
            .slot_entries(Slot::SidePane)
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        // order desc first (b/three), then registration order (a/one, a/two).
        assert_eq!(ids, ["b/three", "a/one", "a/two"]);
    }

    #[test]
    fn stack_slots_show_the_most_recent_entry() {
        let mut registry = registry();
        registry
            .put(request("a", "a/first", Slot::Overlay, tree("1")))
            .unwrap();
        registry
            .put(request("a", "a/second", Slot::Overlay, tree("2")))
            .unwrap();
        assert_eq!(
            registry.stack_top(Slot::Overlay).map(|e| e.id.as_str()),
            Some("a/second")
        );
        let mut focused = request("a", "a/first", Slot::Overlay, tree("1"));
        focused.focus = Some(true);
        registry.put(focused).unwrap();
        assert_eq!(
            registry.stack_top(Slot::Overlay).map(|e| e.id.as_str()),
            Some("a/first")
        );
    }

    #[test]
    fn dispose_removes_only_the_owners_entries() {
        let mut registry = registry();
        registry
            .put(request("a", "a/p", Slot::SidePane, tree("1")))
            .unwrap();
        registry
            .put(request("b", "b/p", Slot::SidePane, tree("2")))
            .unwrap();
        registry
            .put(request("b", "b/w", Slot::Widget, tree("3")))
            .unwrap();
        registry.take_patch();
        let removed = registry.remove_owner("b");
        assert_eq!(removed, ["b/p", "b/w"]);
        assert_eq!(registry.ids(), ["a/p"]);
        let patch = registry.take_patch().unwrap();
        assert_eq!(patch.rev, 2, "the first batch was the three puts");
        assert_eq!(patch.ops.len(), 2);
        assert!(
            patch
                .ops
                .iter()
                .all(|op| matches!(op, RegistryOp::Del { .. }))
        );

        // A user `$close` deletes exactly one entry.
        registry.close("a/p");
        assert_eq!(registry.ids().len(), 0);

        // Session close clears the rest.
        registry
            .put(request("a", "a/p", Slot::SidePane, tree("1")))
            .unwrap();
        assert_eq!(registry.clear(), ["a/p"]);
        assert_eq!(registry.ids().len(), 0);
    }

    #[test]
    fn foreign_ids_are_rejected() {
        let mut registry = registry();
        let error = registry
            .put(request("a", "b/panel", Slot::SidePane, tree("1")))
            .unwrap_err();
        assert_eq!(
            error,
            RegistryError::ForeignId {
                id: "b/panel".to_owned(),
                extension: "a".to_owned()
            }
        );
        assert_eq!(registry.ids().len(), 0);
    }

    #[test]
    fn reserved_ids_are_gated_to_their_owner() {
        register_reserved(ReservedPanelSpec {
            allowed: vec![Slot::Fullscreen, Slot::SidePane],
            orders: vec![(Slot::Fullscreen, 1000), (Slot::SidePane, 960)],
            ..ReservedPanelSpec::new(
                "acme.board/board",
                "acme.board",
                "board",
                Slot::Fullscreen,
                "Board",
                1000,
            )
        });
        register_reserved(ReservedPanelSpec::new(
            "acme.cron/jobs",
            "acme.cron",
            "cron",
            Slot::Fullscreen,
            "Cron",
            990,
        ));
        let mut registry = registry();
        let error = registry
            .put(request(
                "evil",
                "acme.board/board",
                Slot::Fullscreen,
                tree("1"),
            ))
            .unwrap_err();
        assert_eq!(
            error,
            RegistryError::ReservedOwned {
                id: "acme.board/board".to_owned(),
                owner: "acme.board".to_owned()
            }
        );
        // A reserved component outside the table is not allowed either.
        let mut fake = request("evil", "evil/board", Slot::Overlay, tree("1"));
        fake.view = UiView::Reserved {
            component: ReservedComponent::new("board"),
            fallback: None,
            params: None,
        };
        assert_eq!(
            registry.put(fake).unwrap_err(),
            RegistryError::NotReserved("evil/board".to_owned())
        );
        // The owner gets the component entry; its spec is ignored (§8).
        let out = registry
            .put(request(
                "acme.board",
                "acme.board/board",
                Slot::Fullscreen,
                tree("ignored"),
            ))
            .unwrap();
        assert!(out.created);
        let entry = registry.get("acme.board/board").unwrap();
        assert_eq!(
            entry.view,
            UiView::Reserved {
                component: ReservedComponent::new("board"),
                fallback: None,
                params: None,
            }
        );
        assert_eq!(entry.order, 1000);
        assert_eq!(entry.title.as_deref(), Some("Board"));
        // `sidePane` is allowed for the board; other slots fall back to the default.
        registry
            .put(request(
                "acme.board",
                "acme.board/board",
                Slot::SidePane,
                tree("ignored"),
            ))
            .unwrap();
        assert_eq!(
            registry.get("acme.board/board").unwrap().slot,
            Slot::SidePane
        );
        assert_eq!(registry.get("acme.board/board").unwrap().order, 960);
        registry
            .put(request(
                "acme.cron",
                "acme.cron/jobs",
                Slot::SidePane,
                tree("ignored"),
            ))
            .unwrap();
        assert_eq!(
            registry.get("acme.cron/jobs").unwrap().slot,
            Slot::Fullscreen
        );
    }

    #[test]
    fn reserved_fallbacks_are_set_by_update_panel() {
        register_reserved(ReservedPanelSpec::new(
            "acme.kanban/board",
            "acme.kanban",
            "board",
            Slot::Fullscreen,
            "Board",
            1000,
        ));
        let mut registry = registry();
        registry
            .put(request(
                "acme.kanban",
                "acme.kanban/board",
                Slot::Fullscreen,
                tree("ignored"),
            ))
            .unwrap();
        registry
            .update(EntryUpdate {
                id: "acme.kanban/board".to_owned(),
                fallback: Some(tree("tasks")),
                ..EntryUpdate::default()
            })
            .unwrap();
        let entry = registry.get("acme.kanban/board").unwrap();
        let UiView::Reserved { fallback, .. } = &entry.view else {
            panic!("not reserved");
        };
        assert_eq!(fallback.as_deref(), Some(&tree("tasks")));
    }

    #[test]
    fn consecutive_ops_for_one_entry_coalesce() {
        let mut registry = registry();
        let lines = |text: &str| EntryRequest {
            view: UiView::Lines {
                lines: vec![text.to_owned()],
            },
            ..request("a", "a/p", Slot::Widget, tree("unused"))
        };
        registry.put(lines("1")).unwrap();
        registry.put(lines("2")).unwrap();
        let patch = registry.take_patch().unwrap();
        assert_eq!(patch.ops.len(), 1, "the later put replaced the earlier one");
        let RegistryOp::Put { entry } = &patch.ops[0] else {
            panic!("expected a put");
        };
        assert_eq!(entry.rev, 2);
        assert_eq!(
            entry.view,
            UiView::Lines {
                lines: vec!["2".to_owned()]
            }
        );
        assert_eq!(entry.extension_id, "a");
    }

    #[test]
    fn patches_coalesce_and_snapshots_drop_pending() {
        let mut registry = registry();
        registry
            .put(request("a", "a/one", Slot::Widget, tree("1")))
            .unwrap();
        registry
            .put(request("a", "a/two", Slot::Widget, tree("2")))
            .unwrap();
        let patch = registry.take_patch().unwrap();
        assert_eq!(patch.rev, 1, "one patch per flush");
        assert_eq!(patch.ops.len(), 2);
        assert_eq!(patch.session_id.as_deref(), Some("s1"));
        assert!(registry.take_patch().is_none());
        registry
            .put(request("a", "a/three", Slot::Widget, tree("3")))
            .unwrap();
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.rev, 1);
        assert_eq!(snapshot.entries.len(), 3);
        assert!(
            !registry.has_pending(),
            "the snapshot supersedes queued ops"
        );
        registry
            .put(request("a", "a/four", Slot::Widget, tree("4")))
            .unwrap();
        assert_eq!(registry.take_patch().unwrap().rev, 2);
    }

    #[test]
    fn a_revision_gap_requests_a_resync() {
        let mut registry = registry();
        registry
            .put(request("a", "a/one", Slot::Widget, tree("1")))
            .unwrap();
        let patch = registry.take_patch().unwrap();
        assert!(!registry.needs_resync(patch.rev));
        assert!(registry.needs_resync(patch.rev - 1));
        assert!(registry.needs_resync(patch.rev + 3));
    }

    #[test]
    fn tree_updates_emit_keyed_patches() {
        let mut registry = registry();
        registry
            .put(request("a", "a/p", Slot::SidePane, tree("one")))
            .unwrap();
        let first = registry.snapshot().entries[0].clone();
        registry
            .put(request("a", "a/p", Slot::SidePane, tree("two")))
            .unwrap();
        let patch = registry.take_patch().unwrap();
        assert_eq!(patch.ops.len(), 1);
        let RegistryOp::Tree { id, rev, ops } = &patch.ops[0] else {
            panic!("expected a tree op, got {:?}", patch.ops[0]);
        };
        assert_eq!(id, "a/p");
        assert_eq!(*rev, 2);
        let UiView::Tree { root } = &first.view else {
            panic!("not a tree");
        };
        assert_eq!(apply(root, ops).unwrap(), tree("two"));
        assert_eq!(registry.get("a/p").unwrap().rev, 2);
    }

    #[test]
    fn foreign_entries_cannot_be_updated_or_closed() {
        let mut registry = registry();
        registry
            .put(request("a", "a/p", Slot::SidePane, tree("1")))
            .unwrap();
        let update = params(serde_json::json!({
            "panelId": "a/p",
            "patch": [{"op": "set", "path": [], "props": {"text": "hijacked"}}]
        }));
        assert_eq!(
            registry.produce("b", "updatePanel", &update).unwrap_err(),
            RegistryError::ForeignId {
                id: "a/p".to_owned(),
                extension: "b".to_owned()
            }
        );
        let close = params(serde_json::json!({"panelId": "a/p"}));
        assert!(matches!(
            registry.produce("b", "closePanel", &close),
            Err(RegistryError::ForeignId { .. })
        ));
        assert!(registry.get("a/p").is_some());
    }

    #[test]
    fn stale_turn_footers_are_invalidated() {
        let mut registry = registry();
        let id = registry.register_turn_footer("a", "files").unwrap();
        assert_eq!(id, "a/turnFooter:files");
        let entry = registry.get(&id).unwrap();
        assert_eq!(entry.slot, Slot::TurnFooter);
        assert_eq!(
            entry.view,
            UiView::Renderer {
                key: "files".to_owned()
            }
        );
        registry.take_patch();
        registry
            .invalidate_turn_footer("a", "files", Some(vec!["t1".to_owned()]))
            .unwrap();
        let patch = registry.take_patch().unwrap();
        assert_eq!(
            patch.ops,
            [RegistryOp::Invalidate {
                id: id.clone(),
                turns: Some(vec!["t1".to_owned()])
            }]
        );
        assert_eq!(registry.get(&id).unwrap().rev, 2);
        assert_eq!(
            registry
                .invalidate_turn_footer("a", "other", None)
                .unwrap_err(),
            RegistryError::NotFound("a/turnFooter:other".to_owned())
        );
    }

    #[test]
    fn visibility_aggregates_over_frontends() {
        let mut registry = registry();
        registry
            .put(request("a", "a/p", Slot::SidePane, tree("1")))
            .unwrap();
        assert_eq!(
            registry.set_visible("tui", "a/p", true),
            Some(VisibilityChange {
                entry_id: "a/p".to_owned(),
                visible: true
            })
        );
        assert_eq!(registry.set_visible("app", "a/p", true), None);
        assert!(registry.is_visible("a/p"));
        assert_eq!(registry.set_visible("tui", "a/p", false), None);
        assert_eq!(
            registry.set_visible("app", "a/p", false),
            Some(VisibilityChange {
                entry_id: "a/p".to_owned(),
                visible: false
            })
        );
        assert!(!registry.is_visible("a/p"));
        assert_eq!(registry.set_visible("tui", "a/unknown", true), None);
    }

    #[test]
    fn producers_map_to_entries() {
        let mut registry = registry();
        let status =
            params(serde_json::json!({"statusKey": "k", "statusText": "1", "priority": 3}));
        let outcome = registry.produce("a", "setStatus", &status).unwrap();
        assert_eq!(
            outcome,
            ProduceOutcome::Entry {
                id: "a/status:k".to_owned(),
                created: true
            }
        );
        let entry = registry.get("a/status:k").unwrap();
        assert_eq!(entry.slot, Slot::Status);
        assert_eq!(entry.order, 3);
        assert_eq!(
            entry.view,
            UiView::Status {
                text: Line::from("1")
            }
        );

        let widget = params(serde_json::json!({
            "widgetKey": "w",
            "widgetLines": ["a", "b"],
            "widgetPlacement": "belowEditor"
        }));
        registry.produce("a", "setWidget", &widget).unwrap();
        let entry = registry.get("a/widget:w").unwrap();
        assert_eq!(entry.slot, Slot::WidgetBelow);
        assert_eq!(
            entry.view,
            UiView::Lines {
                lines: vec!["a".to_owned(), "b".to_owned()]
            }
        );

        let open = params(serde_json::json!({
            "placement": "sidePane",
            "title": "Panel",
            "spec": {"t": "Paragraph", "text": "hi"}
        }));
        let outcome = registry.produce("a", "openPanel", &open).unwrap();
        let ProduceOutcome::Entry { id, created } = outcome else {
            panic!("not an entry");
        };
        assert!(created);
        assert_eq!(id, "a/panel-1");
        assert_eq!(registry.get(&id).unwrap().title.as_deref(), Some("Panel"));

        // `updatePanel` with explicit ops keeps the stored tree current.
        let update = params(serde_json::json!({
            "panelId": "a/panel-1",
            "patch": [{"op": "set", "path": [], "props": {"text": "bye"}}]
        }));
        registry.produce("a", "updatePanel", &update).unwrap();
        registry.take_patch();

        // A clearing producer deletes its entry.
        let clear = params(serde_json::json!({"statusKey": "k"}));
        assert_eq!(
            registry.produce("a", "setStatus", &clear).unwrap(),
            ProduceOutcome::Deleted("a/status:k".to_owned())
        );
        assert!(registry.get("a/status:k").is_none());
        let close = params(serde_json::json!({"panelId": "a/panel-1"}));
        assert_eq!(
            registry.produce("a", "closePanel", &close).unwrap(),
            ProduceOutcome::Deleted("a/panel-1".to_owned())
        );
        assert_eq!(registry.ids(), ["a/widget:w"]);
    }

    #[test]
    fn one_shot_effects_are_not_producers() {
        let mut registry = registry();
        assert!(!is_producer("notify"));
        assert!(is_producer("openPanel"));
        assert!(is_meta_method("setTitle"));
        let notify = params(serde_json::json!({"message": "hi"}));
        assert_eq!(
            registry.produce("a", "notify", &notify).unwrap(),
            ProduceOutcome::Effect
        );
    }

    #[test]
    fn limits_reject_the_65th_entry() {
        let mut registry = registry();
        for index in 0..MAX_ENTRIES {
            registry
                .put(request(
                    "a",
                    &format!("a/widget:{index}"),
                    Slot::Widget,
                    tree("x"),
                ))
                .unwrap();
        }
        assert_eq!(
            registry
                .put(request("a", "a/widget:over", Slot::Widget, tree("x")))
                .unwrap_err(),
            RegistryError::Limit("a".to_owned())
        );
        // Replacing an existing entry never hits the limit.
        registry
            .put(request("a", "a/widget:0", Slot::Widget, tree("y")))
            .unwrap();
    }

    #[test]
    fn settings_namespaces_register_once() {
        let mut registry = registry();
        let namespace = |namespace: &str| oino_ui::settings::SettingsNamespace {
            namespace: namespace.to_owned(),
            title: "Acme".to_owned(),
            extension_id: "oino".to_owned(),
            ..oino_ui::settings::SettingsNamespace::default()
        };
        let id = registry.register_settings(namespace("acme")).unwrap();
        assert_eq!(id, "oino/settings:acme");
        let entry = registry.get(&id).unwrap();
        assert_eq!(entry.slot, Slot::Settings);
        assert_eq!(entry.order, 1000);
        assert!(matches!(entry.view, UiView::Settings { .. }));
        assert!(
            registry
                .register_settings(namespace("acme"))
                .unwrap_err()
                .to_string()
                .contains("already registered")
        );
    }
}
