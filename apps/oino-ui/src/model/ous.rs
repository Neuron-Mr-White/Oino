//! Oino UI Spec v1 nodes (ui-protocol §6), parsed into a typed tree with the §6.2 limits
//! applied. Unknown kinds keep their `fallback` node; interactive nodes carry the action id
//! the frontend reports back through `extension_ui_action`.

use oino_ai::JsonValue;

/// Maximum nesting depth (§6.2).
pub const MAX_DEPTH: usize = 32;
/// Maximum number of nodes (§6.2).
pub const MAX_NODES: usize = 10_000;
/// Maximum `log.lines` (§6.2).
pub const MAX_LOG_LINES: usize = 5_000;

/// Semantic tone (§6.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// Default text.
    #[default]
    Default,
    /// Muted.
    Muted,
    /// Accent.
    Accent,
    /// Info.
    Info,
    /// Success.
    Success,
    /// Warning.
    Warning,
    /// Error.
    Error,
    /// Harness provenance (violet rail).
    Harness,
    /// Thinking.
    Thinking,
    /// Tool.
    Tool,
    /// User.
    User,
    /// Assistant.
    Assistant,
    /// Categorical palette `c1`…`c12` (index 0–11).
    Palette(u8),
}

impl Tone {
    /// Parses a tone name; unknown names are `Default`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "muted" => Self::Muted,
            "accent" => Self::Accent,
            "info" => Self::Info,
            "success" => Self::Success,
            "warning" => Self::Warning,
            "error" => Self::Error,
            "harness" => Self::Harness,
            "thinking" => Self::Thinking,
            "tool" => Self::Tool,
            "user" => Self::User,
            "assistant" => Self::Assistant,
            other => other
                .strip_prefix('c')
                .and_then(|n| n.parse::<u8>().ok())
                .filter(|n| (1..=12).contains(n))
                .map_or(Self::Default, |n| Self::Palette(n - 1)),
        }
    }

    fn of(value: Option<&JsonValue>) -> Self {
        value
            .and_then(JsonValue::as_str)
            .map_or(Self::Default, Self::parse)
    }
}

/// A styled run of text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag per OUS span style (ui-protocol §6.1)"
)]
pub struct Span {
    /// Text.
    pub text: String,
    /// Tone.
    pub tone: Tone,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Dim.
    pub dim: bool,
    /// Monospace.
    pub code: bool,
    /// Link target.
    pub link: Option<String>,
}

/// `Rich = string | Span[]`.
pub type Rich = Vec<Span>;

/// Parses a `Rich` value.
#[must_use]
pub fn rich(value: Option<&JsonValue>) -> Rich {
    match value {
        Some(JsonValue::String(text)) => vec![Span {
            text: text.clone(),
            ..Span::default()
        }],
        Some(JsonValue::Array(spans)) => spans
            .iter()
            .filter_map(|span| {
                if let Some(text) = span.as_str() {
                    return Some(Span {
                        text: text.to_owned(),
                        ..Span::default()
                    });
                }
                let flag = |key: &str| span.get(key).and_then(JsonValue::as_bool) == Some(true);
                Some(Span {
                    text: span.get("text")?.as_str()?.to_owned(),
                    tone: Tone::of(span.get("tone")),
                    bold: flag("bold"),
                    italic: flag("italic"),
                    dim: flag("dim"),
                    code: flag("code"),
                    link: span
                        .get("link")
                        .and_then(JsonValue::as_str)
                        .map(str::to_owned),
                })
            })
            .collect(),
        Some(JsonValue::Number(n)) => vec![Span {
            text: n.to_string(),
            ..Span::default()
        }],
        _ => Vec::new(),
    }
}

/// Plain text of a `Rich` value.
#[must_use]
pub fn plain(rich: &Rich) -> String {
    rich.iter().map(|s| s.text.as_str()).collect()
}

