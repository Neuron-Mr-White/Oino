//! Focus order, local UI state and key routing for registry entries (§4.6, D-U13).
//!
//! Frontends own the local state (selection, offset, expansion, collapsed blocks, typed
//! text); a `state`/`value`/`selected`/`expanded`/`collapsed` prop wins when it changes in
//! an update, otherwise the local state persists across updates (§4.6). Interaction is
//! sent back as `oino_ui_action{entryId, actionId, value?, values?, path?}`.

use std::collections::HashMap;

use oino_ai::{JsonObject, JsonValue};
use oino_ui::extras::tree::Tree;
use oino_ui::node::Node;
use oino_ui::slot::UiEntry;
use oino_view::keys::{Key, KeyId, Keybindings};
use serde_json::json;

/// Per-entry local state (§4.6), keyed by the node path inside the entry tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UiLocalState {
    /// Bumped on every local-state change (render cache key).
    pub(crate) epoch: u64,
    /// Focused focusable (index into the depth-first focus order).
    pub(crate) focused: Option<usize>,
    /// List / table / tree / tabs selection.
    pub(crate) selection: HashMap<String, usize>,
    /// Multi-select sets (`List.multi`).
    pub(crate) multi: HashMap<String, Vec<usize>>,
    /// Tree expansion overrides (`Tree` nodes).
    pub(crate) expanded: HashMap<String, bool>,
    /// Collapsed blocks (`Block.collapsible`).
    pub(crate) collapsed: HashMap<String, bool>,
    /// Typed text of keyed inputs.
    pub(crate) input: HashMap<String, String>,
    /// Toggle values overrides.
    pub(crate) toggles: HashMap<String, bool>,
    /// Select values overrides.
    pub(crate) selects: HashMap<String, String>,
    /// Confirmation pending for a `Button.confirm` (`path`).
    pub(crate) confirm: HashMap<String, bool>,
}

impl UiLocalState {
    /// Selection of a collection, or the entry-declared default.
    pub(crate) fn selected(&self, path: &str, default: usize) -> usize {
        self.selection.get(path).copied().unwrap_or(default)
    }

    /// Expansion of a tree node, or the entry-declared default.
    pub(crate) fn expanded(&self, path: &str, default: bool) -> bool {
        self.expanded.get(path).copied().unwrap_or(default)
    }

    /// Collapsed state of a collapsible block.
    pub(crate) fn collapsed(&self, path: &str, default: bool) -> bool {
        self.collapsed.get(path).copied().unwrap_or(default)
    }

    /// Typed value of a keyed input (`None` when untouched).
    pub(crate) fn input(&self, path: &str) -> Option<&str> {
        self.input.get(path).map(String::as_str)
    }

    /// Current value of a keyed toggle.
    pub(crate) fn toggle(&self, path: &str, default: bool) -> bool {
        self.toggles.get(path).copied().unwrap_or(default)
    }

    /// Current value of a keyed select.
    pub(crate) fn select<'a>(&'a self, path: &str, default: Option<&'a str>) -> Option<&'a str> {
        self.selects.get(path).map(String::as_str).or(default)
    }
}

/// One action to deliver to the extension (§4.6).
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "the wire field is `actionId`; keeping `action_id` matches it"
)]
pub(crate) struct Action {
    /// Action id (`$close`, `$key`, … for system actions).
    pub(crate) action_id: String,
    /// Primary value.
    pub(crate) value: Option<JsonValue>,
    /// Current values of every keyed `Input`/`Toggle`/`Select`.
    pub(crate) values: Option<JsonValue>,
    /// Path of the node that fired.
    pub(crate) path: Option<JsonValue>,
}

impl Action {
    fn plain(action_id: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
            value: None,
            values: None,
            path: None,
        }
    }

    fn with_value(action_id: impl Into<String>, value: JsonValue) -> Self {
        Self {
            action_id: action_id.into(),
            value: Some(value),
            values: None,
            path: None,
        }
    }

    fn at(mut self, path: &str) -> Self {
        self.path = Some(JsonValue::Array(
            path.split('/')
                .filter(|segment| !segment.is_empty())
                .map(|segment| JsonValue::String(segment.to_owned()))
                .collect(),
        ));
        self
    }
}

