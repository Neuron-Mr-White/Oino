//! Session tree navigator model (pi `tree-selector.ts`).
//!
//! Built from `get_tree` data. Flattening follows pi: linear chains stay at one indent,
//! branch points indent their children, the branch containing the current leaf is listed
//! first, and gutters/connectors (`├─`, `└─`, `│`) are computed per row. Filter modes,
//! search, folding and branch-segment jumps match pi.

use std::collections::{HashMap, HashSet};

use oino_ai::{AgentMessage, AssistantContent, JsonObject, JsonValue, StopReason};
use oino_session::{EntryKind, SessionEntry};

use crate::format::read_line_range;

/// One tree node (pi `SessionTreeNode`).
#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    /// The entry.
    pub entry: SessionEntry,
    /// Children.
    pub children: Vec<TreeNode>,
    /// Current label.
    pub label: Option<String>,
    /// When the label was set (ISO).
    pub label_timestamp: Option<String>,
}

impl TreeNode {
    /// Parses one `get_tree` node.
    #[must_use]
    pub fn from_json(value: &JsonValue) -> Option<Self> {
        // Iterative to survive very deep linear sessions.
        let entry: SessionEntry = serde_json::from_value(value.get("entry")?.clone()).ok()?;
        let mut node = Self {
            entry,
            children: Vec::new(),
            label: value
                .get("label")
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
            label_timestamp: value
                .get("labelTimestamp")
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
        };
        if let Some(children) = value.get("children").and_then(JsonValue::as_array) {
            node.children = children.iter().filter_map(Self::from_json).collect();
        }
        Some(node)
    }
}

/// pi `treeFilterMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterMode {
    /// Hide bookkeeping entries.
    #[default]
    Default,
    /// Default minus tool results.
    NoTools,
    /// User messages only.
    UserOnly,
    /// Labelled entries only.
    LabeledOnly,
    /// Everything.
    All,
}

impl FilterMode {
    const CYCLE: [Self; 5] = [
        Self::Default,
        Self::NoTools,
        Self::UserOnly,
        Self::LabeledOnly,
        Self::All,
    ];

    /// pi status label (`[no-tools]`, …; empty for default).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::NoTools => " [no-tools]",
            Self::UserOnly => " [user]",
            Self::LabeledOnly => " [labeled]",
            Self::All => " [all]",
        }
    }

    /// Parses the `treeFilterMode` setting value.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "no-tools" => Self::NoTools,
            "user-only" => Self::UserOnly,
            "labeled-only" => Self::LabeledOnly,
            "all" => Self::All,
            _ => Self::Default,
        }
    }

    fn step(self, forward: bool) -> Self {
        let index = Self::CYCLE.iter().position(|m| *m == self).unwrap_or(0);
        let len = Self::CYCLE.len();
        Self::CYCLE[if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        }]
    }
}

/// Semantic kind of a row's display text (frontends map to colours).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    /// `user: …` (accent prefix).
    User,
    /// `assistant: …` (success prefix).
    Assistant,
    /// Assistant error text.
    AssistantError,
    /// Muted assistant note (`(aborted)`, `(no content)`).
    AssistantMuted,
    /// Tool call summary (muted).
    Tool,
    /// Dim bookkeeping text.
    Dim,
    /// Custom message (`[type]: …`).
    Custom,
    /// Compaction (border accent).
    Compaction,
    /// Branch summary (warning prefix).
    BranchSummary,
}

/// Display text of a row: optional coloured prefix plus body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayText {
    /// Kind.
    pub kind: DisplayKind,
    /// Prefix (`user: `, `assistant: `, `[branch summary]: `…).
    pub prefix: String,
    /// Body.
    pub body: String,
}

/// One visible row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    /// Entry id.
    pub entry_id: String,
    /// Gutter/connector prefix (`│  ├⊟ `…).
    pub prefix: String,
    /// The node is folded.
    pub folded: bool,
    /// On the root → leaf path.
    pub on_active_path: bool,
    /// Label.
    pub label: Option<String>,
    /// Label timestamp (shown when enabled).
    pub label_timestamp: Option<String>,
    /// Display text.
    pub text: DisplayText,
    /// This is the current leaf.
    pub is_leaf: bool,
}

