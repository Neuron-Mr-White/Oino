//! UI probe fixture: `/ui-probe` raises every dialog method, fires every fire-and-forget
//! method, opens an entry in every slot, sends a tree that contains every node kind (ui-protocol
//! §4.5 / §12.1) plus a patch and a fallback, and returns what came back.

#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "SDK handler signatures are async even when a body does not await"
)]

use oino_extension::command::Command;
use oino_extension::prelude::*;
use oino_extension::ui::KeyBinding;
use oino_extension::ui::widgets::prelude::*;
use oino_extension::wit::registry::RendererTarget;

/// Fixture state.
#[derive(Default)]
struct UiProbe;

const DIALOGS: &[&str] = &[
    "select",
    "confirm",
    "input",
    "editor",
    "multiSelect",
    "form",
    "approval",
    "custom",
];
const FIRES: &[&str] = &[
    "notify",
    "setStatus",
    "setWidget",
    "setTitle",
    "set_editor_text",
    "setFrame",
    "setWorkingMessage",
    "setHeader",
    "setFooter",
];

/// Every panel placement of §6.2.
const PANEL_SLOTS: &[(ui::Placement, &str)] = &[
    (ui::Placement::Overlay, "overlay"),
    (ui::Placement::SidePane, "side"),
    (ui::Placement::BottomPane, "bottom"),
    (ui::Placement::EditorReplace, "editor"),
    (ui::Placement::Fullscreen, "full"),
];

/// Text and container nodes (`Block`, `Paragraph`, `List`, `Table`, `Tabs`).
fn text_nodes() -> Vec<Node> {
    vec![
        Block::bordered()
            .title("Block")
            .child(Paragraph::new("child"))
            .into(),
        Paragraph::new(Line::from(vec![
            Span::from("Paragraph").bold(),
            Span::from(" dim").dim(),
        ]))
        .into(),
        List::new([ListItem::new("List item")
            .key("item-1")
            .detail("detail")
            .badge("badge")
            .icon(Icon::List)])
        .highlight_symbol("› ")
        .on_select("select")
        .on_activate("open")
        .into(),
        Table::new(
            [Row::new([Cell::new("cell"), Cell::new("cell 2")])],
            [Constraint::Max(24), Constraint::Fill(1)],
        )
        .header(Row::new(["Key", "Value"]))
        .into(),
        Tabs::new([TabTitle::new("one", "One"), TabTitle::new("two", "Two")])
            .panes([
                ("one".to_owned(), Paragraph::new("tab one")),
                ("two".to_owned(), Paragraph::new("tab two")),
            ])
            .on_select("tab")
            .into(),
    ]
}

/// Data-display nodes (`Gauge`, `LineGauge`, `Sparkline`, `BarChart`, `Chart`, `Scrollbar`).
fn data_nodes() -> Vec<Node> {
    vec![
        Gauge::new().ratio(0.4).label("40%").into(),
        LineGauge::new(0.5).label("50%").into(),
        Sparkline::new([Some(1), Some(2), None, Some(4)])
            .max(4)
            .into(),
        BarChart::new([BarGroup::new([
            Bar::new(3).label("a").text_value("3"),
            Bar::new(5).label("b"),
        ])
        .label("group")])
        .direction(Direction::Horizontal)
        .into(),
        Chart::new([
            Dataset::new([[0.0, 1.0], [1.0, 2.0]])
                .name("series")
                .graph_type(GraphType::Line),
            Dataset::new([[0.5, 1.5]]).name("dots"),
        ])
        .x_axis(Axis::bounds(0.0, 2.0).title("x"))
        .y_axis(Axis::bounds(0.0, 3.0).title("y"))
        .into(),
        Scrollbar::new(ScrollbarState::new(10).position(2)).into(),
    ]
}

