//! Patch ops (§4.7): the [`NodeOp`] wire types plus the reference
//! [`diff`] / [`apply`] pair (`apply(old, diff(old, new)) == new`, §4.7).
//!
//! Diffing rule set (§4.7): different `t`/`key` ⇒ `replace`; same kind ⇒ one
//! `set` with all changed scalar/object props; collections match by key —
//! unchanged key sequence ⇒ recurse/`item` per changed member, tail-append-only
//! (`items`/`rows`/`lines`) ⇒ `append`, otherwise one `splice` per maximal
//! contiguous changed run (LIS keeps stable members in place). If the encoded
//! ops exceed 50 % of the encoded new node, a single root `replace` is emitted
//! instead.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::node::{Key, Node};

/// A path from the entry root to a node (§4.6): sibling keys plus the fixed
/// segments `"child"` ([`Block.child`]), `"fallback"` and `"action"`
/// ([`Empty.action`]), and tab keys for `Tabs.panes`.
pub type NodePath = Vec<String>;

/// Collection a patch op addresses (§4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PatchField {
    /// `Layout.children`.
    Children,
    /// `List.items`.
    Items,
    /// `Table.rows`.
    Rows,
    /// `Tree.nodes`.
    Nodes,
    /// `Log.lines`.
    Lines,
    /// `Tabs.titles`.
    Titles,
    /// `Chart.datasets`.
    Datasets,
    /// `Sparkline.data` / `BarChart` bars.
    Data,
    /// `Tabs.panes` (segment = tab key).
    Panes,
}

/// A collection member: a node (`Layout.children`, `Tabs.panes`) or a plain
/// item (`List.items`, `Table.rows`, `Log.lines`, …).
#[allow(
    clippy::large_enum_variant,
    reason = "patch ops are built once per update; boxing every node adds noise"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PatchItem {
    /// A node subtree.
    Node(Node),
    /// A plain item (shape depends on `field`).
    Item(Json),
}

/// One patch op (§4.7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum NodeOp {
    /// Replace the node at `path` (empty path = the entry root).
    Replace {
        /// Path to the node.
        path: NodePath,
        /// The new node.
        node: Node,
    },
    /// Shallow prop merge at `path`; `null` unsets a prop. Not for collections.
    Set {
        /// Path to the node.
        path: NodePath,
        /// Changed props (`null` unsets).
        props: serde_json::Map<String, Json>,
    },
    /// Remove `remove` items at `index` and insert `insert` (§4.7).
    Splice {
        /// Path to the owning node.
        path: NodePath,
        /// Collection to modify.
        field: PatchField,
        /// Index of the first removed item.
        index: u32,
        /// Number of removed items.
        remove: u32,
        /// Items inserted at `index`.
        insert: Vec<PatchItem>,
    },
    /// Append items to a ring-trimmed collection (`Log.lines`, `List.items`,
    /// `Table.rows`); older items drop silently past the limit (§4.8).
    Append {
        /// Path to the owning node.
        path: NodePath,
        /// Collection to extend.
        field: PatchField,
        /// Items appended at the tail.
        items: Vec<PatchItem>,
    },
    /// Replace one keyed item (key sequence unchanged, §4.7).
    Item {
        /// Path to the owning node.
        path: NodePath,
        /// Collection to modify.
        field: PatchField,
        /// Key of the replaced item.
        key: String,
        /// The new item.
        value: PatchItem,
    },
}

