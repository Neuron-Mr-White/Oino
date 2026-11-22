//! Oino UI Spec v1 nodes (ui-protocol §6), parsed from JSON into a typed tree.
//!
//! Parsing never fails: unknown kinds become [`OusNode::Unknown`] (rendered via `fallback`),
//! missing props take defaults, and trees deeper than [`MAX_DEPTH`] are cut.

use oino_ai::JsonValue;

/// Maximum nesting depth (ui-protocol §6.2).
pub const MAX_DEPTH: usize = 32;
/// Maximum `log.lines` kept.
pub const MAX_LOG_LINES: usize = 5_000;

/// Semantic tone (ui-protocol §6.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// Default text.
    #[default]
    Default,
    /// Muted.
    Muted,
    /// Accent.
    Accent,
    /// Informational.
    Info,
    /// Success.
    Success,
    /// Warning.
    Warning,
    /// Error.
    Error,
    /// Harness provenance.
    Harness,
    /// Thinking.
    Thinking,
    /// Tool.
    Tool,
    /// User.
    User,
    /// Assistant.
    Assistant,
    /// Categorical palette `c1`..`c12`.
    Category(u8),
}

impl Tone {
    /// Parses a tone name; unknown names are [`Tone::Default`].
    #[must_use]
    pub fn parse(name: &str) -> Self {
        match name {
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
                .map_or(Self::Default, Self::Category),
        }
    }

    fn from_json(value: Option<&JsonValue>) -> Self {
        value
            .and_then(JsonValue::as_str)
            .map_or(Self::Default, Self::parse)
    }
}

/// Styled text run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent style flags of an OUS span"
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
    /// Underline.
    pub underline: bool,
    /// Inline code.
    pub code: bool,
    /// Link target.
    pub link: Option<String>,
}

/// `Rich = string | Span[]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rich(pub Vec<Span>);

impl Rich {
    /// Plain text rich value.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self(vec![Span {
            text: text.into(),
            ..Span::default()
        }])
    }

    /// Concatenated text.
    #[must_use]
    pub fn text(&self) -> String {
        self.0.iter().map(|s| s.text.as_str()).collect()
    }

    fn from_json(value: Option<&JsonValue>) -> Self {
        match value {
            Some(JsonValue::String(s)) => Self::plain(s.clone()),
            Some(JsonValue::Array(items)) => Self(
                items
                    .iter()
                    .map(|item| match item {
                        JsonValue::String(s) => Span {
                            text: s.clone(),
                            ..Span::default()
                        },
                        other => Span {
                            text: str_prop(other, "text"),
                            tone: Tone::from_json(other.get("tone")),
                            bold: bool_prop(other, "bold"),
                            italic: bool_prop(other, "italic"),
                            dim: bool_prop(other, "dim"),
                            underline: bool_prop(other, "underline"),
                            code: bool_prop(other, "code"),
                            link: opt_str(other, "link"),
                        },
                    })
                    .collect(),
            ),
            Some(JsonValue::Number(n)) => Self::plain(n.to_string()),
            _ => Self::default(),
        }
    }
}

/// Layout direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// Side by side.
    Horizontal,
    /// Stacked.
    #[default]
    Vertical,
}

/// Table column alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// Left.
    #[default]
    Left,
    /// Centre.
    Center,
    /// Right.
    Right,
}

/// Table column width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnWidth {
    /// Fit content.
    #[default]
    Auto,
    /// Take remaining space.
    Fill,
    /// Fixed cells.
    Fixed(u16),
}

/// Table column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    /// Header.
    pub title: String,
    /// Alignment.
    pub align: Align,
    /// Width.
    pub width: ColumnWidth,
}

/// List item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListItem {
    /// Item id (action value).
    pub id: String,
    /// Label.
    pub label: Rich,
    /// Secondary text.
    pub detail: Option<Rich>,
    /// Badge.
    pub badge: Option<Rich>,
    /// Tone.
    pub tone: Tone,
}

/// Tree node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

/// Tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    /// Tab id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Body.
    pub body: OusNode,
}

/// Bar of a bar chart.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    /// Label.
    pub label: String,
    /// Value.
    pub value: f64,
    /// Tone.
    pub tone: Tone,
}