/// Content and control nodes (`Markdown`, `Code`, `Diff`, `Image`, `Log`, `Tree`, `KeyHints`,
/// `Input`, `Toggle`, `Select`, `Button`, `Spinner`, `Empty`).
fn extras_nodes() -> Vec<Node> {
    vec![
        Markdown::new("# Markdown").into(),
        Code::new("fn main() {}")
            .language("rust")
            .line_numbers(true)
            .into(),
        Diff::patch("--- a\n+++ b\n@@ -1 +1 @@\n-a\n+b")
            .path("a.rs")
            .into(),
        Image::new(ImageSource::path("/tmp/probe.png"), "image")
            .max_width(40)
            .into(),
        Log::new(["log 1", "log 2"]).follow(true).into(),
        Tree::new([TreeNode::new("root", "Root").children([TreeNode::new("leaf", "Leaf")])])
            .on_select("tree")
            .into(),
        KeyHints::new([("enter", "run")])
            .hint_action("enter", "run")
            .into(),
        Input::new("input").placeholder("type").multiline(2).into(),
        Toggle::new("toggle", "Toggle", true)
            .on_change("toggle")
            .into(),
        Select::new(
            "select",
            [SelectOption::new("a", "A"), SelectOption::new("b", "B")],
        )
        .value("a")
        .on_change("select")
        .into(),
        Button::new("Button", "button")
            .variant(ButtonVariant::Primary)
            .into(),
        Spinner::new("working").into(),
        Empty::new().title("Empty").message("nothing here").into(),
    ]
}

/// One tree holding every node kind of §4.5 (a frontend that renders this renders all of them).
#[must_use]
fn every_node() -> Node {
    let mut children = text_nodes();
    children.extend(data_nodes());
    children.extend(extras_nodes());
    Layout::default().spacing(1).children(children).into()
}

/// A node as the wire JSON the SDK's renderer callback returns.
fn wire(node: impl Into<Node>) -> Value {
    serde_json::to_value(node.into()).unwrap_or(Value::Null)
}

async fn run_probe() -> Result<Value> {
    let mut answers = Vec::new();
    for method in DIALOGS {
        answers.push(
            ui::dialog(&json!({"method": method, "title": format!("probe {method}")})).await?,
        );
    }
    for method in FIRES {
        ui::fire(&json!({"method": method, "probe": true}))?;
    }
    // One entry per panel slot, each with the full node tree and a key binding.
    let mut panels = Vec::new();
    for (slot, name) in PANEL_SLOTS {
        let id = ui::Panel::new(format!("probe/{name}"))
            .placement(*slot)
            .title(format!("Probe {name}"))
            .icon(Icon::Dot)
            .spec(every_node())
            .keys([KeyBinding {
                key: "ctrl+p".into(),
                action: Some("probe".into()),
                label: "probe".into(),
            }])
            .focus(true)
            .close_on_escape(false)
            .open()?;
        ui::Panel::update(&id).spec(every_node()).send()?;
        panels.push(id);
    }
    let first = panels.first().cloned().unwrap_or_default();
    if !first.is_empty() {
        // Explicit ops (§4.7) and the fallback tree of a non-reserved entry (§9.1).
        ui::Panel::update(&first)
            .patch([NodeOp::append(
                ["children"],
                PatchField::Lines,
                ["line".to_owned()],
            )])
            .send()?;
        ui::Panel::update(&first)
            .fallback(Paragraph::new("fallback"))
            .send()?;
    }
    // Widget slots (`aboveEditor` default, `belowEditor`), status, header, footer.
    ui::widget("probe", Some(every_node()))?;
    ui::fire(
        &json!({"method": "setWidget", "widgetKey": "probe-below", "widgetPlacement": "belowEditor", "widgetSpec": wire(every_node())}),
    )?;
    ui::status("probe", Some(Line::from("Probe").fg(Color::Info)))?;
    ui::fire(&json!({"method": "setHeader", "key": "probe", "spec": wire(every_node())}))?;
    ui::fire(&json!({"method": "setFooter", "key": "probe", "spec": wire(every_node())}))?;
    ui::editor_set("draft");
    ui::editor_insert("!");
    let editor = ui::editor_text();
    ui::clipboard_write_text("copied")?;
    ui::open_url("https://example.com")?;
    let confirmed = ui::confirm("Sure?", "really").await?;
    for id in &panels {
        ui::Panel::update(id).close();
    }
    Ok(
        json!({"answers": answers, "panels": panels, "panel": first, "editor": editor, "confirmed": confirmed}),
    )
}

impl Extension for UiProbe {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.command(
            Command::new("ui-probe")
                .description("Exercise every UI request")
                .run_value(|_ext, _args, _cx| async { run_probe().await.map(Some) }),
        );
        // The `turnFooter` slot (§6.7): registering the render key is what creates the entry.
        cx.renderer(RendererTarget::TurnFooter, "probe", |_ext, _req, _cx| {
            Some(wire(
                KeyHints::new([("enter", "run")]).hint_action("enter", "probe"),
            ))
        });
        Ok(())
    }
}

oino_extension::export_extension!(UiProbe);