/// Why a patch failed to resolve (§4.7 / §9.3): the consumer drops the tree and
/// requests a resync. Ops before the failing one stay applied — the protocol
/// applies ops in order and gives no transactionality guarantee.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// No node resolves at the op path (the resolved prefix is shown).
    MissingPath {
        /// The path prefix that could not be descended.
        path: NodePath,
    },
    /// The node at `path` does not own `field` (wrong kind).
    FieldMismatch {
        /// Path of the offending node.
        path: NodePath,
        /// The collection the op addressed.
        field: PatchField,
        /// Kind of the node found there.
        kind: String,
    },
    /// No member with `key` exists in the collection at `path`.
    MissingKey {
        /// Path of the owning node.
        path: NodePath,
        /// The collection.
        field: PatchField,
        /// The member key that did not resolve.
        key: String,
    },
    /// `index + remove` runs past the end of the collection.
    OutOfBounds {
        /// Path of the owning node.
        path: NodePath,
        /// The collection.
        field: PatchField,
        /// The splice start index.
        index: u32,
        /// Current length of the collection.
        len: usize,
    },
    /// A splice/insert payload is not a member of `field` (e.g. a node in
    /// `rows`).
    BadItem {
        /// Path of the owning node.
        path: NodePath,
        /// The collection.
        field: PatchField,
        /// What was wrong with the payload.
        detail: String,
    },
    /// A `set` op merged the node into something that no longer parses.
    BadPatch {
        /// Path of the offending node.
        path: NodePath,
        /// The (de)serialization error.
        detail: String,
    },
    /// A `panes` splice insert carried a node without the pane key.
    MissingItemKey {
        /// Path of the owning `Tabs` node.
        path: NodePath,
    },
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPath { path } => write!(f, "no node at path {path:?}"),
            Self::FieldMismatch { path, field, kind } => {
                write!(
                    f,
                    "node at {path:?} is {kind}, which has no {field:?} collection"
                )
            }
            Self::MissingKey { path, field, key } => {
                write!(f, "no {field:?} member keyed {key:?} at {path:?}")
            }
            Self::OutOfBounds {
                path,
                field,
                index,
                len,
            } => {
                write!(
                    f,
                    "splice at {index} past end ({len}) of {field:?} at {path:?}"
                )
            }
            Self::BadItem {
                path,
                field,
                detail,
            } => {
                write!(f, "bad {field:?} item at {path:?}: {detail}")
            }
            Self::BadPatch { path, detail } => {
                write!(f, "set at {path:?} broke the node: {detail}")
            }
            Self::MissingItemKey { path } => {
                write!(f, "panes insert at {path:?} without a key")
            }
        }
    }
}

impl std::error::Error for PatchError {}

/// Diffs `old` into `new` (§4.7): ops whose paths are relative to the entry
/// root, with `apply(old, &ops) == new`. Always succeeds — anything the rule
/// set cannot express incrementally degenerates to one root `replace`.
#[must_use]
pub fn diff(old: &Node, new: &Node) -> Vec<NodeOp> {
    let mut ops = Vec::new();
    diff_node(old, new, &NodePath::new(), &mut ops);
    // §4.7 rule 4: ops exceeding 50 % of the encoded new node ⇒ one replace.
    let new_bytes = serde_json::to_vec(new).map_or(usize::MAX, |v| v.len());
    let ops_bytes = serde_json::to_vec(&ops).map_or(usize::MAX, |v| v.len());
    if ops_bytes
        .checked_mul(2)
        .is_none_or(|twice| twice > new_bytes)
    {
        return vec![NodeOp::Replace {
            path: NodePath::new(),
            node: new.clone(),
        }];
    }
    ops
}

/// Applies ops in order (§4.7). The first op that does not resolve aborts the
/// batch with [`PatchError`] — consumers then request a resync (§9.3).
///
/// # Errors
///
/// When a path, key, index or payload does not resolve against the tree.
pub fn apply(old: &Node, ops: &[NodeOp]) -> Result<Node, PatchError> {
    let mut root = old.clone();
    for op in ops {
        apply_op(&mut root, op)?;
    }
    Ok(root)
}

/// One collection member as the differ sees it: derived key (§4.1: explicit
/// `key`, else the positional `#<index>`) plus the payload.
#[allow(
    clippy::large_enum_variant,
    reason = "patch ops are built once per update; boxing every node adds noise"
)]
#[derive(Clone, Debug, PartialEq)]
enum Member {
    /// A node member (`children`, `panes`); recursed into, not serialized.
    Node {
        /// Derived member key.
        key: String,
        /// The subtree.
        node: Node,
    },
    /// A plain item member, kept in its serialized form.
    Item {
        /// Derived member key.
        key: String,
        /// Serialized item.
        json: Json,
    },
}

impl Member {
    /// The derived key. Keys starting with `'#'` are positional (§4.1) and
    /// never anchor members across mutations.
    fn key(&self) -> &str {
        match self {
            Self::Node { key, .. } | Self::Item { key, .. } => key,
        }
    }

    /// The payload as a patch insert.
    fn to_patch_item(&self) -> PatchItem {
        match self {
            Self::Node { node, .. } => PatchItem::Node(node.clone()),
            Self::Item { json, .. } => PatchItem::Item(json.clone()),
        }
    }
}

/// Keys starting with `'#'` are the implicit positional form (§4.1); the differ
/// treats every such key as positional and never matches it across positions.
fn is_positional(key: &str) -> bool {
    key.starts_with('#')
}

/// Derived key of the member at `index` (§4.1).
fn derived_key(explicit: Option<&str>, index: usize) -> String {
    match explicit {
        Some(key) => key.to_owned(),
        None => format!("#{index}"),
    }
}

/// `path` extended by one member-key segment (§4.6).
fn child_path(path: &NodePath, key: &str) -> NodePath {
    let mut path = path.clone();
    path.push(key.to_owned());
    path
}

