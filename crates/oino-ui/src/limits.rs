//! Limits and validation (§4.8). Core runs [`validate`] on every producer call
//! (≤ 1 ms for a 256 KiB tree): limits, duplicate sibling keys, action-id syntax,
//! the v1 gate, with soft diagnostics for unknown colour tokens.

use std::collections::BTreeSet;

use crate::extras::image::ImageSource;
use crate::extras::tree::TreeNode;
use crate::layout::Constraint;
use crate::node::Node;
use crate::patch::{NodeOp, NodePath};
use crate::style::{Color, Style};
use crate::text::{Line, Span, Text};
use crate::widgets::block::{Block, BlockTitle};
use crate::widgets::list::ListItem;

/// A validation diagnostic (§4.8). Rejections go to `host-error` plus the
/// session's `extension_error` stream; soft ones (unknown colour tokens) only to
/// the diagnostics stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Human-readable message.
    pub message: String,
    /// Path from the entry root (empty at the root).
    pub path: NodePath,
}

impl Diagnostic {
    /// A diagnostic at the given path.
    #[must_use]
    pub fn at(path: &NodePath, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: path.clone(),
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{} (at /{})", self.message, self.path.join("/"))
        }
    }
}

/// The limits of §4.8 (defaults are the spec values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Encoded tree per entry (§4.8: 256 KiB).
    pub max_tree_bytes: usize,
    /// Encoded patch per update (§4.8: 128 KiB).
    pub max_patch_bytes: usize,
    /// Maximum tree depth (§4.8: 32).
    pub max_depth: usize,
    /// Nodes per tree, items included (§4.8: 5 000).
    pub max_nodes: usize,
    /// `List.items` / `Table.rows` / `Tree` nodes (§4.8: 5 000 each).
    pub max_items: usize,
    /// `Log.maxLines` (§4.8: ≤ 5 000).
    pub max_log_max_lines: u32,
    /// Span content length (§4.8: 64 KiB).
    pub max_span_bytes: usize,
    /// `Markdown.source` length (§4.8: 256 KiB, inside the tree bound).
    pub max_markdown_bytes: usize,
    /// `Image` inline decoded bytes — hard reject (§4.8: 4 MiB).
    pub max_image_decoded_bytes: usize,
    /// `Image` inline decoded bytes above which a diagnostic advises `{blob}` /
    /// `{path}` (§4.8: 1 MiB).
    pub image_blob_advice_bytes: usize,
    /// Total `Chart` points (§4.8: 10 000).
    pub max_chart_points: usize,
    /// `Sparkline` points (§4.8: 1 000).
    pub max_sparkline_points: usize,
    /// Children / constraints per `Layout` (§4.8: 64).
    pub max_layout_children: usize,
    /// Key / action id length (§4.8: 128).
    pub max_key_len: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tree_bytes: 256 * 1024,
            max_patch_bytes: 128 * 1024,
            max_depth: 32,
            max_nodes: 5_000,
            max_items: 5_000,
            max_log_max_lines: 5_000,
            max_span_bytes: 64 * 1024,
            max_markdown_bytes: 256 * 1024,
            max_image_decoded_bytes: 4 * 1024 * 1024,
            image_blob_advice_bytes: 1024 * 1024,
            max_chart_points: 10_000,
            max_sparkline_points: 1_000,
            max_layout_children: 64,
            max_key_len: 128,
        }
    }
}

/// OUS v1 node kinds (v1 §6): a lowercase `t` in this set is rejected by the
/// migration gate with a pointer to the v2 mapping (§4.8, §12.1).
pub const OUS_V1_KINDS: [&str; 30] = [
    "text",
    "markdown",
    "code",
    "diff",
    "image",
    "log",
    "kv",
    "table",
    "list",
    "tree",
    "tabs",
    "split",
    "stack",
    "section",
    "card",
    "badge",
    "meter",
    "gauge",
    "sparkline",
    "barChart",
    "area",
    "spinner",
    "divider",
    "keyHints",
    "input",
    "toggle",
    "select",
    "button",
    "empty",
    "responsive",
];

/// Action id syntax (§4.6).
pub const ACTION_ID_PATTERN: &str = "^[A-Za-z0-9_.:/-]{1,128}$";

/// Rejections and soft diagnostics produced by a validation run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The tree must be rejected (§4.8 `rejected` rows).
    pub rejects: Vec<Diagnostic>,
    /// Advisories: dropped colour tokens, oversized logs, large inline images.
    pub soft: Vec<Diagnostic>,
}