/// Area chart series.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// Label.
    pub label: String,
    /// Values.
    pub values: Vec<f64>,
}

/// Key hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyHint {
    /// Keys text.
    pub keys: String,
    /// Label.
    pub label: String,
}

/// Select option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    /// Value.
    pub value: String,
    /// Label.
    pub label: String,
}

/// Diff content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffContent {
    /// Unified patch.
    Patch(String),
    /// Old/new texts.
    Texts {
        /// Old text.
        old_text: String,
        /// New text.
        new_text: String,
        /// File path.
        path: Option<String>,
    },
}

/// Image source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSource {
    /// Inline base64 data.
    Data {
        /// Base64.
        data: String,
        /// MIME type.
        mime_type: String,
    },
    /// File path.
    Path(String),
}

/// One OUS node.
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "a node tree is built once per spec; boxing every variant adds noise"
)]
pub enum OusNode {
    /// Paragraph.
    Text {
        /// Value.
        value: Rich,
        /// Wrap.
        wrap: bool,
    },
    /// Markdown.
    Markdown {
        /// Source.
        value: String,
    },
    /// Code block.
    Code {
        /// Source.
        value: String,
        /// Language.
        language: Option<String>,
        /// Line numbers.
        line_numbers: bool,
    },
    /// Diff.
    Diff(DiffContent),
    /// Image.
    Image {
        /// Source.
        source: Option<ImageSource>,
        /// Alt text.
        alt: String,
    },
    /// Log lines.
    Log {
        /// Lines.
        lines: Vec<String>,
        /// Follow the tail.
        tail: bool,
        /// Max visible lines.
        max_lines: Option<usize>,
    },
    /// Key/value pairs.
    Kv {
        /// Items.
        items: Vec<(Rich, Rich)>,
    },
    /// Table.
    Table {
        /// Columns.
        columns: Vec<Column>,
        /// Rows.
        rows: Vec<Vec<Rich>>,
        /// Row selection action.
        selectable: Option<String>,
    },
    /// List.
    List {
        /// Items.
        items: Vec<ListItem>,
        /// Selection action.
        selectable: Option<String>,
        /// Selected id.
        selected: Option<String>,
        /// Multi-select.
        multi: bool,
    },
    /// Tree.
    Tree {
        /// Roots.
        nodes: Vec<TreeItem>,
        /// Selection action.
        selectable: Option<String>,
    },
    /// Tabs.
    Tabs {
        /// Tabs.
        tabs: Vec<Tab>,
        /// Active tab id.
        active: Option<String>,
        /// Change action.
        on_change: Option<String>,
    },
    /// Split layout.
    Split {
        /// Direction.
        direction: Direction,
        /// Children.
        children: Vec<OusNode>,
        /// Ratios.
        ratios: Vec<f64>,
    },
    /// Stack layout.
    Stack {
        /// Direction.
        direction: Direction,
        /// Gap.
        gap: u16,
        /// Children.
        children: Vec<OusNode>,
    },
    /// Titled section.
    Section {
        /// Title.
        title: Rich,
        /// Body.
        body: Box<OusNode>,
        /// Collapsed.
        collapsed: bool,
    },
    /// Card.
    Card {
        /// Title.
        title: Option<Rich>,
        /// Rail tone.
        rail: Option<Tone>,
        /// Provenance label.
        source: Option<String>,
        /// Body.
        body: Box<OusNode>,
    },
    /// Badge.
    Badge {
        /// Value.
        value: Rich,
        /// Tone.
        tone: Tone,
    },
    /// Meter.
    Meter {
        /// Value.
        value: f64,
        /// Max.
        max: f64,
        /// Label.
        label: Option<String>,
        /// Tone.
        tone: Tone,
    },
    /// Gauge (0..1).
    Gauge {
        /// Value.
        value: f64,
        /// Label.
        label: Option<String>,
    },
    /// Sparkline.
    Sparkline {
        /// Values.
        values: Vec<f64>,
        /// Label.
        label: Option<String>,
    },
    /// Bar chart.
    BarChart {
        /// Bars.
        bars: Vec<Bar>,
    },
    /// Area chart.
    Area {
        /// Series.
        series: Vec<Series>,
    },
    /// Spinner.
    Spinner {
        /// Label.
        label: Option<String>,
    },
    /// Divider.
    Divider {
        /// Label.
        label: Option<String>,
    },
    /// Key hints.
    KeyHints {
        /// Hints.
        hints: Vec<KeyHint>,
    },
    /// Text input.
    Input {
        /// Field id.
        id: String,
        /// Value.
        value: String,
        /// Placeholder.
        placeholder: Option<String>,
        /// Submit action.
        on_submit: String,
        /// Multi-line.
        multiline: bool,
    },
    /// Toggle.
    Toggle {
        /// Field id.
        id: String,
        /// Label.
        label: String,
        /// Value.
        value: bool,
        /// Change action.
        on_change: String,
    },
    /// Cycling select.
    Select {
        /// Field id.
        id: String,
        /// Options.
        options: Vec<SelectOption>,
        /// Current value.
        value: Option<String>,
        /// Change action.
        on_change: String,
    },
    /// Button.
    Button {
        /// Label.
        label: String,
        /// Action.
        action_id: String,
        /// Tone.
        tone: Tone,
        /// Primary.
        primary: bool,
    },
    /// Empty state.
    Empty {
        /// Message.
        message: Option<String>,
    },
    /// Unknown kind.
    Unknown {
        /// The kind.
        kind: String,
        /// Fallback node.
        fallback: Option<Box<OusNode>>,
    },
}