/// The node props that hold collections addressed by patch ops (§4.7); they
/// are excluded from the shallow `set` prop diff and handled structurally.
fn collection_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "Layout" => &["children"],
        "Tabs" => &["titles", "panes"],
        "List" => &["items"],
        "Table" => &["rows"],
        "Tree" => &["nodes"],
        "Log" => &["lines"],
        "Chart" => &["datasets"],
        "Sparkline" | "BarChart" => &["data"],
        _ => &[],
    }
}

/// The collections a node kind owns, in op order.
fn node_fields(node: &Node) -> &'static [PatchField] {
    match node {
        Node::Layout(_) => &[PatchField::Children],
        Node::Tabs(_) => &[PatchField::Titles, PatchField::Panes],
        Node::List(_) => &[PatchField::Items],
        Node::Table(_) => &[PatchField::Rows],
        Node::Tree(_) => &[PatchField::Nodes],
        Node::Log(_) => &[PatchField::Lines],
        Node::Chart(_) => &[PatchField::Datasets],
        Node::Sparkline(_) | Node::BarChart(_) => &[PatchField::Data],
        _ => &[],
    }
}

/// Serializes a slice of members with a per-member explicit key extractor.
fn seq_members<T: Serialize, F: Fn(&T) -> Option<String>>(
    items: &[T],
    explicit_key: F,
) -> Vec<Member> {
    items
        .iter()
        .enumerate()
        .map(|(index, value)| Member::Item {
            key: explicit_key(value).unwrap_or_else(|| format!("#{index}")),
            json: serde_json::to_value(value).unwrap_or(Json::Null),
        })
        .collect()
}

/// The members of `field` on `node`, in collection order (panes in sorted key
/// order; `None` panes and empty panes both yield no members).
fn members(node: &Node, field: PatchField) -> Vec<Member> {
    match (node, field) {
        (Node::Layout(l), PatchField::Children) => l
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| Member::Node {
                key: derived_key(child.common().key, index),
                node: child.clone(),
            })
            .collect(),
        (Node::Tabs(t), PatchField::Panes) => t
            .panes
            .iter()
            .flatten()
            .map(|(key, pane)| Member::Node {
                key: key.clone(),
                node: pane.clone(),
            })
            .collect(),
        (Node::Tabs(t), PatchField::Titles) => {
            seq_members(&t.titles, |title| Some(title.key.clone()))
        }
        (Node::List(l), PatchField::Items) => seq_members(&l.items, |item| match item {
            crate::widgets::list::ListItem::Full { key: Some(key), .. } => Some(key.clone()),
            _ => None,
        }),
        (Node::Table(t), PatchField::Rows) => seq_members(&t.rows, |row| row.key.clone()),
        (Node::Tree(t), PatchField::Nodes) => seq_members(&t.nodes, |n| Some(n.key.clone())),
        (Node::Log(l), PatchField::Lines) => seq_members(&l.lines, |_| None),
        (Node::Chart(c), PatchField::Datasets) => seq_members(&c.datasets, |_| None),
        (Node::Sparkline(s), PatchField::Data) => seq_members(&s.data, |_| None),
        (Node::BarChart(b), PatchField::Data) => seq_members(&b.data, |_| None),
        _ => Vec::new(),
    }
}

/// Rules 1–3 of §4.7 at one position, appending to `ops`.
fn diff_node(old: &Node, new: &Node, path: &NodePath, ops: &mut Vec<NodeOp>) {
    // Rule 1: different kind or key ⇒ replace.
    if old.kind() != new.kind() || old.common().key != new.common().key {
        ops.push(NodeOp::Replace {
            path: path.clone(),
            node: new.clone(),
        });
        return;
    }
    // Rule 2: one `set` with all changed scalar/object props.
    diff_props(old, new, path, ops);
    // Rule 3: collections matched by key.
    for field in node_fields(new) {
        diff_collection(old, new, *field, path, ops);
    }
}

/// Rule 2: shallow JSON prop diff (collections and identity excluded).
fn diff_props(old: &Node, new: &Node, path: &NodePath, ops: &mut Vec<NodeOp>) {
    let old_json = serde_json::to_value(old).unwrap_or(Json::Null);
    let new_json = serde_json::to_value(new).unwrap_or(Json::Null);
    let (Some(old_obj), Some(new_obj)) = (old_json.as_object(), new_json.as_object()) else {
        return;
    };
    let skip = collection_fields(new.kind());
    let is_skipped = |key: &&str| *key == "t" || skip.contains(key);
    let mut props = serde_json::Map::new();
    for (key, value) in new_obj {
        if is_skipped(&key.as_str()) {
            continue;
        }
        if old_obj.get(key) != Some(value) {
            props.insert(key.clone(), value.clone());
        }
    }
    for key in old_obj
        .keys()
        .filter(|key| !new_obj.contains_key(key.as_str()))
    {
        if is_skipped(&key.as_str()) {
            continue;
        }
        props.insert(key.clone(), Json::Null);
    }
    if !props.is_empty() {
        ops.push(NodeOp::Set {
            path: path.clone(),
            props,
        });
    }
}