/// A selectable item (`list`).
#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// Item id (action value).
    pub id: String,
    /// Label.
    pub label: Rich,
    /// Detail line.
    pub detail: Option<Rich>,
    /// Badge.
    pub badge: Option<Rich>,
    /// Tone.
    pub tone: Tone,
}

/// A tree node.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeItem {
    /// Node id.
    pub id: String,
    /// Label.
    pub label: Rich,
    /// Children.
    pub children: Vec<TreeItem>,
    /// Expanded.
    pub expanded: bool,
}

/// A labelled bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    /// Label.
    pub label: String,
    /// Value.
    pub value: f64,
    /// Tone.
    pub tone: Tone,
}

/// A key-hint chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyHint {
    /// Keys (`ctrl+r`).
    pub keys: String,
    /// Label.
    pub label: String,
}

/// One OUS node.
#[derive(Clone, Debug, PartialEq)]
#[allow(
    missing_docs,
    reason = "fields mirror the ui-protocol §6.1 table one to one"
)]
pub enum OusNode {
    Text {
        value: Rich,
        wrap: bool,
    },
    Markdown {
        value: String,
    },
    Code {
        value: String,
        language: Option<String>,
    },
    Diff {
        patch: String,
        path: Option<String>,
    },
    Image {
        alt: String,
    },
    Log {
        lines: Vec<String>,
        tail: bool,
        max_lines: Option<usize>,
    },
    Kv {
        items: Vec<(Rich, Rich)>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<Rich>>,
        selectable: Option<String>,
    },
    List {
        items: Vec<ListItem>,
        selectable: Option<String>,
        selected: Option<String>,
    },
    Tree {
        nodes: Vec<TreeItem>,
        selectable: Option<String>,
    },
    Tabs {
        tabs: Vec<(String, String, OusNode)>,
        active: Option<String>,
        on_change: Option<String>,
    },
    Split {
        horizontal: bool,
        children: Vec<OusNode>,
    },
    Stack {
        horizontal: bool,
        gap: Option<f32>,
        children: Vec<OusNode>,
    },
    Section {
        title: Rich,
        body: Box<OusNode>,
        collapsible: bool,
        collapsed: bool,
    },
    Card {
        title: Option<Rich>,
        rail: Option<Tone>,
        source: Option<String>,
        body: Box<OusNode>,
    },
    Badge {
        value: Rich,
        tone: Tone,
    },
    Meter {
        value: f64,
        max: f64,
        label: Option<String>,
        tone: Tone,
    },
    Gauge {
        value: f64,
        label: Option<String>,
    },
    Sparkline {
        values: Vec<f64>,
        label: Option<String>,
    },
    BarChart {
        bars: Vec<Bar>,
    },
    Area {
        series: Vec<(String, Vec<f64>)>,
    },
    Spinner {
        label: Option<String>,
    },
    Divider {
        label: Option<String>,
    },
    KeyHints {
        hints: Vec<KeyHint>,
    },
    Input {
        id: String,
        value: String,
        placeholder: Option<String>,
        on_submit: String,
        multiline: bool,
    },
    Toggle {
        id: String,
        label: String,
        value: bool,
        on_change: String,
    },
    Select {
        id: String,
        options: Vec<(String, String)>,
        value: Option<String>,
        on_change: String,
    },
    Button {
        label: String,
        action_id: String,
        tone: Tone,
        primary: bool,
    },
    Empty {
        message: Option<String>,
    },
}

struct Budget {
    nodes: usize,
}

fn st(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn flag(value: &JsonValue, key: &str, default: bool) -> bool {
    value
        .get(key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(default)
}

fn num(value: &JsonValue, key: &str) -> Option<f64> {
    value.get(key).and_then(JsonValue::as_f64)
}

fn nums(value: Option<&JsonValue>) -> Vec<f64> {
    value
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_f64)
        .collect()
}

fn array<'a>(value: &'a JsonValue, key: &str) -> impl Iterator<Item = &'a JsonValue> {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
}