#[derive(Clone, Debug)]
struct Gutter {
    position: usize,
    show: bool,
}

/// Work item of the flattening walk: node, indent, just branched, show connector, is last,
/// gutters, virtual-root child.
type FlattenItem<'a> = (&'a TreeNode, usize, bool, bool, bool, Vec<Gutter>, bool);

#[derive(Clone, Debug)]
struct Flat {
    entry: SessionEntry,
    label: Option<String>,
    label_timestamp: Option<String>,
    indent: usize,
    show_connector: bool,
    is_last: bool,
    gutters: Vec<Gutter>,
    virtual_root_child: bool,
}

/// The navigator model.
#[derive(Clone, Debug, Default)]
pub struct TreeModel {
    flat: Vec<Flat>,
    leaf_id: Option<String>,
    active_path: HashSet<String>,
    tool_calls: HashMap<String, (String, JsonObject)>,
    multiple_roots: bool,
    /// Visible flat indices.
    visible: Vec<usize>,
    visible_children: HashMap<String, Vec<String>>,
    visible_parent: HashMap<String, Option<String>>,
    folded: HashSet<String>,
    /// Filter mode.
    pub filter: FilterMode,
    /// Search query.
    pub query: String,
    /// Show label timestamps.
    pub show_label_timestamps: bool,
    selected: usize,
    home: Option<String>,
}

fn text_of(content: &oino_ai::UserContentField) -> String {
    content.text("")
}