impl Report {
    fn reject(&mut self, path: &NodePath, message: impl Into<String>) {
        self.rejects.push(Diagnostic::at(path, message));
    }

    fn soft(&mut self, path: &NodePath, message: impl Into<String>) {
        self.soft.push(Diagnostic::at(path, message));
    }

    /// True when nothing was found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.rejects.is_empty() && self.soft.is_empty()
    }
}

/// Validates a tree against the §4.8 limits.
///
/// # Errors
/// All rejections (empty vector ⇒ the tree is valid).
pub fn validate(node: &Node) -> Result<(), Vec<Diagnostic>> {
    let report = validate_report(node);
    if report.rejects.is_empty() {
        Ok(())
    } else {
        Err(report.rejects)
    }
}

/// Validates a tree returning the full [`Report`] (rejections + soft diagnostics).
#[must_use]
pub fn validate_report(node: &Node) -> Report {
    validate_with(node, &Limits::default())
}

/// Validates a tree with explicit limits.
#[must_use]
pub fn validate_with(node: &Node, limits: &Limits) -> Report {
    let mut report = Report::default();
    if encoded_size(node) > limits.max_tree_bytes {
        report.reject(
            &NodePath::new(),
            format!(
                "tree exceeds {} KiB encoded (§4.8)",
                limits.max_tree_bytes / 1024
            ),
        );
        return report;
    }
    let mut count = 0_usize;
    let mut path = NodePath::new();
    walk(node, &mut path, limits, &mut count, &mut report);
    report
}

/// Validates patch ops against the §4.8 patch bound.
///
/// # Errors
/// All rejections (empty vector ⇒ the patch is valid).
pub fn validate_patch(ops: &[NodeOp]) -> Result<(), Vec<Diagnostic>> {
    validate_patch_with(ops, &Limits::default())
}

/// Validates patch ops with explicit limits.
///
/// # Errors
/// All rejections (empty vector ⇒ the patch is valid).
pub fn validate_patch_with(ops: &[NodeOp], limits: &Limits) -> Result<(), Vec<Diagnostic>> {
    match serde_json::to_vec(ops) {
        Ok(bytes) if bytes.len() > limits.max_patch_bytes => Err(vec![Diagnostic::at(
            &NodePath::new(),
            format!(
                "patch exceeds {} KiB encoded — send a full tree instead (§4.8)",
                limits.max_patch_bytes / 1024
            ),
        )]),
        Ok(_) => Ok(()),
        Err(e) => Err(vec![Diagnostic::at(
            &NodePath::new(),
            format!("patch is not serializable: {e}"),
        )]),
    }
}

/// Encoded size of a node in bytes (`usize::MAX` when serialization fails).
#[must_use]
pub fn encoded_size(node: &Node) -> usize {
    serde_json::to_vec(node).map_or(usize::MAX, |v| v.len())
}

fn valid_key(key: &str, limits: &Limits) -> bool {
    !key.is_empty() && key.len() <= limits.max_key_len
}

fn valid_action_id(action: &str, limits: &Limits) -> bool {
    !action.is_empty()
        && action.len() <= limits.max_key_len
        && action
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '-'))
}

fn check_key(key: Option<&String>, path: &NodePath, limits: &Limits, report: &mut Report) {
    match key {
        Some(key) if valid_key(key, limits) => {}
        Some(key) => report.reject(
            path,
            format!(
                "key {key:?} must be 1..={} chars (§4.8)",
                limits.max_key_len
            ),
        ),
        None => report.reject(path, "interactive nodes need a key (§4.5.2)"),
    }
}

fn check_action(action: &str, path: &NodePath, limits: &Limits, report: &mut Report) {
    if !valid_action_id(action, limits) {
        report.reject(
            path,
            format!("action id {action:?} must match {ACTION_ID_PATTERN} (§4.6)"),
        );
    }
}

fn check_style(style: &Style, path: &NodePath, report: &mut Report) {
    for (prop, color) in [
        ("fg", style.fg.as_ref()),
        ("bg", style.bg.as_ref()),
        ("underlineColor", style.underline_color.as_ref()),
    ] {
        if let Some(Color::Other(name)) = color {
            report.soft(
                path,
                format!("unknown colour token {name:?} in {prop} dropped (§4.8, §5)"),
            );
        }
    }
}