fn horizontal(value: &JsonValue) -> bool {
    value.get("direction").and_then(JsonValue::as_str) == Some("horizontal")
}

fn tree_items(value: &JsonValue, depth: usize, budget: &mut Budget) -> Vec<TreeItem> {
    if depth >= MAX_DEPTH {
        return Vec::new();
    }
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            if budget.nodes >= MAX_NODES {
                return None;
            }
            budget.nodes += 1;
            Some(TreeItem {
                id: st(node, "id")?,
                label: rich(node.get("label")),
                children: node
                    .get("children")
                    .map(|c| tree_items(c, depth + 1, budget))
                    .unwrap_or_default(),
                expanded: flag(node, "expanded", false),
            })
        })
        .collect()
}

impl OusNode {
    /// Parses a node tree; malformed input becomes `Empty`, never an error.
    #[must_use]
    pub fn parse(value: &JsonValue) -> Self {
        Self::parse_at(value, 0, &mut Budget { nodes: 0 })
    }

    fn children(value: &JsonValue, key: &str, depth: usize, budget: &mut Budget) -> Vec<Self> {
        array(value, key)
            .map(|child| Self::parse_at(child, depth + 1, budget))
            .filter(|n| !matches!(n, Self::Empty { message: None }))
            .collect()
    }

    fn child(value: &JsonValue, key: &str, depth: usize, budget: &mut Budget) -> Self {
        value.get(key).map_or(Self::Empty { message: None }, |c| {
            Self::parse_at(c, depth + 1, budget)
        })
    }