/// One visible tree row (depth-first, expanded nodes only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeRow {
    /// Node path.
    pub(crate) path: String,
    /// Node key.
    pub(crate) key: String,
    /// Depth (0 = root level).
    pub(crate) depth: usize,
    /// Whether the node has children.
    pub(crate) has_children: bool,
    /// Whether the node is expanded (children visible).
    pub(crate) expanded: bool,
}

/// Flattens the visible rows of a tree (§4.5.2, §4.6).
pub(crate) fn tree_rows(tree: &Tree, path: &str, local: &UiLocalState) -> Vec<TreeRow> {
    fn walk(
        nodes: &[oino_ui::extras::tree::TreeNode],
        path: &str,
        depth: usize,
        local: &UiLocalState,
        out: &mut Vec<TreeRow>,
    ) {
        for (index, node) in nodes.iter().enumerate() {
            let key = node.key.clone();
            let child_path = join(path, &key_or(&Some(key.clone()), index));
            let has_children = !node.children.is_empty();
            let expanded =
                has_children && local.expanded(&child_path, node.expanded.unwrap_or(false));
            out.push(TreeRow {
                path: child_path.clone(),
                key,
                depth,
                has_children,
                expanded,
            });
            if expanded {
                walk(&node.children, &child_path, depth + 1, local, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&tree.nodes, path, 0, local, &mut out);
    out
}

/// Positional / keyed segment of a child (§4.1: missing key = `#<index>`).
#[allow(
    clippy::ref_option,
    reason = "callers hold `&Option<Key>` while walking the tree"
)]
pub(crate) fn key_or(key: &Option<String>, index: usize) -> String {
    key.clone().unwrap_or_else(|| format!("#{index}"))
}

/// Joins a parent path and a segment.
pub(crate) fn join(path: &str, segment: &str) -> String {
    if path.is_empty() {
        segment.to_owned()
    } else {
        format!("{path}/{segment}")
    }
}

/// A focusable node (§4.6), in depth-first order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FocusKind {
    /// Collapsible block.
    Block {
        /// `onToggle` action.
        toggle: Option<String>,
    },
    /// List.
    List {
        /// Item keys.
        items: Vec<String>,
        /// `onSelect` action.
        on_select: Option<String>,
        /// `onActivate` action.
        on_activate: Option<String>,
        /// Multiple selection.
        multi: bool,
    },
    /// Table.
    Table {
        /// Row keys.
        rows: Vec<String>,
        /// `onSelect` action.
        on_select: Option<String>,
        /// `onActivate` action.
        on_activate: Option<String>,
    },
    /// Tree.
    Tree {
        /// Visible rows.
        rows: Vec<TreeRow>,
        /// `onSelect` action.
        on_select: Option<String>,
        /// `onActivate` action.
        on_activate: Option<String>,
        /// `onToggle` action.
        on_toggle: Option<String>,
    },
    /// Tabs bar.
    Tabs {
        /// Tab keys.
        tabs: Vec<String>,
        /// `onSelect` action.
        on_select: Option<String>,
    },
    /// Text input.
    Input {
        /// Keyed value name.
        key: Option<String>,
        /// `onSubmit` action.
        on_submit: Option<String>,
    },
    /// Boolean switch.
    Toggle {
        /// Keyed value name.
        key: Option<String>,
        /// `onChange` action.
        on_change: Option<String>,
    },
    /// Cycling select.
    Select {
        /// Keyed value name.
        key: Option<String>,
        /// Option values.
        options: Vec<String>,
        /// `onChange` action.
        on_change: Option<String>,
    },
    /// Button.
    Button {
        /// Action id.
        action: String,
        /// Action value.
        value: Option<JsonValue>,
        /// Inline confirmation text.
        confirm: Option<String>,
    },
    /// A `KeyHints` hint with an action.
    Hint {
        /// Action id.
        action: String,
    },
}

/// One focusable node and its path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FocusTarget {
    /// Node path.
    pub(crate) path: String,
    /// Interaction.
    pub(crate) kind: FocusKind,
}

/// Collects the focusables of an entry tree in render order (§4.6).
pub(crate) fn focusables(entry: &UiEntry, local: &UiLocalState) -> Vec<FocusTarget> {
    let Some(root) = entry_root(entry) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect(root, "", local, &mut out);
    out
}

