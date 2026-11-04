//! Tree view of a session, built and dropped without recursion so very deep
//! linear sessions cannot overflow the stack. The wire form is flat
//! ([`SessionTreeFlatNode`]): the old nested serialization recursed one level
//! per entry and overflowed small worker stacks on long chats.

use std::collections::HashMap;

use serde::Serialize;

use crate::entry::{EntryId, IsoTimestamp, SessionEntry};

/// One node of [`super::SessionManager::tree`].
#[derive(Clone, Debug, PartialEq)]
pub struct SessionTreeNode {
    /// The entry.
    pub entry: SessionEntry,
    /// Children sorted by entry timestamp (stable).
    pub children: Vec<SessionTreeNode>,
    /// Current label of this entry.
    pub label: Option<String>,
    /// When the label was set.
    pub label_timestamp: Option<IsoTimestamp>,
}

impl Drop for SessionTreeNode {
    fn drop(&mut self) {
        let mut stack = std::mem::take(&mut self.children);
        while let Some(mut node) = stack.pop() {
            stack.append(&mut node.children);
        }
    }
}

/// One node of [`super::SessionManager::tree_flat`]: the wire form of the
/// session tree (`get_tree`, `SessionQuery::Tree`). A flat `{entry, label?}`
/// list in file order — `entry.id` and `entry.parentId` link the nodes, the
/// client nests — so serialization never nests and cannot recurse one frame
/// per entry on a deep chain.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTreeFlatNode {
    /// The entry.
    pub entry: SessionEntry,
    /// Current label of this entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// When the label was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_timestamp: Option<IsoTimestamp>,
}

pub(super) fn build(
    entries: &[SessionEntry],
    labels: &HashMap<EntryId, (String, IsoTimestamp)>,
) -> Vec<SessionTreeNode> {
    let position: HashMap<&EntryId, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (&e.id, i))
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut roots = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match entry
            .parent_id
            .as_ref()
            .filter(|parent| **parent != entry.id)
            .and_then(|parent| position.get(parent))
        {
            Some(&parent) if parent != index => children[parent].push(index),
            _ => roots.push(index),
        }
    }
    let millis: Vec<i64> = entries
        .iter()
        .map(|e| e.timestamp.to_ms().unwrap_or(0))
        .collect();
    for list in &mut children {
        list.sort_by_key(|&index| millis[index]);
    }
    // Post-order without recursion, then assemble bottom-up.
    let mut order = Vec::with_capacity(entries.len());
    let mut stack: Vec<usize> = roots.clone();
    let mut visited = vec![false; entries.len()];
    while let Some(index) = stack.pop() {
        if std::mem::replace(&mut visited[index], true) {
            continue;
        }
        order.push(index);
        stack.extend(children[index].iter().copied());
    }
    let mut built: Vec<Option<SessionTreeNode>> = (0..entries.len()).map(|_| None).collect();
    for &index in order.iter().rev() {
        let entry = &entries[index];
        let kids = children[index]
            .iter()
            .filter_map(|&child| built[child].take())
            .collect();
        let label = labels.get(&entry.id);
        built[index] = Some(SessionTreeNode {
            entry: entry.clone(),
            children: kids,
            label: label.map(|(text, _)| text.clone()),
            label_timestamp: label.map(|(_, ts)| ts.clone()),
        });
    }
    roots
        .into_iter()
        .filter_map(|index| built[index].take())
        .collect()
}

/// The flat wire form of [`build`]: every entry with its current label, in
/// file order. One pass, no nesting, no recursion.
pub(super) fn build_flat(
    entries: &[SessionEntry],
    labels: &HashMap<EntryId, (String, IsoTimestamp)>,
) -> Vec<SessionTreeFlatNode> {
    entries
        .iter()
        .map(|entry| {
            let label = labels.get(&entry.id);
            SessionTreeFlatNode {
                entry: entry.clone(),
                label: label.map(|(text, _)| text.clone()),
                label_timestamp: label.map(|(_, ts)| ts.clone()),
            }
        })
        .collect()
}