fn assistant_text(message: &oino_ai::AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|c| match c {
            AssistantContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect()
}

fn normalize(text: &str) -> String {
    text.replace(['\n', '\t'], " ").trim().to_owned()
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

impl TreeModel {
    /// Builds the model from `get_tree` data (`{tree, leafId}`).
    #[must_use]
    pub fn from_json(data: &JsonValue, home: Option<String>) -> Self {
        let roots: Vec<TreeNode> = data
            .get("tree")
            .and_then(JsonValue::as_array)
            .map(|nodes| nodes.iter().filter_map(TreeNode::from_json).collect())
            .unwrap_or_default();
        let leaf = data
            .get("leafId")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        Self::new(&roots, leaf, home)
    }

    /// Builds the model from parsed roots.
    #[must_use]
    pub fn new(roots: &[TreeNode], leaf_id: Option<String>, home: Option<String>) -> Self {
        let mut model = Self {
            leaf_id,
            multiple_roots: roots.len() > 1,
            home,
            ..Self::default()
        };
        model.flatten(roots);
        model.build_active_path();
        model.apply_filter();
        model.select_leaf();
        model
    }

    fn flatten(&mut self, roots: &[TreeNode]) {
        // Post-order "contains active leaf" flags, keyed by entry id.
        let mut contains: HashMap<String, bool> = HashMap::new();
        let mut order: Vec<&TreeNode> = Vec::new();
        let mut stack: Vec<&TreeNode> = roots.iter().collect();
        while let Some(node) = stack.pop() {
            order.push(node);
            stack.extend(node.children.iter());
        }
        for node in order.iter().rev() {
            let id = node.entry.id.as_str();
            let has = self.leaf_id.as_deref() == Some(id)
                || node
                    .children
                    .iter()
                    .any(|c| contains.get(c.entry.id.as_str()).copied().unwrap_or(false));
            contains.insert(id.to_owned(), has);
        }
        let active = |n: &TreeNode| contains.get(n.entry.id.as_str()).copied().unwrap_or(false);
        let multiple = roots.len() > 1;
        let mut ordered_roots: Vec<&TreeNode> = roots.iter().collect();
        ordered_roots.sort_by_key(|n| !active(n));
        let mut work: Vec<FlattenItem<'_>> = Vec::new();
        for (i, root) in ordered_roots.iter().enumerate().rev() {
            let is_last = i == ordered_roots.len() - 1;
            work.push((
                root,
                usize::from(multiple),
                multiple,
                multiple,
                is_last,
                Vec::new(),
                multiple,
            ));
        }
        while let Some((
            node,
            indent,
            just_branched,
            show_connector,
            is_last,
            gutters,
            virtual_child,
        )) = work.pop()
        {
            if let EntryKind::Message {
                message: AgentMessage::Assistant(a),
            } = &node.entry.kind
            {
                for c in &a.content {
                    if let AssistantContent::ToolCall(call) = c {
                        self.tool_calls
                            .insert(call.id.clone(), (call.name.clone(), call.arguments.clone()));
                    }
                }
            }
            self.flat.push(Flat {
                entry: node.entry.clone(),
                label: node.label.clone(),
                label_timestamp: node.label_timestamp.clone(),
                indent,
                show_connector,
                is_last,
                gutters: gutters.clone(),
                virtual_root_child: virtual_child,
            });
            let multiple_children = node.children.len() > 1;
            let mut children: Vec<&TreeNode> = node.children.iter().collect();
            children.sort_by_key(|c| !active(c));
            let child_indent = if multiple_children || (just_branched && indent > 0) {
                indent + 1
            } else {
                indent
            };
            let connector_displayed = show_connector && !virtual_child;
            let display_indent = if self.multiple_roots {
                indent.saturating_sub(1)
            } else {
                indent
            };
            let child_gutters = if connector_displayed {
                let mut g = gutters.clone();
                g.push(Gutter {
                    position: display_indent.saturating_sub(1),
                    show: !is_last,
                });
                g
            } else {
                gutters
            };
            for (i, child) in children.iter().enumerate().rev() {
                work.push((
                    child,
                    child_indent,
                    multiple_children,
                    multiple_children,
                    i == children.len() - 1,
                    child_gutters.clone(),
                    false,
                ));
            }
        }
    }

    fn build_active_path(&mut self) {
        let parents: HashMap<&str, Option<&str>> = self
            .flat
            .iter()
            .map(|f| {
                (
                    f.entry.id.as_str(),
                    f.entry
                        .parent_id
                        .as_ref()
                        .map(oino_session::EntryId::as_str),
                )
            })
            .collect();
        let mut current = self.leaf_id.clone();
        while let Some(id) = current {
            let parent = parents
                .get(id.as_str())
                .copied()
                .flatten()
                .map(str::to_owned);
            self.active_path.insert(id);
            current = parent;
        }
    }

    fn passes(&self, flat: &Flat) -> bool {
        let entry = &flat.entry;
        if matches!(entry.kind, EntryKind::Usage { .. }) {
            return false;
        }
        let is_leaf = self.leaf_id.as_deref() == Some(entry.id.as_str());
        if let EntryKind::Message {
            message: AgentMessage::Assistant(a),
        } = &entry.kind
            && !is_leaf
        {
            let has_text = !assistant_text(a).trim().is_empty();
            let error = !matches!(a.stop_reason, StopReason::Stop | StopReason::ToolUse);
            if !has_text && !error {
                return false;
            }
        }
        let settings_entry = matches!(
            entry.kind,
            EntryKind::Label { .. }
                | EntryKind::ContextEdit { .. }
                | EntryKind::Custom { .. }
                | EntryKind::ModelChange { .. }
                | EntryKind::ThinkingLevelChange { .. }
                | EntryKind::SessionInfo { .. }
        );
        let role = entry.message().map(AgentMessage::role);
        let passes = match self.filter {
            FilterMode::UserOnly => role == Some("user"),
            FilterMode::NoTools => !settings_entry && role != Some("toolResult"),
            FilterMode::LabeledOnly => flat.label.is_some(),
            FilterMode::All => true,
            FilterMode::Default => !settings_entry,
        };
        if !passes {
            return false;
        }
        let tokens: Vec<String> = self
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        if tokens.is_empty() {
            return true;
        }
        let text = self.searchable(flat).to_lowercase();
        tokens.iter().all(|t| text.contains(t.as_str()))
    }

    fn searchable(&self, flat: &Flat) -> String {
        let display = self.display(flat);
        let mut text = format!("{}{}", display.prefix, display.body);
        if let Some(label) = &flat.label {
            text.push(' ');
            text.push_str(label);
        }
        text
    }

    fn apply_filter(&mut self) {
        let previous = self.selected_id();
        let mut visible: Vec<usize> = (0..self.flat.len())
            .filter(|&i| self.passes(&self.flat[i]))
            .collect();
        // Visible parent/children maps over the filtered set.
        let visible_ids: HashSet<&str> = visible
            .iter()
            .map(|&i| self.flat[i].entry.id.as_str())
            .collect();
        let by_id: HashMap<&str, &Flat> =
            self.flat.iter().map(|f| (f.entry.id.as_str(), f)).collect();
        let mut parent_map: HashMap<String, Option<String>> = HashMap::new();
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
        for &i in &visible {
            let id = self.flat[i].entry.id.as_str();
            let mut parent = self.flat[i]
                .entry
                .parent_id
                .as_ref()
                .map(oino_session::EntryId::as_str);
            while let Some(p) = parent {
                if visible_ids.contains(p) {
                    break;
                }
                parent = by_id.get(p).and_then(|f| {
                    f.entry
                        .parent_id
                        .as_ref()
                        .map(oino_session::EntryId::as_str)
                });
            }
            parent_map.insert(id.to_owned(), parent.map(str::to_owned));
            if let Some(p) = parent {
                children_map
                    .entry(p.to_owned())
                    .or_default()
                    .push(id.to_owned());
            }
        }
        if !self.folded.is_empty() {
            let mut hidden: HashSet<String> = HashSet::new();
            visible.retain(|&i| {
                let id = self.flat[i].entry.id.as_str();
                let parent = parent_map.get(id).cloned().flatten();
                let skip = parent
                    .as_ref()
                    .is_some_and(|p| self.folded.contains(p) || hidden.contains(p));
                if skip {
                    hidden.insert(id.to_owned());
                }
                !skip
            });
        }
        self.visible = visible;
        self.visible_parent = parent_map;
        self.visible_children = children_map;
        self.selected = previous
            .and_then(|id| self.nearest_visible(&id))
            .unwrap_or_else(|| self.visible.len().saturating_sub(1));
    }

    fn nearest_visible(&self, id: &str) -> Option<usize> {
        let index: HashMap<&str, usize> = self
            .visible
            .iter()
            .enumerate()
            .map(|(pos, &i)| (self.flat[i].entry.id.as_str(), pos))
            .collect();
        let parents: HashMap<&str, Option<&str>> = self
            .flat
            .iter()
            .map(|f| {
                (
                    f.entry.id.as_str(),
                    f.entry
                        .parent_id
                        .as_ref()
                        .map(oino_session::EntryId::as_str),
                )
            })
            .collect();
        let mut current = Some(id);
        while let Some(c) = current {
            if let Some(pos) = index.get(c) {
                return Some(*pos);
            }
            current = parents.get(c).copied().flatten();
        }
        None
    }

    fn select_leaf(&mut self) {
        if let Some(leaf) = self.leaf_id.clone()
            && let Some(pos) = self.nearest_visible(&leaf)
        {
            self.selected = pos;
        }
    }

    fn selected_id(&self) -> Option<String> {
        self.visible
            .get(self.selected)
            .map(|&i| self.flat[i].entry.id.as_str().to_owned())
    }

    /// Number of visible rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// No visible rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Selected visible index.
    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Selected entry.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&SessionEntry> {
        self.visible
            .get(self.selected)
            .map(|&i| &self.flat[i].entry)
    }

    /// Label of the selected entry.
    #[must_use]
    pub fn selected_label(&self) -> Option<&str> {
        self.visible
            .get(self.selected)
            .and_then(|&i| self.flat[i].label.as_deref())
    }

    /// Moves the selection (wrapping, pi `tui.select.up/down`).
    pub fn move_by(&mut self, delta: isize) {
        let len = self.visible.len();
        if len == 0 {
            return;
        }
        let len_i = isize::try_from(len).unwrap_or(isize::MAX);
        let current = isize::try_from(self.selected).unwrap_or(0);
        let next = (current + delta).rem_euclid(len_i);
        self.selected = usize::try_from(next).unwrap_or(0);
    }

    /// Pages the selection (clamped).
    pub fn page(&mut self, delta: isize) {
        let max = self.visible.len().saturating_sub(1);
        let current = isize::try_from(self.selected).unwrap_or(0);
        let next = (current + delta).clamp(0, isize::try_from(max).unwrap_or(0));
        self.selected = usize::try_from(next).unwrap_or(0);
    }

    fn is_foldable(&self, id: &str) -> bool {
        let Some(children) = self.visible_children.get(id) else {
            return false;
        };
        if children.is_empty() {
            return false;
        }
        match self.visible_parent.get(id).cloned().flatten() {
            None => true,
            Some(parent) => self
                .visible_children
                .get(&parent)
                .is_some_and(|s| s.len() > 1),
        }
    }

    /// pi `app.tree.foldOrUp`.
    pub fn fold_or_up(&mut self) {
        let Some(id) = self.selected_id() else {
            return;
        };
        if self.is_foldable(&id) && !self.folded.contains(&id) {
            self.folded.insert(id);
            self.apply_filter();
        } else {
            self.selected = self.segment_start(false);
        }
    }

    /// pi `app.tree.unfoldOrDown`.
    pub fn unfold_or_down(&mut self) {
        let Some(id) = self.selected_id() else {
            return;
        };
        if self.folded.remove(&id) {
            self.apply_filter();
        } else {
            self.selected = self.segment_start(true);
        }
    }

    fn segment_start(&self, down: bool) -> usize {
        let Some(mut current) = self.selected_id() else {
            return self.selected;
        };
        let index: HashMap<&str, usize> = self
            .visible
            .iter()
            .enumerate()
            .map(|(pos, &i)| (self.flat[i].entry.id.as_str(), pos))
            .collect();
        if down {
            loop {
                let children = self
                    .visible_children
                    .get(&current)
                    .cloned()
                    .unwrap_or_default();
                match children.len() {
                    0 => {
                        return index
                            .get(current.as_str())
                            .copied()
                            .unwrap_or(self.selected);
                    }
                    1 => current.clone_from(&children[0]),
                    _ => {
                        return index
                            .get(children[0].as_str())
                            .copied()
                            .unwrap_or(self.selected);
                    }
                }
            }
        }
        loop {
            let Some(parent) = self.visible_parent.get(&current).cloned().flatten() else {
                return index
                    .get(current.as_str())
                    .copied()
                    .unwrap_or(self.selected);
            };
            let siblings = self.visible_children.get(&parent).map_or(0, Vec::len);
            if siblings > 1 && current != self.selected_id().unwrap_or_default() {
                return index
                    .get(current.as_str())
                    .copied()
                    .unwrap_or(self.selected);
            }
            current = parent;
        }
    }

    /// Sets the filter mode (clears folds).
    pub fn set_filter(&mut self, mode: FilterMode) {
        self.filter = mode;
        self.folded.clear();
        self.apply_filter();
    }

    /// pi toggle semantics: `mode` ↔ default.
    pub fn toggle_filter(&mut self, mode: FilterMode) {
        let next = if self.filter == mode {
            FilterMode::Default
        } else {
            mode
        };
        self.set_filter(next);
    }

    /// Cycles the filter.
    pub fn cycle_filter(&mut self, forward: bool) {
        self.set_filter(self.filter.step(forward));
    }

    /// Replaces the search query.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.folded.clear();
        self.apply_filter();
    }

    /// Updates a label after `_oino/set_label`.
    pub fn set_label(&mut self, entry_id: &str, label: Option<String>, timestamp: Option<String>) {
        if let Some(flat) = self
            .flat
            .iter_mut()
            .find(|f| f.entry.id.as_str() == entry_id)
        {
            flat.label_timestamp = label.as_ref().and(timestamp);
            flat.label = label;
        }
        self.apply_filter();
    }

    /// Status line: `(n/total)` plus filter labels.
    #[must_use]
    pub fn status(&self) -> String {
        let mut labels = self.filter.label().to_owned();
        if self.show_label_timestamps {
            labels.push_str(" [+label time]");
        }
        if self.visible.is_empty() {
            format!("(0/0){labels}")
        } else {
            format!("({}/{}){labels}", self.selected + 1, self.visible.len())
        }
    }

    /// Rows `start..end` of the visible list.
    #[must_use]
    pub fn rows(&self, start: usize, end: usize) -> Vec<TreeRow> {
        let end = end.min(self.visible.len());
        (start.min(end)..end)
            .map(|pos| self.row(&self.flat[self.visible[pos]]))
            .collect()
    }

    /// pi window: centre the selection in `height` rows.
    #[must_use]
    pub fn window(&self, height: usize) -> (usize, usize) {
        let len = self.visible.len();
        let start = self
            .selected
            .saturating_sub(height / 2)
            .min(len.saturating_sub(height));
        (start, (start + height).min(len))
    }

    fn row(&self, flat: &Flat) -> TreeRow {
        let id = flat.entry.id.as_str();
        let display_indent = if self.multiple_roots {
            flat.indent.saturating_sub(1)
        } else {
            flat.indent
        };
        let connector = flat.show_connector && !flat.virtual_root_child;
        let connector_position = connector.then(|| display_indent.saturating_sub(1));
        let folded = self.folded.contains(id);
        let mut prefix = String::new();
        for i in 0..display_indent * 3 {
            let level = i / 3;
            let pos = i % 3;
            if let Some(g) = flat.gutters.iter().find(|g| g.position == level) {
                prefix.push(if pos == 0 && g.show { '│' } else { ' ' });
            } else if connector_position == Some(level) {
                prefix.push(match pos {
                    0 if flat.is_last => '└',
                    0 => '├',
                    1 if folded => '⊞',
                    1 if self.is_foldable(id) => '⊟',
                    1 => '─',
                    _ => ' ',
                });
            } else {
                prefix.push(' ');
            }
        }
        TreeRow {
            entry_id: id.to_owned(),
            prefix,
            folded: folded && !connector,
            on_active_path: self.active_path.contains(id),
            label: flat.label.clone(),
            label_timestamp: if self.show_label_timestamps {
                flat.label_timestamp.clone()
            } else {
                None
            },
            text: self.display(flat),
            is_leaf: self.leaf_id.as_deref() == Some(id),
        }
    }

    fn shorten(&self, path: &str) -> String {
        crate::format::shorten_path(path, self.home.as_deref())
    }

    fn tool_call_text(&self, name: &str, args: &JsonObject) -> String {
        let s = |key: &str| args.get(key).and_then(JsonValue::as_str).unwrap_or("");
        let path = || {
            self.shorten(if s("path").is_empty() {
                s("file_path")
            } else {
                s("path")
            })
        };
        match name {
            "read" => format!(
                "[read: {}{}]",
                path(),
                read_line_range(
                    args.get("offset").and_then(JsonValue::as_u64),
                    args.get("limit").and_then(JsonValue::as_u64)
                )
            ),
            "write" => format!("[write: {}]", path()),
            "edit" => format!("[edit: {}]", path()),
            "bash" => {
                let raw = s("command");
                let cmd = clip(&normalize(raw), 50);
                format!(
                    "[bash: {cmd}{}]",
                    if raw.chars().count() > 50 { "..." } else { "" }
                )
            }
            "grep" => format!(
                "[grep: /{}/ in {}]",
                s("pattern"),
                self.shorten(if s("path").is_empty() { "." } else { s("path") })
            ),
            "find" => format!(
                "[find: {} in {}]",
                s("pattern"),
                self.shorten(if s("path").is_empty() { "." } else { s("path") })
            ),
            "ls" => format!(
                "[ls: {}]",
                self.shorten(if s("path").is_empty() { "." } else { s("path") })
            ),
            other => {
                let json = serde_json::to_string(args).unwrap_or_default();
                format!(
                    "[{other}: {}{}]",
                    clip(&json, 40),
                    if json.chars().count() > 40 { "..." } else { "" }
                )
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per entry type (pi getEntryDisplayText)"
    )]
    fn display(&self, flat: &Flat) -> DisplayText {
        let text = |kind, prefix: &str, body: String| DisplayText {
            kind,
            prefix: prefix.to_owned(),
            body,
        };
        match &flat.entry.kind {
            EntryKind::Message { message } => match message {
                AgentMessage::User(u) => text(
                    DisplayKind::User,
                    "user: ",
                    normalize(&clip(&text_of(&u.content), 200)),
                ),
                AgentMessage::Assistant(a) => {
                    let body = normalize(&clip(&assistant_text(a), 200));
                    if !body.is_empty() {
                        text(DisplayKind::Assistant, "assistant: ", body)
                    } else if a.stop_reason == StopReason::Aborted {
                        text(
                            DisplayKind::AssistantMuted,
                            "assistant: ",
                            "(aborted)".into(),
                        )
                    } else if let Some(error) = &a.error_message {
                        text(
                            DisplayKind::AssistantError,
                            "assistant: ",
                            clip(&normalize(error), 80),
                        )
                    } else {
                        text(
                            DisplayKind::AssistantMuted,
                            "assistant: ",
                            "(no content)".into(),
                        )
                    }
                }
                AgentMessage::ToolResult(r) => {
                    let body = self.tool_calls.get(&r.tool_call_id).map_or_else(
                        || format!("[{}]", r.tool_name),
                        |(name, args)| self.tool_call_text(name, args),
                    );
                    text(DisplayKind::Tool, "", body)
                }
                AgentMessage::BashExecution(b) => text(
                    DisplayKind::Dim,
                    "",
                    format!("[bash]: {}", normalize(&b.command)),
                ),
                other => text(DisplayKind::Dim, "", format!("[{}]", other.role())),
            },
            EntryKind::CustomMessage {
                custom_type,
                content,
                ..
            } => text(
                DisplayKind::Custom,
                &format!("[{custom_type}]: "),
                normalize(&text_of(content)),
            ),
            EntryKind::Compaction(c) => text(
                DisplayKind::Compaction,
                "",
                format!("[compaction: {}k tokens]", (c.tokens_before + 500) / 1000),
            ),
            EntryKind::BranchSummary(b) => text(
                DisplayKind::BranchSummary,
                "[branch summary]: ",
                normalize(&b.summary),
            ),
            EntryKind::ModelChange { model_id, .. } => {
                text(DisplayKind::Dim, "", format!("[model: {model_id}]"))
            }
            EntryKind::ThinkingLevelChange { thinking_level } => text(
                DisplayKind::Dim,
                "",
                format!("[thinking: {thinking_level}]"),
            ),
            EntryKind::Custom { custom_type, .. } => {
                text(DisplayKind::Dim, "", format!("[custom: {custom_type}]"))
            }
            EntryKind::ContextEdit {
                target_id,
                replacement,
            } => text(
                DisplayKind::Dim,
                "",
                format!(
                    "[context {}: {}]",
                    if replacement.is_none() {
                        "omit"
                    } else {
                        "replace"
                    },
                    target_id.as_str()
                ),
            ),
            EntryKind::Label { label, .. } => text(
                DisplayKind::Dim,
                "",
                format!("[label: {}]", label.as_deref().unwrap_or("(cleared)")),
            ),
            EntryKind::SessionInfo { name } => text(
                DisplayKind::Dim,
                "",
                format!("[title: {}]", name.as_deref().unwrap_or("empty")),
            ),
            EntryKind::Usage { .. } => text(DisplayKind::Dim, "", String::new()),
        }
    }

    /// pi `getEntryCopyText` for the selected entry.
    #[must_use]
    pub fn selected_copy_text(&self) -> Option<String> {
        let entry = self.selected_entry()?;
        let text = match &entry.kind {
            EntryKind::Message { message } => match message {
                AgentMessage::BashExecution(b) => b.command.clone(),
                AgentMessage::User(u) => text_of(&u.content),
                AgentMessage::Assistant(a) => {
                    let t = assistant_text(a);
                    if t.is_empty() {
                        a.error_message.clone().unwrap_or_default()
                    } else {
                        t
                    }
                }
                AgentMessage::Custom(c) => text_of(&c.content),
                AgentMessage::ToolResult(r) => r
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        oino_ai::UserContent::Text(t) => Some(t.text.as_str()),
                        oino_ai::UserContent::Image(_) => None,
                    })
                    .collect(),
                _ => String::new(),
            },
            EntryKind::CustomMessage { content, .. } => text_of(content),
            EntryKind::Compaction(c) => c.summary.clone(),
            EntryKind::BranchSummary(b) => b.summary.clone(),
            _ => String::new(),
        };
        (!text.trim().is_empty()).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(id: &str, parent: Option<&str>, text: &str) -> JsonValue {
        json!({"type":"message","id":id,"parentId":parent,"timestamp":"2026-01-01T00:00:00.000Z",
               "message":{"role":"user","content":text,"timestamp":0}})
    }

    fn tree() -> JsonValue {
        // a ─ b ─┬─ c ─ d (leaf)
        //        └─ e
        json!({"leafId":"d","tree":[{"entry":user("a",None,"one"),"children":[
            {"entry":user("b",Some("a"),"two"),"children":[
                {"entry":user("c",Some("b"),"three"),"children":[{"entry":user("d",Some("c"),"four"),"children":[]}]},
                {"entry":user("e",Some("b"),"alt"),"children":[],"label":"keep"}
            ]}
        ]}]})
    }

    #[test]
    fn flattens_like_pi() {
        let model = TreeModel::from_json(&tree(), None);
        let rows = model.rows(0, model.len());
        let ids: Vec<&str> = rows.iter().map(|r| r.entry_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c", "d", "e"]);
        assert_eq!(rows[0].prefix, "");
        assert_eq!(rows[2].prefix, "├⊟ ");
        assert_eq!(rows[3].prefix, "│     ");
        assert_eq!(rows[4].prefix, "└─ ");
        assert!(rows[3].on_active_path && !rows[4].on_active_path);
        assert_eq!(model.selected_entry().unwrap().id.as_str(), "d");
        assert_eq!(rows[4].label.as_deref(), Some("keep"));
        assert_eq!(model.status(), "(4/5)");
    }

    #[test]
    fn filters_search_and_folding() {
        let mut model = TreeModel::from_json(&tree(), None);
        model.toggle_filter(FilterMode::LabeledOnly);
        assert_eq!(model.len(), 1);
        assert_eq!(model.status(), "(1/1) [labeled]");
        model.toggle_filter(FilterMode::LabeledOnly);
        model.set_query("thr");
        assert_eq!(model.len(), 1);
        model.set_query("");
        // Select c and fold it: d disappears.
        while model.selected_entry().unwrap().id.as_str() != "c" {
            model.move_by(-1);
        }
        model.fold_or_up();
        assert_eq!(model.len(), 4);
        model.unfold_or_down();
        assert_eq!(model.len(), 5);
        model.cycle_filter(true);
        assert_eq!(model.filter, FilterMode::NoTools);
    }
}