/// Rule 3 for one collection.
fn diff_collection(
    old: &Node,
    new: &Node,
    field: PatchField,
    path: &NodePath,
    ops: &mut Vec<NodeOp>,
) {
    let old_m = members(old, field);
    let new_m = members(new, field);
    // Key sequence unchanged ⇒ recurse/`item` per changed member.
    if old_m.len() == new_m.len() && old_m.iter().zip(&new_m).all(|(o, n)| o.key() == n.key()) {
        diff_stable(&old_m, &new_m, field, path, ops);
        return;
    }
    // Tail append only (Log/List/Table, §4.7) ⇒ `append`.
    if matches!(
        field,
        PatchField::Items | PatchField::Rows | PatchField::Lines
    ) && new_m.len() > old_m.len()
        && old_m.iter().zip(&new_m).all(|(o, n)| o == n)
    {
        ops.push(NodeOp::Append {
            path: path.clone(),
            field,
            items: new_m[old_m.len()..]
                .iter()
                .map(Member::to_patch_item)
                .collect(),
        });
        return;
    }
    // `panes` inserts need the pane key on the node (the protocol has no keyed
    // insert op); anything else degenerates to replacing the whole `Tabs`.
    if field == PatchField::Panes
        && !new_m.iter().all(|member| match member {
            Member::Node { key, node } => node.common().key == Some(key.as_str()),
            Member::Item { .. } => false,
        })
    {
        ops.push(NodeOp::Replace {
            path: path.clone(),
            node: new.clone(),
        });
        return;
    }
    // Otherwise: one `splice` per maximal contiguous changed run (LIS keeps
    // stable members in place), then keyed ops for stable-but-changed members.
    let kept = splice_diff(&old_m, &new_m, field, path, ops);
    let stable = kept
        .into_iter()
        .map(|(new_index, old_index)| (&old_m[old_index], &new_m[new_index]));
    for (o, n) in stable {
        if o == n {
            continue;
        }
        emit_stable_change(o, n, field, path, ops);
    }
}

/// Emits the keyed op for one stable member that changed (recursion for node
/// members, `item` for plain items). Mixed node/item pairs cannot occur within
/// one collection.
fn emit_stable_change(
    o: &Member,
    n: &Member,
    field: PatchField,
    path: &NodePath,
    ops: &mut Vec<NodeOp>,
) {
    match (o, n) {
        (Member::Node { node: old_node, .. }, Member::Node { key, node }) => {
            diff_node(old_node, node, &child_path(path, key), ops);
        }
        (_, Member::Item { key, json }) => ops.push(NodeOp::Item {
            path: path.clone(),
            field,
            key: key.clone(),
            value: PatchItem::Item(json.clone()),
        }),
        (Member::Item { .. }, Member::Node { .. }) => {}
    }
}

/// Members whose derived keys line up one-to-one (unchanged key sequence).
fn diff_stable(
    old_m: &[Member],
    new_m: &[Member],
    field: PatchField,
    path: &NodePath,
    ops: &mut Vec<NodeOp>,
) {
    for (o, n) in old_m.iter().zip(new_m) {
        if o == n {
            continue;
        }
        emit_stable_change(o, n, field, path, ops);
    }
}