/// The tree root of an entry (`None` for lines/status/renderer/settings/reserved).
pub(crate) fn entry_root(entry: &UiEntry) -> Option<&Node> {
    match &entry.view {
        oino_ui::slot::UiView::Tree { root } => Some(root),
        oino_ui::slot::UiView::Reserved { fallback, .. } => fallback.as_deref(),
        _ => None,
    }
}

/// Collects focusables (render order).
#[allow(
    clippy::too_many_lines,
    reason = "one arm per node kind, mirroring the renderer"
)]
pub(crate) fn collect(node: &Node, path: &str, local: &UiLocalState, out: &mut Vec<FocusTarget>) {
    match node {
        Node::Layout(layout) => {
            for (index, child) in layout.children.iter().enumerate() {
                let segment = key_or(&child.common().key.map(str::to_owned), index);
                collect(child, &join(path, &segment), local, out);
            }
        }
        Node::Block(block) => {
            if block.collapsible.unwrap_or(false) {
                out.push(FocusTarget {
                    path: path.to_owned(),
                    kind: FocusKind::Block {
                        toggle: block.on_toggle.clone(),
                    },
                });
            }
            if let Some(child) = &block.child
                && !block.collapsed.unwrap_or(false)
            {
                collect(child, &join(path, "child"), local, out);
            }
        }
        Node::List(list) => {
            let items = list_item_keys(list);
            out.push(FocusTarget {
                path: path.to_owned(),
                kind: FocusKind::List {
                    items,
                    on_select: list.on_select.clone(),
                    on_activate: list.on_activate.clone(),
                    multi: list.multi.unwrap_or(false),
                },
            });
        }
        Node::Table(table) => {
            let rows = table_row_keys(table);
            out.push(FocusTarget {
                path: path.to_owned(),
                kind: FocusKind::Table {
                    rows,
                    on_select: table.on_select.clone(),
                    on_activate: table.on_activate.clone(),
                },
            });
        }
        Node::Tree(tree) => {
            out.push(FocusTarget {
                path: path.to_owned(),
                kind: FocusKind::Tree {
                    rows: tree_rows(tree, path, &UiLocalState::default()),
                    on_select: tree.on_select.clone(),
                    on_activate: tree.on_activate.clone(),
                    on_toggle: tree.on_toggle.clone(),
                },
            });
        }
        Node::Tabs(tabs) => {
            let keys = tabs.titles.iter().map(|t| t.key.clone()).collect();
            out.push(FocusTarget {
                path: path.to_owned(),
                kind: FocusKind::Tabs {
                    tabs: keys,
                    on_select: tabs.on_select.clone(),
                },
            });
            if let Some(panes) = &tabs.panes {
                let declared = tabs
                    .selected
                    .as_ref()
                    .and_then(|key| tabs.titles.iter().position(|title| &title.key == key))
                    .unwrap_or(0);
                let selected = local
                    .selected(path, declared)
                    .min(tabs.titles.len().saturating_sub(1));
                if let Some((key, pane)) = tabs
                    .titles
                    .get(selected)
                    .and_then(|title| panes.get_key_value(&title.key))
                {
                    collect(pane, &join(path, key), local, out);
                }
            }
        }
        Node::Input(input) => {
            if !input.disabled.unwrap_or(false) {
                out.push(FocusTarget {
                    path: path.to_owned(),
                    kind: FocusKind::Input {
                        key: input.common.key.clone(),
                        on_submit: input.on_submit.clone(),
                    },
                });
            }
        }
        Node::Toggle(toggle) => {
            if !toggle.disabled.unwrap_or(false) {
                out.push(FocusTarget {
                    path: path.to_owned(),
                    kind: FocusKind::Toggle {
                        key: toggle.common.key.clone(),
                        on_change: toggle.on_change.clone(),
                    },
                });
            }
        }
        Node::Select(select) => {
            if !select.disabled.unwrap_or(false) {
                out.push(FocusTarget {
                    path: path.to_owned(),
                    kind: FocusKind::Select {
                        key: select.common.key.clone(),
                        options: select.options.iter().map(|o| o.value.clone()).collect(),
                        on_change: select.on_change.clone(),
                    },
                });
            }
        }
        Node::Button(button) => {
            if !button.disabled.unwrap_or(false) {
                out.push(FocusTarget {
                    path: path.to_owned(),
                    kind: FocusKind::Button {
                        action: button.action.clone(),
                        value: button.value.clone(),
                        confirm: button.confirm.clone(),
                    },
                });
            }
        }
        Node::KeyHints(hints) => {
            for hint in &hints.hints {
                if let Some(action) = &hint.action {
                    out.push(FocusTarget {
                        path: path.to_owned(),
                        kind: FocusKind::Hint {
                            action: action.clone(),
                        },
                    });
                }
            }
        }
        Node::Empty(empty) => {
            if let Some(action) = &empty.action {
                collect(action, &join(path, "action"), local, out);
            }
        }
        Node::Unknown(unknown) => {
            if let Some(fallback) = &unknown.fallback {
                collect(fallback, &join(path, "fallback"), local, out);
            }
        }
        _ => {}
    }
}