fn check_style_opt(style: Option<&Style>, path: &NodePath, report: &mut Report) {
    if let Some(style) = style {
        check_style(style, path, report);
    }
}

fn check_span(span: &Span, path: &NodePath, limits: &Limits, report: &mut Report) {
    check_style_opt(span.style(), path, report);
    if span.content().len() > limits.max_span_bytes {
        report.reject(
            path,
            format!("span exceeds {} KiB (§4.8)", limits.max_span_bytes / 1024),
        );
    }
}

fn check_line(line: &Line, path: &NodePath, limits: &Limits, report: &mut Report) {
    check_style_opt(line.style(), path, report);
    for span in line.spans() {
        check_span(&span, path, limits, report);
    }
}

fn check_line_opt(line: Option<&Line>, path: &NodePath, limits: &Limits, report: &mut Report) {
    if let Some(line) = line {
        check_line(line, path, limits, report);
    }
}

fn check_text(text: &Text, path: &NodePath, limits: &Limits, report: &mut Report) {
    check_style_opt(text.style(), path, report);
    for line in text.lines() {
        check_line(&line, path, limits, report);
    }
}

fn check_common(
    common: &crate::node::Common,
    path: &mut NodePath,
    limits: &Limits,
    count: &mut usize,
    report: &mut Report,
) {
    if let Some(key) = &common.key
        && !valid_key(key, limits)
    {
        report.reject(
            path,
            format!(
                "key {key:?} must be 1..={} chars (§4.8)",
                limits.max_key_len
            ),
        );
    }
    check_style_opt(common.style.as_ref(), path, report);
    if let Some(block) = &common.block {
        path.push("block".to_owned());
        check_style_opt(block.style.as_ref(), path, report);
        check_style_opt(block.border_style.as_ref(), path, report);
        check_style_opt(block.title_style.as_ref(), path, report);
        path.pop();
    }
    if let Some(fallback) = &common.fallback {
        path.push("fallback".to_owned());
        walk(fallback, path, limits, count, report);
        path.pop();
    }
}