/// Emits one `splice` per maximal contiguous changed run, keeping LIS-stable
/// members in place, and returns the kept `(new_index, old_index)` pairs.
fn splice_diff(
    old_m: &[Member],
    new_m: &[Member],
    field: PatchField,
    path: &NodePath,
    ops: &mut Vec<NodeOp>,
) -> Vec<(usize, usize)> {
    // Anchor members by explicit (non-positional) key.
    let old_by_key: std::collections::HashMap<&str, usize> = old_m
        .iter()
        .enumerate()
        .filter(|(_, m)| !is_positional(m.key()))
        .map(|(index, m)| (m.key(), index))
        .collect();
    let pairs: Vec<(usize, usize)> = new_m
        .iter()
        .enumerate()
        .filter(|(_, m)| !is_positional(m.key()))
        .filter_map(|(new_index, m)| {
            old_by_key
                .get(m.key())
                .map(|&old_index| (new_index, old_index))
        })
        .collect();
    let kept = longest_increasing_chain(&pairs); // (new_index, old_index), both ascending
    // One `splice` per maximal contiguous changed run. Each run's index is
    // its start in old coordinates, adjusted by the net length change of the
    // runs before it (they are emitted head-first, so by the time a run
    // applies, every earlier run has already landed).
    let mut gaps: Vec<(usize, usize, Vec<&Member>)> = Vec::new(); // (old start, removed, inserts)
    let (mut n_cursor, mut o_cursor) = (0usize, 0usize);
    for (new_index, old_index) in &kept {
        if *new_index > n_cursor || *old_index > o_cursor {
            gaps.push((
                o_cursor,
                old_index - o_cursor,
                new_m[n_cursor..*new_index].iter().collect(),
            ));
        }
        n_cursor = new_index + 1;
        o_cursor = old_index + 1;
    }
    if new_m.len() > n_cursor || old_m.len() > o_cursor {
        gaps.push((
            o_cursor,
            old_m.len() - o_cursor,
            new_m[n_cursor..].iter().collect(),
        ));
    }
    let mut delta: isize = 0; // net member count change of the emitted runs
    for (start, removed, inserts) in &gaps {
        if *removed == 0 && inserts.is_empty() {
            continue;
        }
        let index = isize::try_from(*start).unwrap_or(0) + delta;
        ops.push(NodeOp::Splice {
            path: path.clone(),
            field,
            index: u32::try_from(index.max(0)).unwrap_or(u32::MAX),
            remove: u32::try_from(*removed).unwrap_or(u32::MAX),
            insert: inserts.iter().map(|m| m.to_patch_item()).collect(),
        });
        delta +=
            isize::try_from(inserts.len()).unwrap_or(0) - isize::try_from(*removed).unwrap_or(0);
    }
    kept
}

/// Indices into `pairs` forming the longest strictly increasing chain by
/// `.1` (pairs are ascending by `.0`; old indices are unique per collection).
fn longest_increasing_chain(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut tails: Vec<usize> = Vec::new(); // smallest chain-ending old index per length
    let mut tail_pair: Vec<usize> = Vec::new(); // pair index achieving it
    let mut prev = vec![usize::MAX; pairs.len()];
    for (i, &(_, old_index)) in pairs.iter().enumerate() {
        match tails.binary_search(&old_index) {
            Ok(pos) => {
                tails[pos] = old_index;
                tail_pair[pos] = i;
            }
            Err(pos) => {
                prev[i] = pos.checked_sub(1).map_or(usize::MAX, |q| tail_pair[q]);
                if pos == tails.len() {
                    tails.push(old_index);
                    tail_pair.push(i);
                } else {
                    tails[pos] = old_index;
                    tail_pair[pos] = i;
                }
            }
        }
    }
    let mut chain = Vec::new();
    let mut cursor = tail_pair.last().copied();
    while let Some(i) = cursor {
        chain.push(pairs[i]);
        cursor = (prev[i] != usize::MAX).then_some(prev[i]);
    }
    chain.reverse();
    chain
}

/// Resolves the mutable node at `path` (§4.6: member keys plus the fixed
/// segments `"child"`, `"fallback"` and `"action"`).
fn node_at<'a>(root: &'a mut Node, path: &NodePath) -> Result<&'a mut Node, PatchError> {
    let mut node = root;
    for (depth, segment) in path.iter().enumerate() {
        let missing = || PatchError::MissingPath {
            path: path[..=depth].to_vec(),
        };
        node = match segment.as_str() {
            "child" => match node {
                Node::Block(block) => block.child.as_deref_mut(),
                _ => None,
            }
            .ok_or_else(missing)?,
            "fallback" => fallback_slot(node).ok_or_else(missing)?,
            "action" => match node {
                Node::Empty(empty) => empty.action.as_deref_mut(),
                _ => None,
            }
            .ok_or_else(missing)?,
            key => descend_member(node, key).ok_or_else(missing)?,
        };
    }
    Ok(node)
}

/// Mutable slot of the common `fallback` of any node kind.
fn fallback_slot(node: &mut Node) -> Option<&mut Node> {
    macro_rules! common_fallback {
        ($($variant:ident),* $(,)?) => {
            match node {
                $(Node::$variant(inner) => inner.common.fallback.as_deref_mut(),)*
                Node::Unknown(unknown) => unknown.fallback.as_deref_mut(),
            }
        };
    }
    common_fallback!(
        Layout, Block, Paragraph, List, Table, Tabs, Gauge, LineGauge, Sparkline, BarChart, Chart,
        Scrollbar, Markdown, Code, Diff, Image, Log, Tree, KeyHints, Input, Toggle, Select, Button,
        Spinner, Empty
    )
}

