//! Golden per node (WP-U4): every fixture of `crates/oino-ui/fixtures/nodes` renders at
//! 40 and 100 columns; the snapshot includes text and a style run per row (`ui-protocol
//! §12.4` point 6).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::fs;
use std::path::PathBuf;

use oino_tui::theme::{DARK, Theme};
use oino_ui::node::Node;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

fn fixtures() -> Vec<(String, PathBuf)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../oino-ui/fixtures/nodes");
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .expect("fixtures dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".json")
        })
        .map(|entry| {
            (
                entry
                    .file_name()
                    .to_string_lossy()
                    .trim_end_matches(".json")
                    .to_owned(),
                entry.path(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn base(theme: &Theme) -> Style {
    Style::default().fg(theme.text)
}

fn color(color: Color) -> String {
    match color {
        Color::Reset => "reset".to_owned(),
        Color::Rgb(r, g, b) => format!("rgb({r:02x}{g:02x}{b:02x})"),
        other => format!("{other:?}"),
    }
}

fn modifiers(modifiers: Modifier) -> String {
    let mut out = Vec::new();
    for (flag, name) in [
        (Modifier::BOLD, "BOLD"),
        (Modifier::DIM, "DIM"),
        (Modifier::ITALIC, "ITALIC"),
        (Modifier::UNDERLINED, "UNDERLINED"),
        (Modifier::SLOW_BLINK, "SLOW_BLINK"),
        (Modifier::RAPID_BLINK, "RAPID_BLINK"),
        (Modifier::REVERSED, "REVERSED"),
        (Modifier::HIDDEN, "HIDDEN"),
        (Modifier::CROSSED_OUT, "CROSSED_OUT"),
    ] {
        if modifiers.contains(flag) {
            out.push(name);
        }
    }
    out.join("+")
}

/// Text plus the style runs that differ from the body-text base (style markers, §12.4).
fn dump(lines: &[Line<'_>], theme: &Theme) -> String {
    let base = base(theme);
    let mut out = String::new();
    for line in lines {
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        out.push_str(text.trim_end());
        let mut runs = Vec::new();
        let mut offset = 0_usize;
        for span in &line.spans {
            let width = span.content.chars().count();
            if width > 0 {
                let mut fields = Vec::new();
                if let Some(fg) = span.style.fg
                    && fg != Color::Reset
                    && Some(fg) != base.fg
                {
                    fields.push(format!("fg={}", color(fg)));
                }
                if let Some(bg) = span.style.bg
                    && bg != Color::Reset
                {
                    fields.push(format!("bg={}", color(bg)));
                }
                if let Some(ul) = span.style.underline_color
                    && ul != Color::Reset
                {
                    fields.push(format!("ul={}", color(ul)));
                }
                let mods = modifiers(span.style.add_modifier);
                if !mods.is_empty() {
                    fields.push(format!("mods={mods}"));
                }
                if !fields.is_empty() {
                    runs.push(format!("{offset}..{}={}", offset + width, fields.join(",")));
                }
            }
            offset += width;
        }
        if !runs.is_empty() {
            out.push('\n');
            out.push_str("  ~ ");
            out.push_str(&runs.join(" "));
        }
        out.push('\n');
    }
    out
}

#[test]
fn every_fixture_renders_at_40_and_100_columns() {
    let fixtures = fixtures();
    assert!(
        fixtures.len() >= 60,
        "the node corpus holds {} fixtures",
        fixtures.len()
    );
    for (name, path) in fixtures {
        let raw = fs::read_to_string(&path).expect("fixture readable");
        let node: Node = serde_json::from_str(&raw).expect("fixture parses as a node");
        for width in [40_u16, 100] {
            let height = oino_tui::render::measure_v2_node(&node, width, &DARK).clamp(1, 24);
            let lines = oino_tui::render::render_v2_node(&node, width, height, &DARK);
            let rendered = dump(&lines, &DARK);
            insta::assert_snapshot!(format!("{name}-{width}x{height}"), rendered);
        }
    }
}