    #[allow(clippy::too_many_lines, reason = "one arm per node kind (§6.1 table)")]
    fn parse_at(value: &JsonValue, depth: usize, budget: &mut Budget) -> Self {
        let empty = Self::Empty { message: None };
        if depth >= MAX_DEPTH || budget.nodes >= MAX_NODES {
            return empty;
        }
        budget.nodes += 1;
        if let Some(text) = value.as_str() {
            return Self::Text {
                value: rich(Some(value)),
                wrap: !text.is_empty(),
            };
        }
        let Some(kind) = value.get("t").and_then(JsonValue::as_str) else {
            return empty;
        };
        let d = depth;
        match kind {
            "text" => Self::Text {
                value: rich(value.get("value")),
                wrap: flag(value, "wrap", true),
            },
            "markdown" => Self::Markdown {
                value: st(value, "value").unwrap_or_default(),
            },
            "code" => Self::Code {
                value: st(value, "value").unwrap_or_default(),
                language: st(value, "language"),
            },
            "diff" => {
                let patch = st(value, "patch").unwrap_or_else(|| {
                    let old = st(value, "oldText").unwrap_or_default();
                    let new = st(value, "newText").unwrap_or_default();
                    naive_patch(&old, &new)
                });
                Self::Diff {
                    patch,
                    path: st(value, "path"),
                }
            }
            "image" => Self::Image {
                alt: st(value, "alt").unwrap_or_else(|| "image".to_owned()),
            },
            "log" => {
                let mut lines: Vec<String> = array(value, "lines")
                    .filter_map(JsonValue::as_str)
                    .map(str::to_owned)
                    .collect();
                if lines.len() > MAX_LOG_LINES {
                    lines.drain(..lines.len() - MAX_LOG_LINES);
                }
                Self::Log {
                    lines,
                    tail: flag(value, "tail", false),
                    max_lines: value
                        .get("maxLines")
                        .and_then(JsonValue::as_u64)
                        .and_then(|n| usize::try_from(n).ok()),
                }
            }
            "kv" => Self::Kv {
                items: array(value, "items")
                    .map(|i| (rich(i.get("key")), rich(i.get("value"))))
                    .collect(),
            },
            "table" => Self::Table {
                columns: array(value, "columns")
                    .map(|c| st(c, "title").unwrap_or_default())
                    .collect(),
                rows: array(value, "rows")
                    .map(|r| {
                        r.as_array()
                            .into_iter()
                            .flatten()
                            .map(|cell| rich(Some(cell)))
                            .collect()
                    })
                    .collect(),
                selectable: st(value, "selectable"),
            },
            "list" => Self::List {
                items: array(value, "items")
                    .filter_map(|i| {
                        Some(ListItem {
                            id: st(i, "id")?,
                            label: rich(i.get("label")),
                            detail: i.get("detail").map(|v| rich(Some(v))),
                            badge: i.get("badge").map(|v| rich(Some(v))),
                            tone: Tone::of(i.get("tone")),
                        })
                    })
                    .collect(),
                selectable: st(value, "selectable"),
                selected: st(value, "selected"),
            },
            "tree" => Self::Tree {
                nodes: value
                    .get("nodes")
                    .map(|n| tree_items(n, d + 1, budget))
                    .unwrap_or_default(),
                selectable: st(value, "selectable"),
            },
            "tabs" => Self::Tabs {
                tabs: array(value, "tabs")
                    .filter_map(|tab| {
                        Some((
                            st(tab, "id")?,
                            st(tab, "title").unwrap_or_default(),
                            Self::child(tab, "body", d, budget),
                        ))
                    })
                    .collect(),
                active: st(value, "active"),
                on_change: st(value, "onChange"),
            },
            "split" => Self::Split {
                horizontal: horizontal(value),
                children: Self::children(value, "children", d, budget),
            },
            "stack" => Self::Stack {
                horizontal: horizontal(value),
                #[allow(clippy::cast_possible_truncation, reason = "pixel gap")]
                gap: num(value, "gap").map(|g| g as f32),
                children: Self::children(value, "children", d, budget),
            },
            "section" => Self::Section {
                title: rich(value.get("title")),
                body: Box::new(Self::child(value, "body", d, budget)),
                collapsible: flag(value, "collapsible", false),
                collapsed: flag(value, "collapsed", false),
            },
            "card" => Self::Card {
                title: value.get("title").map(|t| rich(Some(t))),
                rail: value.get("rail").map(|r| Tone::of(Some(r))),
                source: st(value, "source"),
                body: Box::new(Self::child(value, "body", d, budget)),
            },
            "badge" => Self::Badge {
                value: rich(value.get("value")),
                tone: Tone::of(value.get("tone")),
            },
            "meter" => Self::Meter {
                value: num(value, "value").unwrap_or(0.0),
                max: num(value, "max").unwrap_or(1.0),
                label: st(value, "label"),
                tone: Tone::of(value.get("tone")),
            },
            "gauge" => Self::Gauge {
                value: num(value, "value").unwrap_or(0.0).clamp(0.0, 1.0),
                label: st(value, "label"),
            },
            "sparkline" => Self::Sparkline {
                values: nums(value.get("values")),
                label: st(value, "label"),
            },
            "barChart" => Self::BarChart {
                bars: array(value, "bars")
                    .map(|b| Bar {
                        label: st(b, "label").unwrap_or_default(),
                        value: num(b, "value").unwrap_or(0.0),
                        tone: Tone::of(b.get("tone")),
                    })
                    .collect(),
            },
            "area" => Self::Area {
                series: array(value, "series")
                    .map(|s| (st(s, "label").unwrap_or_default(), nums(s.get("values"))))
                    .collect(),
            },
            "spinner" => Self::Spinner {
                label: st(value, "label"),
            },
            "divider" => Self::Divider {
                label: st(value, "label"),
            },
            "keyHints" => Self::KeyHints {
                hints: array(value, "hints")
                    .map(|h| KeyHint {
                        keys: st(h, "keys").unwrap_or_default(),
                        label: st(h, "label").unwrap_or_default(),
                    })
                    .collect(),
            },
            "input" => Self::Input {
                id: st(value, "id").unwrap_or_default(),
                value: st(value, "value").unwrap_or_default(),
                placeholder: st(value, "placeholder"),
                on_submit: st(value, "onSubmit").unwrap_or_default(),
                multiline: flag(value, "multiline", false),
            },
            "toggle" => Self::Toggle {
                id: st(value, "id").unwrap_or_default(),
                label: st(value, "label").unwrap_or_default(),
                value: flag(value, "value", false),
                on_change: st(value, "onChange").unwrap_or_default(),
            },
            "select" => Self::Select {
                id: st(value, "id").unwrap_or_default(),
                options: array(value, "options")
                    .filter_map(|o| {
                        let v = st(o, "value")?;
                        Some((v.clone(), st(o, "label").unwrap_or(v)))
                    })
                    .collect(),
                value: st(value, "value"),
                on_change: st(value, "onChange").unwrap_or_default(),
            },
            "button" => Self::Button {
                label: st(value, "label").unwrap_or_default(),
                action_id: st(value, "actionId").unwrap_or_default(),
                tone: Tone::of(value.get("tone")),
                primary: flag(value, "primary", false),
            },
            "empty" => Self::Empty {
                message: st(value, "message"),
            },
            _ => value
                .get("fallback")
                .map_or(empty, |f| Self::parse_at(f, d + 1, budget)),
        }
    }