/// Descends into a keyed node member: a `Layout` child by derived key, or a
/// `Tabs` pane by map key.
fn descend_member<'a>(node: &'a mut Node, key: &str) -> Option<&'a mut Node> {
    match node {
        Node::Layout(layout) => {
            let position = layout
                .children
                .iter()
                .enumerate()
                .position(|(index, child)| derived_key(child.common().key, index) == key)?;
            layout.children.get_mut(position)
        }
        Node::Tabs(tabs) => tabs.panes.as_mut()?.get_mut(key),
        _ => None,
    }
}

fn apply_op(root: &mut Node, op: &NodeOp) -> Result<(), PatchError> {
    match op {
        NodeOp::Replace { path, node } => {
            *node_at(root, path)? = node.clone();
        }
        NodeOp::Set { path, props } => set_props(node_at(root, path)?, path, props)?,
        NodeOp::Splice {
            path,
            field,
            index,
            remove,
            insert,
        } => splice_at(node_at(root, path)?, path, *field, *index, *remove, insert)?,
        NodeOp::Append { path, field, items } => {
            append_at(node_at(root, path)?, path, *field, items)?;
        }
        NodeOp::Item {
            path,
            field,
            key,
            value,
        } => replace_item(node_at(root, path)?, path, *field, key, value)?,
    }
    Ok(())
}

/// Shallow prop merge; `null` unsets (§4.7).
fn set_props(
    slot: &mut Node,
    path: &NodePath,
    props: &serde_json::Map<String, Json>,
) -> Result<(), PatchError> {
    let bad = |detail: String| PatchError::BadPatch {
        path: path.clone(),
        detail,
    };
    let mut json = serde_json::to_value(&*slot).map_err(|e| bad(e.to_string()))?;
    let Some(obj) = json.as_object_mut() else {
        return Err(bad("node is not a JSON object".into()));
    };
    for (key, value) in props {
        if value.is_null() {
            obj.remove(key);
        } else {
            obj.insert(key.clone(), value.clone());
        }
    }
    *slot = serde_json::from_value(json).map_err(|e| bad(e.to_string()))?;
    Ok(())
}

/// A plain-item payload: nodes are rejected (they only live in `children` and
/// `panes`).
fn plain_item(value: &PatchItem, field: PatchField, path: &NodePath) -> Result<Json, PatchError> {
    match value {
        PatchItem::Item(json) => Ok(json.clone()),
        PatchItem::Node(_) => Err(PatchError::BadItem {
            path: path.clone(),
            field,
            detail: "a node cannot be a plain item".into(),
        }),
    }
}

/// A node payload (for `children`/`panes`).
fn member_node(value: &PatchItem, field: PatchField, path: &NodePath) -> Result<Node, PatchError> {
    match value {
        PatchItem::Node(node) => Ok(node.clone()),
        PatchItem::Item(json) => {
            serde_json::from_value(json.clone()).map_err(|e| PatchError::BadItem {
                path: path.clone(),
                field,
                detail: e.to_string(),
            })
        }
    }
}

/// A `panes` insert: the pane key comes from the inserted node's `key`.
fn pane_insert(value: &PatchItem, path: &NodePath) -> Result<(Key, Node), PatchError> {
    let node = member_node(value, PatchField::Panes, path)?;
    let Some(key) = node.common().key.map(str::to_owned) else {
        return Err(PatchError::MissingItemKey { path: path.clone() });
    };
    Ok((key, node))
}

/// Deserializes plain-item payloads into `T` and splices `items[start..end]`.
fn splice_seq<T: serde::de::DeserializeOwned>(
    items: &mut Vec<T>,
    field: PatchField,
    path: &NodePath,
    start: usize,
    end: usize,
    insert: &[PatchItem],
) -> Result<(), PatchError> {
    if end > items.len() {
        return Err(PatchError::OutOfBounds {
            path: path.clone(),
            field,
            index: u32::try_from(start).unwrap_or(u32::MAX),
            len: items.len(),
        });
    }
    let inserted = insert
        .iter()
        .map(|value| plain_item(value, field, path))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|json| serde_json::from_value::<T>(json))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| PatchError::BadItem {
            path: path.clone(),
            field,
            detail: e.to_string(),
        })?;
    items.splice(start..end, inserted);
    Ok(())
}

