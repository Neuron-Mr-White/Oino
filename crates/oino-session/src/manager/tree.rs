//! Tree view of a session (pi `getTree`), built and dropped without recursion so very deep
//! linear sessions cannot overflow the stack.

use std::collections::HashMap;

use serde::{Serialize, Serializer};

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

impl Serialize for SessionTreeNode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("entry", &self.entry)?;
        map.serialize_entry("children", &self.children)?;
        if let Some(label) = &self.label {
            map.serialize_entry("label", label)?;
        }
        if let Some(timestamp) = &self.label_timestamp {
            map.serialize_entry("labelTimestamp", timestamp)?;
        }
        map.end()
    }
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