    /// Plain-text rendering used for degraded surfaces (status tooltips, footer widgets) —
    /// one line per block, like `OusNode::to_lines` in ui-protocol §10.
    #[must_use]
    pub fn to_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.lines_into(&mut out);
        out
    }

    fn lines_into(&self, out: &mut Vec<String>) {
        match self {
            Self::Text { value, .. } | Self::Badge { value, .. } => out.push(plain(value)),
            Self::Markdown { value } | Self::Code { value, .. } => {
                out.extend(value.lines().map(str::to_owned));
            }
            Self::Diff { patch, .. } => out.extend(patch.lines().map(str::to_owned)),
            Self::Image { alt } => out.push(format!("[{alt}]")),
            Self::Log { lines, .. } => out.extend(lines.iter().cloned()),
            Self::Kv { items } => {
                out.extend(
                    items
                        .iter()
                        .map(|(k, v)| format!("{}: {}", plain(k), plain(v))),
                );
            }
            Self::Table { rows, .. } => out.extend(
                rows.iter()
                    .map(|r| r.iter().map(plain).collect::<Vec<_>>().join("  ")),
            ),
            Self::List { items, .. } => out.extend(items.iter().map(|i| plain(&i.label))),
            Self::Tree { nodes, .. } => out.extend(nodes.iter().map(|n| plain(&n.label))),
            Self::Tabs { tabs, .. } => {
                if let Some((_, _, body)) = tabs.first() {
                    body.lines_into(out);
                }
            }
            Self::Split { children, .. } | Self::Stack { children, .. } => {
                for child in children {
                    child.lines_into(out);
                }
            }
            Self::Section { title, body, .. } => {
                out.push(plain(title));
                body.lines_into(out);
            }
            Self::Card { title, body, .. } => {
                if let Some(title) = title {
                    out.push(plain(title));
                }
                body.lines_into(out);
            }
            Self::Meter {
                value, max, label, ..
            } => out.push(format!("{}{value}/{max}", label_prefix(label.as_deref()))),
            Self::Gauge { value, label } => out.push(format!(
                "{}{:.0}%",
                label_prefix(label.as_deref()),
                value * 100.0
            )),
            Self::Spinner { label } | Self::Divider { label } => {
                out.push(label.clone().unwrap_or_default());
            }
            Self::KeyHints { hints } => out.push(
                hints
                    .iter()
                    .map(|h| format!("{} {}", h.keys, h.label))
                    .collect::<Vec<_>>()
                    .join("  "),
            ),
            Self::Button { label, .. } => out.push(format!("[ {label} ]")),
            Self::Empty { message } => out.extend(message.clone()),
            Self::Sparkline { label, .. } => out.extend(label.clone()),
            Self::BarChart { bars } => {
                out.extend(bars.iter().map(|b| format!("{}: {}", b.label, b.value)));
            }
            Self::Area { .. } | Self::Input { .. } | Self::Toggle { .. } | Self::Select { .. } => {}
        }
    }
}