fn splice_at(
    slot: &mut Node,
    path: &NodePath,
    field: PatchField,
    index: u32,
    remove: u32,
    insert: &[PatchItem],
) -> Result<(), PatchError> {
    let Some(end) = index.checked_add(remove) else {
        return Err(PatchError::OutOfBounds {
            path: path.clone(),
            field,
            index,
            len: usize::MAX,
        });
    };
    let (start, end) = (usize::try_from(index).unwrap_or(usize::MAX), end as usize);
    match (slot, field) {
        (Node::Layout(layout), PatchField::Children) => {
            if end > layout.children.len() {
                return Err(PatchError::OutOfBounds {
                    path: path.clone(),
                    field,
                    index,
                    len: layout.children.len(),
                });
            }
            let inserted = insert
                .iter()
                .map(|value| member_node(value, field, path))
                .collect::<Result<Vec<_>, _>>()?;
            layout.children.splice(start..end, inserted);
        }
        (Node::Tabs(tabs), PatchField::Panes) => {
            let map = tabs
                .panes
                .get_or_insert_with(std::collections::BTreeMap::new);
            let mut entries: Vec<(Key, Node)> = map
                .iter()
                .map(|(key, pane)| (key.clone(), pane.clone()))
                .collect();
            if end > entries.len() {
                return Err(PatchError::OutOfBounds {
                    path: path.clone(),
                    field,
                    index,
                    len: entries.len(),
                });
            }
            let inserted = insert
                .iter()
                .map(|value| pane_insert(value, path))
                .collect::<Result<Vec<_>, _>>()?;
            entries.splice(start..end, inserted);
            // `None` and an empty map are the same patch surface (§4.7): keep
            // the canonical shape so `apply(old, ops) == new` holds exactly.
            tabs.panes = if entries.is_empty() {
                None
            } else {
                Some(entries.into_iter().collect())
            };
        }
        (Node::List(list), PatchField::Items) => {
            splice_seq(&mut list.items, field, path, start, end, insert)?;
        }
        (Node::Table(table), PatchField::Rows) => {
            splice_seq(&mut table.rows, field, path, start, end, insert)?;
        }
        (Node::Tree(tree), PatchField::Nodes) => {
            splice_seq(&mut tree.nodes, field, path, start, end, insert)?;
        }
        (Node::Log(log), PatchField::Lines) => {
            splice_seq(&mut log.lines, field, path, start, end, insert)?;
        }
        (Node::Tabs(tabs), PatchField::Titles) => {
            splice_seq(&mut tabs.titles, field, path, start, end, insert)?;
        }
        (Node::Chart(chart), PatchField::Datasets) => {
            splice_seq(&mut chart.datasets, field, path, start, end, insert)?;
        }
        (Node::Sparkline(sparkline), PatchField::Data) => {
            splice_seq(&mut sparkline.data, field, path, start, end, insert)?;
        }
        (Node::BarChart(bars), PatchField::Data) => {
            splice_seq(&mut bars.data, field, path, start, end, insert)?;
        }
        (node, _) => {
            return Err(PatchError::FieldMismatch {
                path: path.clone(),
                field,
                kind: node.kind().to_owned(),
            });
        }
    }
    Ok(())
}

/// Tail append (§4.7: `Log.lines`, `List.items`, `Table.rows`). Ring-trimming
/// past the limit is the consumer's rendering concern (§4.8) — `apply` stores
/// everything so the reference invariant stays exact.
fn append_at(
    slot: &mut Node,
    path: &NodePath,
    field: PatchField,
    items: &[PatchItem],
) -> Result<(), PatchError> {
    let convert = |value: &PatchItem| plain_item(value, field, path);
    let jsons = items.iter().map(convert).collect::<Result<Vec<_>, _>>()?;
    match (slot, field) {
        (Node::List(list), PatchField::Items) => push_items(&mut list.items, jsons, path, field)?,
        (Node::Table(table), PatchField::Rows) => push_items(&mut table.rows, jsons, path, field)?,
        (Node::Log(log), PatchField::Lines) => push_items(&mut log.lines, jsons, path, field)?,
        (node, _) => {
            return Err(PatchError::FieldMismatch {
                path: path.clone(),
                field,
                kind: node.kind().to_owned(),
            });
        }
    }
    Ok(())
}

fn push_items<T: serde::de::DeserializeOwned>(
    items: &mut Vec<T>,
    jsons: Vec<Json>,
    path: &NodePath,
    field: PatchField,
) -> Result<(), PatchError> {
    for json in jsons {
        let item = serde_json::from_value::<T>(json).map_err(|e| PatchError::BadItem {
            path: path.clone(),
            field,
            detail: e.to_string(),
        })?;
        items.push(item);
    }
    Ok(())
}