/// Keys of a list's items.
fn list_item_keys(list: &oino_ui::widgets::list::List) -> Vec<String> {
    use oino_ui::widgets::list::ListItem;
    list.items
        .iter()
        .enumerate()
        .map(|(index, item)| match item {
            ListItem::Full { key, .. } => key.clone().unwrap_or_else(|| format!("#{index}")),
            ListItem::Text(_) => format!("#{index}"),
        })
        .collect()
}

/// Keys of a table's rows.
fn table_row_keys(table: &oino_ui::widgets::table::Table) -> Vec<String> {
    table
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| row.key.clone().unwrap_or_else(|| format!("#{index}")))
        .collect()
}

/// Current values of every keyed `Input`/`Toggle`/`Select` in the entry (§4.6).
pub(crate) fn values(entry: &UiEntry, local: &UiLocalState) -> JsonValue {
    let mut out = JsonObject::new();
    if let Some(root) = entry_root(entry) {
        collect_values(root, "", local, &mut out);
    }
    JsonValue::Object(out)
}

fn collect_values(node: &Node, path: &str, local: &UiLocalState, out: &mut JsonObject) {
    match node {
        Node::Layout(layout) => {
            for (index, child) in layout.children.iter().enumerate() {
                let segment = key_or(&child.common().key.map(str::to_owned), index);
                collect_values(child, &join(path, &segment), local, out);
            }
        }
        Node::Block(block) => {
            if let Some(child) = &block.child {
                collect_values(child, &join(path, "child"), local, out);
            }
        }
        Node::Tabs(tabs) => {
            if let Some(panes) = &tabs.panes {
                let selected = tabs
                    .selected
                    .clone()
                    .or_else(|| tabs.titles.first().map(|t| t.key.clone()));
                if let Some(pane) = selected.as_ref().and_then(|key| panes.get(key)) {
                    collect_values(
                        pane,
                        &join(path, selected.unwrap_or_default().as_str()),
                        local,
                        out,
                    );
                }
            }
        }
        Node::Empty(empty) => {
            if let Some(action) = &empty.action {
                collect_values(action, &join(path, "action"), local, out);
            }
        }
        Node::Input(input) => {
            let value = local
                .input(path)
                .map_or_else(|| input.value.clone().unwrap_or_default(), str::to_owned);
            if let Some(key) = &input.common.key {
                out.insert(key.clone(), JsonValue::String(value));
            }
        }
        Node::Toggle(toggle) => {
            if let Some(key) = &toggle.common.key {
                out.insert(
                    key.clone(),
                    JsonValue::Bool(local.toggle(path, toggle.value)),
                );
            }
        }
        Node::Select(select) => {
            if let Some(key) = &select.common.key {
                let value = local
                    .select(path, select.value.as_deref())
                    .map_or(JsonValue::Null, |v| JsonValue::String(v.to_owned()));
                out.insert(key.clone(), value);
            }
        }
        Node::Unknown(unknown) => {
            if let Some(fallback) = &unknown.fallback {
                collect_values(fallback, &join(path, "fallback"), local, out);
            }
        }
        _ => {}
    }
}