fn child_key(key: Option<&str>, index: usize) -> String {
    match key {
        Some(key) => key.to_owned(),
        None => format!("#{index}"),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per node kind is the clearest shape for a validator"
)]
fn walk(node: &Node, path: &mut NodePath, limits: &Limits, count: &mut usize, report: &mut Report) {
    *count += 1;
    if *count > limits.max_nodes {
        report.reject(
            path,
            format!("tree exceeds {} nodes (§4.8)", limits.max_nodes),
        );
    }
    if path.len() > limits.max_depth {
        report.reject(
            path,
            format!("tree deeper than {} levels (§4.8)", limits.max_depth),
        );
    }
    match node {
        Node::Layout(l) => {
            check_common(&l.common, path, limits, count, report);
            if l.children.len() > limits.max_layout_children {
                report.reject(
                    path,
                    format!(
                        "layout exceeds {} children (§4.8)",
                        limits.max_layout_children
                    ),
                );
            }
            if l.constraints.len() > limits.max_layout_children {
                report.reject(
                    path,
                    format!(
                        "layout exceeds {} constraints (§4.8)",
                        limits.max_layout_children
                    ),
                );
            }
            for constraint in &l.constraints {
                if let Constraint::Percentage(p) = constraint
                    && *p > 100
                {
                    report.reject(path, format!("percentage {p} out of 0..=100 (§4.3)"));
                }
            }
            for (i, child) in l.children.iter().enumerate() {
                let key = child.common().key.map(str::to_owned);
                path.push(child_key(key.as_deref(), i));
                walk(child, path, limits, count, report);
                path.pop();
            }
        }
        Node::Block(b) => {
            check_block(b, path, limits, count, report);
        }
        Node::Paragraph(p) => {
            check_common(&p.common, path, limits, count, report);
            check_text(&p.text, path, limits, report);
        }
        Node::List(l) => {
            check_common(&l.common, path, limits, count, report);
            check_style_opt(l.highlight_style.as_ref(), path, report);
            if let Some(action) = &l.on_select {
                check_action(action, path, limits, report);
            }
            if let Some(action) = &l.on_activate {
                check_action(action, path, limits, report);
            }
            if l.items.len() > limits.max_items {
                report.reject(
                    path,
                    format!("list exceeds {} items (§4.8)", limits.max_items),
                );
            }
            path.push("items".to_owned());
            let mut seen = BTreeSet::new();
            for (i, item) in l.items.iter().enumerate() {
                *count += 1;
                match item {
                    ListItem::Text(text) => {
                        let key = child_key(None, i);
                        path.push(key);
                        check_text(text, path, limits, report);
                        path.pop();
                    }
                    ListItem::Full {
                        key,
                        content,
                        style,
                        detail,
                        badge,
                        ..
                    } => {
                        let key = child_key(key.as_deref(), i);
                        path.push(key.clone());
                        if let Some(raw) = key_of(item) {
                            if !valid_key(raw, limits) {
                                report.reject(
                                    path,
                                    format!(
                                        "key {raw:?} must be 1..={} chars (§4.8)",
                                        limits.max_key_len
                                    ),
                                );
                            } else if !seen.insert(raw.clone()) {
                                report
                                    .reject(path, format!("duplicate sibling key {raw:?} (§4.8)"));
                            }
                        }
                        check_text(content, path, limits, report);
                        check_style_opt(style.as_ref(), path, report);
                        check_line_opt(detail.as_ref(), path, limits, report);
                        check_line_opt(badge.as_ref(), path, limits, report);
                        path.pop();
                    }
                }
            }
            path.pop();
        }
        Node::Table(t) => {
            check_common(&t.common, path, limits, count, report);
            check_style_opt(t.row_highlight_style.as_ref(), path, report);
            if let Some(action) = &t.on_select {
                check_action(action, path, limits, report);
            }
            if let Some(action) = &t.on_activate {
                check_action(action, path, limits, report);
            }
            if t.rows.len() > limits.max_items {
                report.reject(
                    path,
                    format!("table exceeds {} rows (§4.8)", limits.max_items),
                );
            }
            let mut seen = BTreeSet::new();
            for row in t.header.iter().chain(t.footer.iter()) {
                check_cells(&row.cells, path, limits, count, report);
            }
            path.push("rows".to_owned());
            for (i, row) in t.rows.iter().enumerate() {
                *count += 1;
                let key = child_key(row.key.as_deref(), i);
                path.push(key.clone());
                if let Some(raw) = &row.key {
                    if !valid_key(raw, limits) {
                        report.reject(
                            path,
                            format!(
                                "key {raw:?} must be 1..={} chars (§4.8)",
                                limits.max_key_len
                            ),
                        );
                    } else if !seen.insert(raw.clone()) {
                        report.reject(path, format!("duplicate sibling key {raw:?} (§4.8)"));
                    }
                }
                check_style_opt(row.style.as_ref(), path, report);
                check_cells(&row.cells, path, limits, count, report);
                path.pop();
            }
            path.pop();
        }
        Node::Tabs(t) => {
            check_common(&t.common, path, limits, count, report);
            check_style_opt(t.highlight_style.as_ref(), path, report);
            if let Some(action) = &t.on_select {
                check_action(action, path, limits, report);
            }
            if let Some([first, second]) = &t.padding {
                check_line(first, path, limits, report);
                check_line(second, path, limits, report);
            }
            path.push("titles".to_owned());
            let mut seen = BTreeSet::new();
            for title in &t.titles {
                *count += 1;
                if !valid_key(&title.key, limits) {
                    report.reject(
                        path,
                        format!(
                            "tab key {:?} must be 1..={} chars (§4.8)",
                            title.key, limits.max_key_len
                        ),
                    );
                } else if !seen.insert(title.key.clone()) {
                    report.reject(
                        path,
                        format!("duplicate sibling key {:?} (§4.8)", title.key),
                    );
                }
                check_line(&title.title, path, limits, report);
                check_line_opt(title.badge.as_ref(), path, limits, report);
            }
            path.pop();
            if let Some(panes) = &t.panes {
                for (key, pane) in panes {
                    path.push(key.clone());
                    walk(pane, path, limits, count, report);
                    path.pop();
                }
            }
        }
        Node::Gauge(g) => {
            check_common(&g.common, path, limits, count, report);
            check_style_opt(g.gauge_style.as_ref(), path, report);
            check_span_opt(g.label.as_ref(), path, limits, report);
        }
        Node::LineGauge(g) => {
            check_common(&g.common, path, limits, count, report);
            check_style_opt(g.filled_style.as_ref(), path, report);
            check_style_opt(g.unfilled_style.as_ref(), path, report);
            check_line_opt(g.label.as_ref(), path, limits, report);
        }
        Node::Sparkline(s) => {
            check_common(&s.common, path, limits, count, report);
            if s.data.len() > limits.max_sparkline_points {
                report.reject(
                    path,
                    format!(
                        "sparkline exceeds {} points (§4.8)",
                        limits.max_sparkline_points
                    ),
                );
            }
        }
        Node::BarChart(c) => {
            check_common(&c.common, path, limits, count, report);
            check_style_opt(c.bar_style.as_ref(), path, report);
            check_style_opt(c.value_style.as_ref(), path, report);
            check_style_opt(c.label_style.as_ref(), path, report);
            if let Some(action) = &c.on_select {
                check_action(action, path, limits, report);
            }
            path.push("data".to_owned());
            for group in &c.data {
                *count += 1;
                check_line_opt(group.label.as_ref(), path, limits, report);
                for bar in &group.bars {
                    *count += 1;
                    check_line_opt(bar.label.as_ref(), path, limits, report);
                    check_style_opt(bar.style.as_ref(), path, report);
                    check_style_opt(bar.value_style.as_ref(), path, report);
                }
            }
            path.pop();
        }
        Node::Chart(c) => {
            check_common(&c.common, path, limits, count, report);
            let mut points = 0_usize;
            path.push("datasets".to_owned());
            for dataset in &c.datasets {
                points += dataset.data.len();
                check_line_opt(dataset.name.as_ref(), path, limits, report);
                check_style_opt(dataset.style.as_ref(), path, report);
            }
            path.pop();
            if points > limits.max_chart_points {
                report.reject(
                    path,
                    format!("chart exceeds {} points (§4.8)", limits.max_chart_points),
                );
            }
            for (name, axis) in [("xAxis", &c.x_axis), ("yAxis", &c.y_axis)] {
                path.push(name.to_owned());
                check_line_opt(axis.title.as_ref(), path, limits, report);
                check_style_opt(axis.style.as_ref(), path, report);
                for label in &axis.labels {
                    check_span(label, path, limits, report);
                }
                path.pop();
            }
        }
        Node::Scrollbar(s) => {
            check_common(&s.common, path, limits, count, report);
            check_style_opt(s.thumb_style.as_ref(), path, report);
            check_style_opt(s.track_style.as_ref(), path, report);
        }
        Node::Markdown(m) => {
            check_common(&m.common, path, limits, count, report);
            if m.source.len() > limits.max_markdown_bytes {
                report.reject(
                    path,
                    format!(
                        "markdown source exceeds {} KiB (§4.8)",
                        limits.max_markdown_bytes / 1024
                    ),
                );
            }
        }
        Node::Code(c) => {
            check_common(&c.common, path, limits, count, report);
        }
        Node::Diff(d) => {
            check_common(&d.common, path, limits, count, report);
            if d.patch.is_none() && (d.old.is_none() || d.new.is_none()) {
                report.reject(path, "diff needs `patch` or both `old` and `new` (§4.5.2)");
            }
        }
        Node::Image(i) => {
            check_common(&i.common, path, limits, count, report);
            if let ImageSource::Data { .. } = &i.src {
                let decoded = i.src.decoded_len_estimate();
                if decoded > limits.max_image_decoded_bytes {
                    report.reject(
                        path,
                        format!(
                            "inline image exceeds {} MiB decoded — use {{blob}} / {{path}} (§4.8)",
                            limits.max_image_decoded_bytes / (1024 * 1024)
                        ),
                    );
                } else if decoded > limits.image_blob_advice_bytes {
                    report.soft(
                        path,
                        "inline image over 1 MiB decoded — prefer {blob} / {path} (§4.8)",
                    );
                }
            }
        }
        Node::Log(l) => {
            check_common(&l.common, path, limits, count, report);
            if let Some(max) = l.max_lines {
                if max > limits.max_log_max_lines {
                    report.reject(
                        path,
                        format!("log maxLines exceeds {} (§4.8)", limits.max_log_max_lines),
                    );
                }
                if l.lines.len() > max as usize {
                    report.soft(path, format!("log lines trimmed to maxLines={max} (§4.8)"));
                }
            } else if l.lines.len() > limits.max_items {
                report.soft(
                    path,
                    format!(
                        "log lines exceed {} and are ring-trimmed (§4.8)",
                        limits.max_items
                    ),
                );
            }
            for line in &l.lines {
                check_line(line, path, limits, report);
            }
        }
        Node::Tree(t) => {
            check_common(&t.common, path, limits, count, report);
            if let Some(action) = &t.on_select {
                check_action(action, path, limits, report);
            }
            if let Some(action) = &t.on_activate {
                check_action(action, path, limits, report);
            }
            if let Some(action) = &t.on_toggle {
                check_action(action, path, limits, report);
            }
            if t.nodes.len() > limits.max_items {
                report.reject(
                    path,
                    format!("tree exceeds {} nodes (§4.8)", limits.max_items),
                );
            }
            path.push("nodes".to_owned());
            let mut seen = BTreeSet::new();
            for child in &t.nodes {
                walk_tree_node(child, path, limits, count, report, &mut seen);
            }
            path.pop();
        }
        Node::KeyHints(k) => {
            check_common(&k.common, path, limits, count, report);
            for hint in &k.hints {
                if let Some(action) = &hint.action {
                    path.push(hint.keys.clone());
                    check_action(action, path, limits, report);
                    path.pop();
                }
            }
        }
        Node::Input(i) => {
            check_common(&i.common, path, limits, count, report);
            check_key(i.common.key.as_ref(), path, limits, report);
            check_line_opt(i.label.as_ref(), path, limits, report);
            if let Some(action) = &i.on_submit {
                check_action(action, path, limits, report);
            }
            if let Some(action) = &i.on_change {
                check_action(action, path, limits, report);
            }
        }
        Node::Toggle(t) => {
            check_common(&t.common, path, limits, count, report);
            check_key(t.common.key.as_ref(), path, limits, report);
            check_line(&t.label, path, limits, report);
            if let Some(action) = &t.on_change {
                check_action(action, path, limits, report);
            }
        }
        Node::Select(s) => {
            check_common(&s.common, path, limits, count, report);
            check_key(s.common.key.as_ref(), path, limits, report);
            for option in &s.options {
                check_line(&option.label, path, limits, report);
            }
            if let Some(action) = &s.on_change {
                check_action(action, path, limits, report);
            }
        }
        Node::Button(b) => {
            check_common(&b.common, path, limits, count, report);
            check_line(&b.label, path, limits, report);
            check_action(&b.action, path, limits, report);
        }
        Node::Spinner(s) => {
            check_common(&s.common, path, limits, count, report);
            check_line_opt(s.label.as_ref(), path, limits, report);
        }
        Node::Empty(e) => {
            check_common(&e.common, path, limits, count, report);
            if let Some(button) = &e.action {
                path.push("action".to_owned());
                walk(button, path, limits, count, report);
                path.pop();
            }
        }
        Node::Unknown(u) => {
            if OUS_V1_KINDS.contains(&u.t.as_str()) {
                report.reject(
                    path,
                    format!("OUS v1 node '{}' — see ui-protocol v2 §12.1", u.t),
                );
            }
            if let Some(key) = &u.key
                && !valid_key(key, limits)
            {
                report.reject(
                    path,
                    format!(
                        "key {key:?} must be 1..={} chars (§4.8)",
                        limits.max_key_len
                    ),
                );
            }
            check_style_opt(u.style.as_ref(), path, report);
            if let Some(fallback) = &u.fallback {
                path.push("fallback".to_owned());
                walk(fallback, path, limits, count, report);
                path.pop();
            }
        }
    }
}