fn str_prop(value: &JsonValue, key: &str) -> String {
    value
        .get(key)
        .map(|v| match v {
            JsonValue::String(s) => s.clone(),
            JsonValue::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn opt_str(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn bool_prop(value: &JsonValue, key: &str) -> bool {
    value.get(key).and_then(JsonValue::as_bool).unwrap_or(false)
}

fn num(value: &JsonValue, key: &str) -> f64 {
    value.get(key).and_then(JsonValue::as_f64).unwrap_or(0.0)
}

fn array<'a>(value: &'a JsonValue, key: &str) -> &'a [JsonValue] {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .map_or(&[], Vec::as_slice)
}

fn direction(value: &JsonValue) -> Direction {
    match value.get("direction").and_then(JsonValue::as_str) {
        Some("horizontal") => Direction::Horizontal,
        _ => Direction::Vertical,
    }
}

impl OusNode {
    /// Parses a node; never fails.
    #[must_use]
    pub fn from_json(value: &JsonValue) -> Self {
        Self::parse(value, 0)
    }

    fn children(value: &JsonValue, key: &str, depth: usize) -> Vec<Self> {
        array(value, key)
            .iter()
            .map(|c| Self::parse(c, depth + 1))
            .collect()
    }

    fn child(value: &JsonValue, key: &str, depth: usize) -> Self {
        value
            .get(key)
            .map_or(Self::Empty { message: None }, |c| Self::parse(c, depth + 1))
    }

    fn tree_items(values: &[JsonValue], depth: usize) -> Vec<TreeItem> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        values
            .iter()
            .map(|n| TreeItem {
                id: str_prop(n, "id"),
                label: Rich::from_json(n.get("label")),
                children: Self::tree_items(array(n, "children"), depth + 1),
                expanded: n
                    .get("expanded")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(true),
            })
            .collect()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per OUS v1 node kind (ui-protocol §6.1 table)"
    )]
    fn parse(value: &JsonValue, depth: usize) -> Self {
        if depth > MAX_DEPTH {
            return Self::Empty { message: None };
        }
        if let JsonValue::String(text) = value {
            return Self::Text {
                value: Rich::plain(text.clone()),
                wrap: true,
            };
        }
        let kind = value.get("t").and_then(JsonValue::as_str).unwrap_or("");
        match kind {
            "text" => Self::Text {
                value: Rich::from_json(value.get("value")),
                wrap: value
                    .get("wrap")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(true),
            },
            "markdown" => Self::Markdown {
                value: str_prop(value, "value"),
            },
            "code" => Self::Code {
                value: str_prop(value, "value"),
                language: opt_str(value, "language"),
                line_numbers: bool_prop(value, "lineNumbers"),
            },
            "diff" => match value.get("patch") {
                Some(JsonValue::String(patch)) => Self::Diff(DiffContent::Patch(patch.clone())),
                Some(object @ JsonValue::Object(_)) => Self::Diff(DiffContent::Texts {
                    old_text: str_prop(object, "oldText"),
                    new_text: str_prop(object, "newText"),
                    path: opt_str(object, "path"),
                }),
                _ => Self::Diff(DiffContent::Texts {
                    old_text: str_prop(value, "oldText"),
                    new_text: str_prop(value, "newText"),
                    path: opt_str(value, "path"),
                }),
            },
            "image" => Self::Image {
                source: match (opt_str(value, "data"), opt_str(value, "path")) {
                    (Some(data), _) => Some(ImageSource::Data {
                        data,
                        mime_type: opt_str(value, "mimeType").unwrap_or_else(|| "image/png".into()),
                    }),
                    (None, Some(path)) => Some(ImageSource::Path(path)),
                    (None, None) => None,
                },
                alt: str_prop(value, "alt"),
            },
            "log" => {
                let mut lines: Vec<String> = array(value, "lines")
                    .iter()
                    .map(|l| l.as_str().unwrap_or_default().to_owned())
                    .collect();
                if lines.len() > MAX_LOG_LINES {
                    lines.drain(..lines.len() - MAX_LOG_LINES);
                }
                Self::Log {
                    lines,
                    tail: bool_prop(value, "tail"),
                    max_lines: value
                        .get("maxLines")
                        .and_then(JsonValue::as_u64)
                        .and_then(|n| usize::try_from(n).ok()),
                }
            }
            "kv" => Self::Kv {
                items: array(value, "items")
                    .iter()
                    .map(|i| {
                        (
                            Rich::from_json(i.get("key")),
                            Rich::from_json(i.get("value")),
                        )
                    })
                    .collect(),
            },
            "table" => Self::Table {
                columns: array(value, "columns")
                    .iter()
                    .map(|c| Column {
                        title: str_prop(c, "title"),
                        align: match c.get("align").and_then(JsonValue::as_str) {
                            Some("center") => Align::Center,
                            Some("right") => Align::Right,
                            _ => Align::Left,
                        },
                        width: match c.get("width") {
                            Some(JsonValue::String(s)) if s == "fill" => ColumnWidth::Fill,
                            Some(JsonValue::Number(n)) => n
                                .as_u64()
                                .and_then(|n| u16::try_from(n).ok())
                                .map_or(ColumnWidth::Auto, ColumnWidth::Fixed),
                            _ => ColumnWidth::Auto,
                        },
                    })
                    .collect(),
                rows: array(value, "rows")
                    .iter()
                    .map(|r| {
                        r.as_array()
                            .map(|cells| cells.iter().map(|c| Rich::from_json(Some(c))).collect())
                            .unwrap_or_default()
                    })
                    .collect(),
                selectable: opt_str(value, "selectable"),
            },
            "list" => Self::List {
                items: array(value, "items")
                    .iter()
                    .map(|i| ListItem {
                        id: str_prop(i, "id"),
                        label: Rich::from_json(i.get("label")),
                        detail: i.get("detail").map(|d| Rich::from_json(Some(d))),
                        badge: i.get("badge").map(|b| Rich::from_json(Some(b))),
                        tone: Tone::from_json(i.get("tone")),
                    })
                    .collect(),
                selectable: opt_str(value, "selectable"),
                selected: opt_str(value, "selected"),
                multi: bool_prop(value, "multi"),
            },
            "tree" => Self::Tree {
                nodes: Self::tree_items(array(value, "nodes"), depth),
                selectable: opt_str(value, "selectable"),
            },
            "tabs" => Self::Tabs {
                tabs: array(value, "tabs")
                    .iter()
                    .map(|t| Tab {
                        id: str_prop(t, "id"),
                        title: str_prop(t, "title"),
                        body: t
                            .get("body")
                            .map_or(Self::Empty { message: None }, |b| Self::parse(b, depth + 1)),
                    })
                    .collect(),
                active: opt_str(value, "active"),
                on_change: opt_str(value, "onChange"),
            },
            "split" => Self::Split {
                direction: direction(value),
                children: Self::children(value, "children", depth),
                ratios: array(value, "ratios")
                    .iter()
                    .filter_map(JsonValue::as_f64)
                    .collect(),
            },
            "stack" => Self::Stack {
                direction: direction(value),
                gap: value
                    .get("gap")
                    .and_then(JsonValue::as_u64)
                    .and_then(|g| u16::try_from(g).ok())
                    .unwrap_or(0),
                children: Self::children(value, "children", depth),
            },
            "section" => Self::Section {
                title: Rich::from_json(value.get("title")),
                body: Box::new(Self::child(value, "body", depth)),
                collapsed: bool_prop(value, "collapsed"),
            },
            "card" => Self::Card {
                title: value.get("title").map(|t| Rich::from_json(Some(t))),
                rail: value.get("rail").map(|r| Tone::from_json(Some(r))),
                source: opt_str(value, "source"),
                body: Box::new(Self::child(value, "body", depth)),
            },
            "badge" => Self::Badge {
                value: Rich::from_json(value.get("value")),
                tone: Tone::from_json(value.get("tone")),
            },
            "meter" => Self::Meter {
                value: num(value, "value"),
                max: value.get("max").and_then(JsonValue::as_f64).unwrap_or(1.0),
                label: opt_str(value, "label"),
                tone: Tone::from_json(value.get("tone")),
            },
            "gauge" => Self::Gauge {
                value: num(value, "value").clamp(0.0, 1.0),
                label: opt_str(value, "label"),
            },
            "sparkline" => Self::Sparkline {
                values: array(value, "values")
                    .iter()
                    .filter_map(JsonValue::as_f64)
                    .collect(),
                label: opt_str(value, "label"),
            },
            "barChart" => Self::BarChart {
                bars: array(value, "bars")
                    .iter()
                    .map(|b| Bar {
                        label: str_prop(b, "label"),
                        value: num(b, "value"),
                        tone: Tone::from_json(b.get("tone")),
                    })
                    .collect(),
            },
            "area" => Self::Area {
                series: array(value, "series")
                    .iter()
                    .map(|s| Series {
                        label: str_prop(s, "label"),
                        values: array(s, "values")
                            .iter()
                            .filter_map(JsonValue::as_f64)
                            .collect(),
                    })
                    .collect(),
            },
            "spinner" => Self::Spinner {
                label: opt_str(value, "label"),
            },
            "divider" => Self::Divider {
                label: opt_str(value, "label"),
            },
            "keyHints" => Self::KeyHints {
                hints: array(value, "hints")
                    .iter()
                    .map(|h| KeyHint {
                        keys: str_prop(h, "keys"),
                        label: str_prop(h, "label"),
                    })
                    .collect(),
            },
            "input" => Self::Input {
                id: str_prop(value, "id"),
                value: str_prop(value, "value"),
                placeholder: opt_str(value, "placeholder"),
                on_submit: str_prop(value, "onSubmit"),
                multiline: bool_prop(value, "multiline"),
            },
            "toggle" => Self::Toggle {
                id: str_prop(value, "id"),
                label: str_prop(value, "label"),
                value: bool_prop(value, "value"),
                on_change: str_prop(value, "onChange"),
            },
            "select" => Self::Select {
                id: str_prop(value, "id"),
                options: array(value, "options")
                    .iter()
                    .map(|o| SelectOption {
                        value: str_prop(o, "value"),
                        label: str_prop(o, "label"),
                    })
                    .collect(),
                value: opt_str(value, "value"),
                on_change: str_prop(value, "onChange"),
            },
            "button" => Self::Button {
                label: str_prop(value, "label"),
                action_id: str_prop(value, "actionId"),
                tone: Tone::from_json(value.get("tone")),
                primary: bool_prop(value, "primary"),
            },
            "empty" => Self::Empty {
                message: opt_str(value, "message"),
            },
            other => Self::Unknown {
                kind: other.to_owned(),
                fallback: value
                    .get("fallback")
                    .map(|f| Box::new(Self::parse(f, depth + 1))),
            },
        }
    }

    /// Interactive targets in document order (focus ring for frontends).
    #[must_use]
    pub fn interactive(&self) -> Vec<Interactive> {
        let mut out = Vec::new();
        self.collect_interactive(&mut out);
        out
    }

    fn collect_interactive(&self, out: &mut Vec<Interactive>) {
        match self {
            Self::Button {
                label, action_id, ..
            } => out.push(Interactive::Button {
                label: label.clone(),
                action_id: action_id.clone(),
            }),
            Self::Input {
                id,
                value,
                on_submit,
                placeholder,
                ..
            } => out.push(Interactive::Input {
                id: id.clone(),
                value: value.clone(),
                placeholder: placeholder.clone(),
                action_id: on_submit.clone(),
            }),
            Self::Toggle {
                id,
                label,
                value,
                on_change,
            } => out.push(Interactive::Toggle {
                id: id.clone(),
                label: label.clone(),
                value: *value,
                action_id: on_change.clone(),
            }),
            Self::Select {
                id,
                options,
                value,
                on_change,
            } => out.push(Interactive::Select {
                id: id.clone(),
                options: options.clone(),
                value: value.clone(),
                action_id: on_change.clone(),
            }),
            Self::List {
                items,
                selectable: Some(action),
                ..
            } => out.push(Interactive::Choice {
                action_id: action.clone(),
                ids: items.iter().map(|i| i.id.clone()).collect(),
                labels: items.iter().map(|i| i.label.text()).collect(),
            }),
            Self::Tabs { tabs, active, .. } => {
                let active = active
                    .as_deref()
                    .or_else(|| tabs.first().map(|t| t.id.as_str()));
                if let Some(tab) = tabs.iter().find(|t| Some(t.id.as_str()) == active) {
                    tab.body.collect_interactive(out);
                }
            }
            Self::Split { children, .. } | Self::Stack { children, .. } => {
                for child in children {
                    child.collect_interactive(out);
                }
            }
            Self::Section {
                body, collapsed, ..
            } => {
                if !collapsed {
                    body.collect_interactive(out);
                }
            }
            Self::Card { body, .. } => body.collect_interactive(out),
            Self::Unknown {
                fallback: Some(f), ..
            } => f.collect_interactive(out),
            _ => {}
        }
    }

    /// Plain-text rendering (ui-protocol §10 degradation, copy, search).
    #[must_use]
    pub fn plain_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_plain(&mut out);
        out
    }

    #[allow(clippy::too_many_lines, reason = "one arm per OUS node kind")]
    fn collect_plain(&self, out: &mut Vec<String>) {
        match self {
            Self::Text { value, .. } => out.extend(value.text().lines().map(str::to_owned)),
            Self::Markdown { value } | Self::Code { value, .. } => {
                out.extend(value.lines().map(str::to_owned));
            }
            Self::Diff(DiffContent::Patch(p)) => out.extend(p.lines().map(str::to_owned)),
            Self::Diff(DiffContent::Texts { path, .. }) => {
                out.push(format!("diff {}", path.as_deref().unwrap_or("")));
            }
            Self::Image { alt, .. } => out.push(format!("[image: {alt}]")),
            Self::Log { lines, .. } => out.extend(lines.iter().cloned()),
            Self::Kv { items } => {
                out.extend(
                    items
                        .iter()
                        .map(|(k, v)| format!("{}: {}", k.text(), v.text())),
                );
            }
            Self::Table { columns, rows, .. } => {
                out.push(
                    columns
                        .iter()
                        .map(|c| c.title.as_str())
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                for row in rows {
                    out.push(row.iter().map(Rich::text).collect::<Vec<_>>().join(" | "));
                }
            }
            Self::List { items, .. } => {
                out.extend(items.iter().map(|i| format!("- {}", i.label.text())));
            }
            Self::Tree { nodes, .. } => {
                fn walk(items: &[TreeItem], depth: usize, out: &mut Vec<String>) {
                    for item in items {
                        out.push(format!("{}{}", "  ".repeat(depth), item.label.text()));
                        walk(&item.children, depth + 1, out);
                    }
                }
                walk(nodes, 0, out);
            }
            Self::Tabs { tabs, active, .. } => {
                let active = active
                    .as_deref()
                    .or_else(|| tabs.first().map(|t| t.id.as_str()));
                if let Some(tab) = tabs.iter().find(|t| Some(t.id.as_str()) == active) {
                    tab.body.collect_plain(out);
                }
            }
            Self::Split { children, .. } | Self::Stack { children, .. } => {
                for c in children {
                    c.collect_plain(out);
                }
            }
            Self::Section { title, body, .. } => {
                out.push(title.text());
                body.collect_plain(out);
            }
            Self::Card { title, body, .. } => {
                if let Some(t) = title {
                    out.push(t.text());
                }
                body.collect_plain(out);
            }
            Self::Badge { value, .. } => out.push(format!("[{}]", value.text())),
            Self::Meter {
                value, max, label, ..
            } => out.push(format!("{} {value}/{max}", label.as_deref().unwrap_or(""))),
            Self::Gauge { value, label } => {
                out.push(format!(
                    "{} {:.0}%",
                    label.as_deref().unwrap_or(""),
                    value * 100.0
                ));
            }
            Self::Spinner { label } | Self::Divider { label } => {
                out.push(label.clone().unwrap_or_default());
            }
            Self::KeyHints { hints } => out.push(
                hints
                    .iter()
                    .map(|h| format!("{} {}", h.keys, h.label))
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
            Self::Button { label, .. } => out.push(format!("[ {label} ]")),
            Self::Toggle { label, value, .. } => {
                out.push(format!("[{}] {label}", if *value { "x" } else { " " }));
            }
            Self::Empty { message } => out.push(message.clone().unwrap_or_default()),
            Self::Unknown {
                fallback: Some(f), ..
            } => f.collect_plain(out),
            Self::Sparkline { .. }
            | Self::BarChart { .. }
            | Self::Area { .. }
            | Self::Input { .. }
            | Self::Select { .. }
            | Self::Unknown { .. } => {}
        }
    }
}

/// A focusable control inside a spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Interactive {
    /// Button.
    Button {
        /// Label.
        label: String,
        /// Action.
        action_id: String,
    },
    /// Text input.
    Input {
        /// Field id.
        id: String,
        /// Initial value.
        value: String,
        /// Placeholder.
        placeholder: Option<String>,
        /// Submit action.
        action_id: String,
    },
    /// Toggle.
    Toggle {
        /// Field id.
        id: String,
        /// Label.
        label: String,
        /// Value.
        value: bool,
        /// Change action.
        action_id: String,
    },
    /// Cycling select.
    Select {
        /// Field id.
        id: String,
        /// Options.
        options: Vec<SelectOption>,
        /// Value.
        value: Option<String>,
        /// Change action.
        action_id: String,
    },
    /// Selectable list.
    Choice {
        /// Action.
        action_id: String,
        /// Item ids.
        ids: Vec<String>,
        /// Item labels.
        labels: Vec<String>,
    },
}