/// Key routing inside a focused entry (§4.6): Tab/arrows/Enter/Esc and entry keys.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per focusable kind keeps the routing table readable"
)]
pub(crate) fn key_action(
    entry: &UiEntry,
    local: &mut UiLocalState,
    key: &KeyId,
    keys: &Keybindings,
) -> Vec<Action> {
    local.epoch = local.epoch.wrapping_add(1);
    // Entry `keys` are bound while the entry is focused (§4.6).
    for binding in &entry.keys {
        if binding.key.parse::<KeyId>().is_ok_and(|id| id == *key) {
            let mut action = match &binding.action {
                Some(action) => Action::plain(action.clone()),
                None => Action::with_value("$key", JsonValue::String(binding.key.clone())),
            };
            action.values = Some(values(entry, local));
            return vec![action];
        }
    }
    if key.key == Key::Escape && entry.close_on_escape.unwrap_or(true) {
        return vec![Action::plain("$close")];
    }
    let targets = {
        let mut targets = focusables(entry, local);
        fix_tree_rows(&mut targets, entry, local);
        targets
    };
    if targets.is_empty() {
        return Vec::new();
    }
    let count = targets.len();
    let focused = local.focused.unwrap_or(0).min(count - 1);
    if key.key == Key::Tab {
        local.focused = Some(if key.mods.shift {
            (focused + count - 1) % count
        } else {
            (focused + 1) % count
        });
        return Vec::new();
    }
    let target = targets[focused].clone();
    let path = target.path.clone();
    let mut out = Vec::new();
    match target.kind {
        FocusKind::Block { toggle } => {
            if matches!(key.key, Key::Enter | Key::Space) {
                let collapsed = !local.collapsed(&path, false);
                local.collapsed.insert(path.clone(), collapsed);
                if let Some(action) = toggle {
                    out.push(Action::with_value(action, JsonValue::Bool(collapsed)).at(&path));
                }
            }
        }
        FocusKind::List {
            items,
            on_select,
            on_activate,
            multi,
        } => {
            let len = items.len();
            if len > 0 {
                let current = local.selected(&path, 0).min(len - 1);
                let next = move_selection(key, keys, current, len);
                if let Some(next) = next {
                    local.selection.insert(path.clone(), next);
                    if let Some(action) = &on_select
                        && let Some(item) = items.get(next)
                    {
                        out.push(
                            Action::with_value(action, JsonValue::String(item.clone())).at(&path),
                        );
                    }
                } else if (key.key == Key::Enter || keys.matches("tui.select.confirm", key))
                    && let Some(action) = on_activate.or(on_select)
                    && let Some(item) = items.get(current)
                {
                    out.push(Action::with_value(action, JsonValue::String(item.clone())).at(&path));
                } else if key.key == Key::Space && multi {
                    let entry = local.multi.entry(path.clone()).or_default();
                    if let Some(pos) = entry.iter().position(|i| *i == current) {
                        entry.remove(pos);
                    } else {
                        entry.push(current);
                    }
                }
            }
        }
        FocusKind::Table {
            rows,
            on_select,
            on_activate,
        } => {
            let len = rows.len();
            if len > 0 {
                let current = local.selected(&path, 0).min(len - 1);
                if let Some(next) = move_selection(key, keys, current, len) {
                    local.selection.insert(path.clone(), next);
                    if let Some(action) = &on_select
                        && let Some(row) = rows.get(next)
                    {
                        out.push(
                            Action::with_value(action, JsonValue::String(row.clone())).at(&path),
                        );
                    }
                } else if (key.key == Key::Enter || keys.matches("tui.select.confirm", key))
                    && let Some(action) = on_activate.or(on_select)
                    && let Some(row) = rows.get(current)
                {
                    out.push(Action::with_value(action, JsonValue::String(row.clone())).at(&path));
                }
            }
        }
        FocusKind::Tree {
            rows,
            on_select,
            on_activate,
            on_toggle,
        } => {
            tree_key(
                key,
                keys,
                local,
                &path,
                &rows,
                on_select.as_deref(),
                on_activate.as_deref(),
                on_toggle.as_deref(),
                &mut out,
            );
        }
        FocusKind::Tabs { tabs, on_select } => {
            let len = tabs.len();
            if len > 0 {
                let current = local.selected(&path, 0).min(len - 1);
                let next = match key.key {
                    Key::Left | Key::Up => (current + len - 1) % len,
                    Key::Right | Key::Down => (current + 1) % len,
                    _ => current,
                };
                if next != current {
                    local.selection.insert(path.clone(), next);
                }
                if (next != current
                    || key.key == Key::Enter
                    || keys.matches("tui.select.confirm", key))
                    && let Some(action) = &on_select
                    && let Some(tab) = tabs.get(next)
                {
                    out.push(Action::with_value(action, JsonValue::String(tab.clone())).at(&path));
                }
            }
        }
        FocusKind::Input { key: _, on_submit } => {
            let text = local.input.entry(path.clone()).or_default();
            match key.key {
                Key::Backspace => {
                    text.pop();
                }
                Key::Enter if !key.mods.shift => {
                    let value = text.clone();
                    let mut action = on_submit.map_or_else(
                        || Action::with_value("$key", JsonValue::String(value.clone())),
                        |action| Action::with_value(action, JsonValue::String(value.clone())),
                    );
                    action.values = Some(values(entry, local));
                    out.push(action.at(&path));
                }
                _ => {
                    if let Some(c) = key.printable() {
                        text.push(c);
                    }
                }
            }
        }
        FocusKind::Toggle { key: _, on_change } => {
            let value = match node_at(entry, &path) {
                Some(Node::Toggle(toggle)) => local.toggle(&path, toggle.value),
                _ => false,
            };
            if matches!(key.key, Key::Enter | Key::Space) {
                local.toggles.insert(path.clone(), !value);
                if let Some(action) = on_change {
                    let mut action = Action::with_value(action, JsonValue::Bool(!value));
                    action.values = Some(values(entry, local));
                    out.push(action.at(&path));
                }
            }
        }
        FocusKind::Select {
            key: _,
            options,
            on_change,
        } => {
            let default = match node_at(entry, &path) {
                Some(Node::Select(select)) => select.value.clone(),
                _ => None,
            };
            let current = local.select(&path, default.as_deref()).map(str::to_owned);
            let pos = current
                .as_ref()
                .and_then(|value| options.iter().position(|o| o == value));
            if matches!(key.key, Key::Enter | Key::Space) && !options.is_empty() {
                let next = options[pos.map_or(0, |pos| (pos + 1) % options.len())].clone();
                local.selects.insert(path.clone(), next.clone());
                if let Some(action) = on_change {
                    let mut action = Action::with_value(action, JsonValue::String(next));
                    action.values = Some(values(entry, local));
                    out.push(action.at(&path));
                }
            }
        }
        FocusKind::Button {
            action,
            value,
            confirm,
        } => {
            if matches!(key.key, Key::Enter) || keys.matches("tui.select.confirm", key) {
                if confirm.is_some() && !local.confirm.get(&path).copied().unwrap_or(false) {
                    local.confirm.insert(path.clone(), true);
                } else {
                    local.confirm.remove(&path);
                    let action = Action {
                        action_id: action,
                        value,
                        values: Some(values(entry, local)),
                        path: None,
                    };
                    out.push(action.at(&path));
                }
            }
        }
        FocusKind::Hint { action } => {
            if key.key == Key::Enter {
                out.push(Action::with_value(action, JsonValue::Null).at(&path));
            }
        }
    }
    out
}