fn key_of(item: &ListItem) -> Option<&String> {
    match item {
        ListItem::Text(_) => None,
        ListItem::Full { key, .. } => key.as_ref(),
    }
}

fn check_block(
    b: &Block,
    path: &mut NodePath,
    limits: &Limits,
    count: &mut usize,
    report: &mut Report,
) {
    check_common(&b.common, path, limits, count, report);
    if let Some(action) = &b.on_toggle {
        check_action(action, path, limits, report);
    }
    match &b.title {
        Some(BlockTitle::Line(line)) => check_line(line, path, limits, report),
        Some(BlockTitle::Titles(titles)) => {
            for title in titles {
                check_line(&title.content, path, limits, report);
            }
        }
        None => {}
    }
    if let Some(child) = &b.child {
        path.push("child".to_owned());
        walk(child, path, limits, count, report);
        path.pop();
    }
}

fn check_cells(
    cells: &[crate::widgets::table::Cell],
    path: &mut NodePath,
    limits: &Limits,
    count: &mut usize,
    report: &mut Report,
) {
    for cell in cells {
        check_text(cell.content(), path, limits, report);
        if let crate::widgets::table::Cell::Full { style, .. } = cell {
            check_style_opt(style.as_ref(), path, report);
        }
        *count += 1;
    }
}