impl Interactive {
    /// Action id fired by this control.
    #[must_use]
    pub fn action_id(&self) -> &str {
        match self {
            Self::Button { action_id, .. }
            | Self::Input { action_id, .. }
            | Self::Toggle { action_id, .. }
            | Self::Select { action_id, .. }
            | Self::Choice { action_id, .. } => action_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_nodes_and_unknowns() {
        let node = OusNode::from_json(&json!({"t":"stack","children":[
            {"t":"text","value":[{"text":"hi","tone":"accent","bold":true}]},
            {"t":"button","label":"Go","actionId":"go","primary":true},
            {"t":"mystery","fallback":{"t":"text","value":"fb"}},
            {"t":"list","selectable":"pick","items":[{"id":"a","label":"A"}]}
        ]}));
        let OusNode::Stack { children, .. } = &node else {
            panic!()
        };
        assert_eq!(children.len(), 4);
        assert!(
            matches!(&children[0], OusNode::Text { value, .. } if value.0[0].tone == Tone::Accent && value.0[0].bold)
        );
        let actions: Vec<String> = node
            .interactive()
            .iter()
            .map(|i| i.action_id().to_owned())
            .collect();
        assert_eq!(actions, vec!["go", "pick"]);
        assert_eq!(node.plain_lines(), vec!["hi", "[ Go ]", "fb", "- A"]);
    }

    #[test]
    fn depth_is_limited() {
        let mut value = json!({"t":"text","value":"leaf"});
        for _ in 0..40 {
            value = json!({"t":"card","body": value});
        }
        let node = OusNode::from_json(&value);
        assert!(node.plain_lines().iter().all(|l| l != "leaf"));
        assert_eq!(Tone::parse("c3"), Tone::Category(3));
        assert_eq!(Tone::parse("c13"), Tone::Default);
    }
}