/// Recomputes tree rows with the live local expansion state (focusables use the defaults).
fn fix_tree_rows(targets: &mut [FocusTarget], entry: &UiEntry, local: &UiLocalState) {
    for target in targets {
        if let FocusKind::Tree { rows, .. } = &mut target.kind
            && let Some(tree) = find_tree(entry_root(entry), &target.path)
        {
            *rows = tree_rows(tree, &target.path, local);
        }
    }
}

fn find_tree<'a>(node: Option<&'a Node>, path: &str) -> Option<&'a Tree> {
    fn walk<'a>(node: &'a Node, segments: &[&str]) -> Option<&'a Tree> {
        if segments.is_empty() {
            return match node {
                Node::Tree(tree) => Some(tree),
                _ => None,
            };
        }
        let first = segments[0];
        match node {
            Node::Layout(layout) => layout
                .children
                .iter()
                .enumerate()
                .find(|(index, child)| {
                    key_or(&child.common().key.map(str::to_owned), *index) == first
                })
                .and_then(|(_, child)| walk(child, &segments[1..])),
            Node::Block(block) if first == "child" => walk(block.child.as_deref()?, &segments[1..]),
            Node::Empty(empty) if first == "action" => {
                walk(empty.action.as_deref()?, &segments[1..])
            }
            _ => None,
        }
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    walk(node?, &segments)
}