fn label_prefix(label: Option<&str>) -> String {
    label.map(|l| format!("{l} ")).unwrap_or_default()
}

/// A whole-text replacement patch for `{oldText, newText}` diffs (good enough for previews;
/// real tool diffs carry a computed patch).
fn naive_patch(old: &str, new: &str) -> String {
    let mut out = format!(
        "@@ -1,{} +1,{} @@\n",
        old.lines().count(),
        new.lines().count()
    );
    for line in old.lines() {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in new.lines() {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_nested_layout_and_rich_text() {
        let node = OusNode::parse(&json!({
            "t":"card","title":"Info","rail":"harness","source":"workflow",
            "body":{"t":"stack","direction":"vertical","children":[
                {"t":"text","value":[{"text":"ok","tone":"success","bold":true}," plain"]},
                {"t":"kv","items":[{"key":"Model","value":"gpt"}]},
                {"t":"meter","value":3,"max":4,"label":"ctx","tone":"c3"},
                {"t":"nope"},
                {"t":"nope","fallback":{"t":"text","value":"fb"}}
            ]}
        }));
        let OusNode::Card {
            rail, source, body, ..
        } = &node
        else {
            panic!()
        };
        assert_eq!(*rail, Some(Tone::Harness));
        assert_eq!(source.as_deref(), Some("workflow"));
        let OusNode::Stack { children, .. } = body.as_ref() else {
            panic!()
        };
        assert_eq!(children.len(), 4, "unknown without fallback is dropped");
        let OusNode::Text { value, .. } = &children[0] else {
            panic!()
        };
        assert_eq!(value[0].tone, Tone::Success);
        assert!(value[0].bold);
        assert_eq!(plain(value), "ok plain");
        assert!(matches!(
            children[2],
            OusNode::Meter {
                tone: Tone::Palette(2),
                ..
            }
        ));
        assert_eq!(
            node.to_lines(),
            vec!["Info", "ok plain", "Model: gpt", "ctx 3/4", "fb"]
        );
    }

    #[test]
    fn interactive_nodes_keep_action_ids() {
        let node = OusNode::parse(&json!({"t":"list","selectable":"pick","items":[
            {"id":"a","label":"A","badge":"new"},{"label":"no id"}]}));
        let OusNode::List {
            items, selectable, ..
        } = node
        else {
            panic!()
        };
        assert_eq!(items.len(), 1);
        assert_eq!(selectable.as_deref(), Some("pick"));
        let button =
            OusNode::parse(&json!({"t":"button","label":"Go","actionId":"go","primary":true}));
        assert!(
            matches!(button, OusNode::Button { primary: true, ref action_id, .. } if action_id == "go")
        );
    }

    #[test]
    fn limits_depth_and_log_lines() {
        let mut deep = json!({"t":"text","value":"leaf"});
        for _ in 0..40 {
            deep = json!({"t":"section","title":"s","body":deep});
        }
        let lines = OusNode::parse(&deep).to_lines();
        assert!(!lines.contains(&"leaf".to_owned()));
        let log = OusNode::parse(&json!({"t":"log","lines": vec!["x"; MAX_LOG_LINES + 10]}));
        let OusNode::Log { lines, .. } = log else {
            panic!()
        };
        assert_eq!(lines.len(), MAX_LOG_LINES);
        assert_eq!(Tone::parse("c13"), Tone::Default);
    }

    #[test]
    fn diff_from_old_and_new_text() {
        let OusNode::Diff { patch, .. } =
            OusNode::parse(&json!({"t":"diff","oldText":"a","newText":"b"}))
        else {
            panic!()
        };
        assert_eq!(patch, "@@ -1,1 +1,1 @@\n-a\n+b\n");
    }
}