fn walk_tree_node(
    node: &TreeNode,
    path: &mut NodePath,
    limits: &Limits,
    count: &mut usize,
    report: &mut Report,
    seen: &mut BTreeSet<String>,
) {
    *count += 1;
    if *count > limits.max_nodes {
        report.reject(
            path,
            format!("tree exceeds {} nodes (§4.8)", limits.max_nodes),
        );
    }
    path.push(node.key.clone());
    if !valid_key(&node.key, limits) {
        report.reject(
            path,
            format!(
                "key {:?} must be 1..={} chars (§4.8)",
                node.key, limits.max_key_len
            ),
        );
    } else if !seen.insert(node.key.clone()) {
        report.reject(path, format!("duplicate sibling key {:?} (§4.8)", node.key));
    }
    check_line(&node.label, path, limits, report);
    check_line_opt(node.badge.as_ref(), path, limits, report);
    if node.children.len() > limits.max_items {
        report.reject(
            path,
            format!("tree exceeds {} nodes (§4.8)", limits.max_items),
        );
    }
    for child in &node.children {
        walk_tree_node(child, path, limits, count, report, seen);
    }
    path.pop();
}

fn check_span_opt(span: Option<&Span>, path: &NodePath, limits: &Limits, report: &mut Report) {
    if let Some(span) = span {
        check_span(span, path, limits, report);
    }
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
    use crate::extras::button::Button;
    use crate::extras::log::Log;
    use crate::extras::spinner::Spinner;
    use crate::layout::Layout;
    use crate::widgets::block::Block;

    #[test]
    fn v1_gate_message() {
        let node: Node = serde_json::from_str(r#"{"t":"text","value":"hello"}"#).expect("parses");
        let err = validate(&node).expect_err("v1 node rejected");
        assert!(
            err[0]
                .message
                .contains("OUS v1 node 'text' — see ui-protocol v2 §12.1")
        );
    }

    #[test]
    fn unknown_but_not_v1_is_accepted() {
        let node: Node =
            serde_json::from_str(r#"{"t":"Holgram","fallback":{"t":"Spinner"}}"#).expect("parses");
        assert!(validate(&node).is_ok(), "unknown kinds render fallback");
    }

    #[test]
    fn duplicate_sibling_keys_rejected() {
        let list = crate::widgets::list::List::new([
            ListItem::new("a").key("one"),
            ListItem::new("b").key("one"),
        ]);
        let err = validate(&Node::List(list)).expect_err("dup keys rejected");
        assert!(err[0].message.contains("duplicate sibling key"));
    }

    #[test]
    fn depth_and_children() {
        let mut deep: Node = Log::new(["leaf"]).into();
        for _ in 0..40 {
            deep = Block::bordered().child(deep).into();
        }
        assert!(validate(&deep).is_err(), "depth 40 rejected");
        let wide: Node = Layout::vertical(std::iter::repeat_n(Constraint::Length(1), 65))
            .children(std::iter::repeat_n(Node::Spinner(Spinner::new("x")), 65))
            .into();
        assert!(validate(&wide).is_err(), "65 children rejected");
    }

    #[test]
    fn action_id_syntax() {
        let bad = Button::new("Go", "open task!");
        let err = validate(&Node::Button(bad)).expect_err("bad action id");
        assert!(err[0].message.contains(ACTION_ID_PATTERN));
        let good = Button::new("Go", "open.task/v2:ok_1");
        assert!(validate(&Node::Button(good)).is_ok());
    }

    #[test]
    fn unknown_colour_is_soft() {
        let node: Node =
            serde_json::from_str(r#"{"t":"Spinner","style":{"fg":"chartreuse"}}"#).expect("parses");
        assert!(validate(&node).is_ok(), "unknown colour is not a rejection");
    }

    #[test]
    fn inline_input_needs_key() {
        let input = crate::extras::input::Input::new("name");
        assert!(validate(&Node::Input(input)).is_ok());
        let raw = r#"{"t":"Input"}"#;
        let node: Node = serde_json::from_str(raw).expect("parses");
        let err = validate(&node).expect_err("input without key rejected");
        assert!(err[0].message.contains("interactive nodes need a key"));
    }
}
