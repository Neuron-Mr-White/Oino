//! Patch ops (§4.7). This module holds the wire types; `diff` / `apply` land in
//! WP-U2 (`oino_ui::patch::diff(old, new)` with the `apply(old, diff) == new`
//! invariant).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

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
    Node(crate::node::Node),
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
        node: crate::node::Node,
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