fn node_at<'a>(entry: &'a UiEntry, path: &str) -> Option<&'a Node> {
    fn walk<'a>(node: &'a Node, segments: &[&str]) -> Option<&'a Node> {
        if segments.is_empty() {
            return Some(node);
        }
        let first = segments[0];
        match node {
            Node::Layout(layout) => layout
                .children
                .iter()
                .enumerate()
                .find(|(index, child)| {
                    key_or(&child.common().key.map(str::to_owned), *index) == first
                })
                .and_then(|(_, child)| walk(child, &segments[1..])),
            Node::Block(block) if first == "child" => walk(block.child.as_deref()?, &segments[1..]),
            Node::Empty(empty) if first == "action" => {
                walk(empty.action.as_deref()?, &segments[1..])
            }
            _ => None,
        }
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    walk(entry_root(entry)?, &segments)
}

/// Up/down/page/home/end selection movement; `None` when the key is not one of them.
fn move_selection(key: &KeyId, keys: &Keybindings, current: usize, len: usize) -> Option<usize> {
    let up = key.key == Key::Up || keys.matches("tui.select.up", key);
    let down = key.key == Key::Down || keys.matches("tui.select.down", key);
    Some(if up {
        (current + len - 1) % len
    } else if down {
        (current + 1) % len
    } else {
        match key.key {
            Key::PageUp => current.saturating_sub(10),
            Key::PageDown => (current + 10).min(len - 1),
            Key::Home => 0,
            Key::End => len - 1,
            _ => return None,
        }
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "tree interaction needs the row list and the three actions"
)]
fn tree_key(
    key: &KeyId,
    keys: &Keybindings,
    local: &mut UiLocalState,
    path: &str,
    rows: &[TreeRow],
    on_select: Option<&str>,
    on_activate: Option<&str>,
    on_toggle: Option<&str>,
    out: &mut Vec<Action>,
) {
    if rows.is_empty() {
        return;
    }
    let current = local.selected(path, 0).min(rows.len() - 1);
    let mut selected = current;
    let row = &rows[current];
    match key.key {
        Key::Left => {
            if row.expanded && row.has_children {
                local.expanded.insert(row.path.clone(), false);
                if let Some(action) = on_toggle {
                    out.push(
                        Action::with_value(action, json!({"key": row.key, "expanded": false}))
                            .at(&row.path),
                    );
                }
            }
        }
        Key::Right => {
            if row.has_children && !row.expanded {
                local.expanded.insert(row.path.clone(), true);
                if let Some(action) = on_toggle {
                    out.push(
                        Action::with_value(action, json!({"key": row.key, "expanded": true}))
                            .at(&row.path),
                    );
                }
            }
        }
        Key::Enter | Key::Space => {
            if row.has_children {
                let expanded = !row.expanded;
                local.expanded.insert(row.path.clone(), expanded);
                if let Some(action) = on_toggle {
                    out.push(
                        Action::with_value(action, json!({"key": row.key, "expanded": expanded}))
                            .at(&row.path),
                    );
                }
            } else if let Some(action) = on_activate.or(on_select)
                && let Some(row) = rows.get(current)
            {
                out.push(
                    Action::with_value(action, JsonValue::String(row.key.clone())).at(&row.path),
                );
            }
        }
        _ => {
            if let Some(next) = move_selection(key, keys, current, rows.len()) {
                selected = next;
            } else {
                return;
            }
        }
    }
    if selected != current {
        local.selection.insert(path.to_owned(), selected);
        if let Some(action) = on_select
            && let Some(row) = rows.get(selected)
        {
            out.push(Action::with_value(action, JsonValue::String(row.key.clone())).at(&row.path));
        }
    }
}