/// Replaces one keyed member (key sequence unchanged, §4.7). Positional keys
/// `#<index>` resolve by index.
fn replace_item(
    slot: &mut Node,
    path: &NodePath,
    field: PatchField,
    key: &str,
    value: &PatchItem,
) -> Result<(), PatchError> {
    let missing = || PatchError::MissingKey {
        path: path.clone(),
        field,
        key: key.to_owned(),
    };
    match (slot, field) {
        (Node::Layout(layout), PatchField::Children) => {
            let position = layout
                .children
                .iter()
                .enumerate()
                .find_map(|(index, child)| {
                    (derived_key(child.common().key, index) == key).then_some(index)
                })
                .ok_or_else(missing)?;
            layout.children[position] = member_node(value, field, path)?;
        }
        (Node::Tabs(tabs), PatchField::Panes) => {
            let pane = tabs
                .panes
                .as_mut()
                .and_then(|m| m.get_mut(key))
                .ok_or_else(missing)?;
            *pane = member_node(value, field, path)?;
        }
        (Node::List(list), PatchField::Items) => replace_seq(
            &mut list.items,
            field,
            path,
            key,
            value,
            |item| match item {
                crate::widgets::list::ListItem::Full {
                    key: Some(item_key),
                    ..
                } => Some(item_key.as_str()),
                _ => None,
            },
        )?,
        (Node::Table(table), PatchField::Rows) => {
            replace_seq(&mut table.rows, field, path, key, value, |row| {
                row.key.as_deref()
            })?;
        }
        (Node::Tree(tree), PatchField::Nodes) => {
            replace_seq(&mut tree.nodes, field, path, key, value, |n| {
                Some(n.key.as_str())
            })?;
        }
        (Node::Tabs(tabs), PatchField::Titles) => {
            replace_seq(&mut tabs.titles, field, path, key, value, |title| {
                Some(title.key.as_str())
            })?;
        }
        (Node::Log(log), PatchField::Lines) => {
            replace_seq(&mut log.lines, field, path, key, value, |_| None)?;
        }
        (Node::Chart(chart), PatchField::Datasets) => {
            replace_seq(&mut chart.datasets, field, path, key, value, |_| None)?;
        }
        (Node::Sparkline(sparkline), PatchField::Data) => {
            replace_seq(&mut sparkline.data, field, path, key, value, |_| None)?;
        }
        (Node::BarChart(bars), PatchField::Data) => {
            replace_seq(&mut bars.data, field, path, key, value, |_| None)?;
        }
        (node, _) => {
            return Err(PatchError::FieldMismatch {
                path: path.clone(),
                field,
                kind: node.kind().to_owned(),
            });
        }
    }
    Ok(())
}

/// Keyed replace on a plain sequence: match by the extractor's explicit key,
/// else fall back to the positional `#<index>` reading.
fn replace_seq<T: serde::de::DeserializeOwned, F: Fn(&T) -> Option<&str>>(
    items: &mut [T],
    field: PatchField,
    path: &NodePath,
    key: &str,
    value: &PatchItem,
    explicit_key: F,
) -> Result<(), PatchError> {
    let missing = || PatchError::MissingKey {
        path: path.clone(),
        field,
        key: key.to_owned(),
    };
    let position = items
        .iter()
        .enumerate()
        .find_map(|(index, item)| (explicit_key(item) == Some(key)).then_some(index))
        .or_else(|| {
            key.strip_prefix('#')
                .and_then(|index| index.parse::<usize>().ok())
                .filter(|&index| index < items.len())
        })
        .ok_or_else(missing)?;
    let json = plain_item(value, field, path)?;
    items[position] = serde_json::from_value(json).map_err(|e| PatchError::BadItem {
        path: path.clone(),
        field,
        detail: e.to_string(),
    })?;
    Ok(())
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
    use crate::extras::log::Log;

    #[test]
    fn ops_round_trip() {
        let ops = vec![
            NodeOp::Replace {
                path: NodePath::new(),
                node: Log::new(["a"]).into(),
            },
            NodeOp::Set {
                path: NodePath::new(),
                props: serde_json::Map::from_iter([("follow".to_owned(), Json::Bool(false))]),
            },
            NodeOp::Splice {
                path: vec!["main".to_owned()],
                field: PatchField::Items,
                index: 2,
                remove: 1,
                insert: vec![PatchItem::Item(Json::Null)],
            },
            NodeOp::Append {
                path: NodePath::new(),
                field: PatchField::Lines,
                items: vec![PatchItem::Item(Json::String("new".into()))],
            },
            NodeOp::Item {
                path: NodePath::new(),
                field: PatchField::Rows,
                key: "r1".into(),
                value: PatchItem::Item(Json::Null),
            },
        ];
        let json = serde_json::to_string(&ops).unwrap_or_default();
        let back: Vec<NodeOp> = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(ops, back);
    }
}
