//! §4.8 gate tests beyond the unit tests in `limits.rs`: byte bounds, node/item
//! caps, ring trims, image bounds and the patch validator.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]
use oino_ui::limits::{Limits, encoded_size, validate, validate_patch, validate_report};
use oino_ui::node::Node;
use oino_ui::patch::{NodeOp, PatchField, PatchItem};
use oino_ui::prelude::*;
use serde_json::Value as Json;

fn big_paragraph(n_bytes: usize) -> Node {
    let text: String = "x".repeat(n_bytes);
    Node::Paragraph(Paragraph::new(text))
}

#[test]
fn tree_over_256_kib_rejected() {
    let node = big_paragraph(256 * 1024 + 1);
    let err = validate(&node).expect_err("oversized tree rejected");
    assert!(err[0].message.contains("256 KiB"));
}

#[test]
fn tree_at_256_kib_accepted_and_measured() {
    // Just under the bound, from spans that each stay below the 64 KiB cap.
    let mut root = Layout::vertical([Constraint::Fill(1)]);
    while encoded_size(&root.clone().into()) < 256 * 1024 - 4 * 1024 {
        root.children
            .push(Node::Paragraph(Paragraph::new("x".repeat(4 * 1024))));
    }
    let node: Node = root.into();
    assert!(
        encoded_size(&node) <= 256 * 1024,
        "must fit: {}",
        encoded_size(&node)
    );
    assert!(validate(&node).is_ok());
}

#[test]
fn node_cap_5000() {
    let items: Vec<ListItem> = (0..5_001_u32)
        .map(|i| ListItem::new(i.to_string()).key(format!("k{i}")))
        .collect();
    let node = Node::List(List::new(items));
    let report = validate_report(&node);
    assert!(
        report.rejects.iter().any(|d| d.message.contains("5000")),
        "node cap must trip: {:?}",
        report.rejects
    );
}

#[test]
fn item_cap_5000_per_collection() {
    // One table over the per-collection row cap (and, necessarily, the tree cap).
    let rows: Vec<Row> = (0..5_001_u32)
        .map(|i| Row::new([i.to_string()]).key(format!("r{i}")))
        .collect();
    let node = Node::Table(Table::new(rows, [Constraint::Length(8)]));
    let report = validate_report(&node);
    assert!(
        report
            .rejects
            .iter()
            .any(|d| d.message.contains("table exceeds")),
        "item cap must trip: {:?}",
        report.rejects
    );
}

#[test]
fn log_ring_is_soft_maxlines_cap_is_hard() {
    let node = Node::Log(Log::new(std::iter::repeat_n("line", 600)).max_lines(500));
    let report = validate_report(&node);
    assert_eq!(report.rejects, Vec::<oino_ui::limits::Diagnostic>::new());
    assert!(report.soft.iter().any(|d| d.message.contains("trimmed")));

    let node = Node::Log(Log::new(["x"]).max_lines(5_001));
    let err = validate(&node).expect_err("maxLines > 5 000 rejected");
    assert!(err[0].message.contains("5000"));
}

#[test]
fn image_bounds() {
    // Lift the 256 KiB tree bound to isolate the §4.8 image decoded-size rules.
    let limits = Limits {
        max_tree_bytes: usize::MAX,
        ..Limits::default()
    };
    // ~5 MiB decoded estimate: 4/3 of the base64 length.
    let big = "A".repeat(7 * 1024 * 1024);
    let node = Node::Image(Image::new(ImageSource::data(big, "image/png"), "huge"));
    let report = oino_ui::limits::validate_with(&node, &limits);
    assert!(
        report.rejects.iter().any(|d| d.message.contains("blob")),
        "> 4 MiB decoded rejected: {:?}",
        report.rejects
    );

    // ~1.5 MiB decoded: soft advice only.
    let medium = "A".repeat(2 * 1024 * 1024);
    let node = Node::Image(Image::new(ImageSource::data(medium, "image/png"), "meh"));
    let report = oino_ui::limits::validate_with(&node, &limits);
    assert_eq!(report.rejects, Vec::<oino_ui::limits::Diagnostic>::new());
    assert!(report.soft.iter().any(|d| d.message.contains("blob")));
}

#[test]
fn patch_over_128_kib_rejected() {
    let text = "x".repeat(200 * 1024);
    let ops = vec![NodeOp::Set {
        path: Vec::new(),
        props: serde_json::Map::from_iter([("text".to_owned(), Json::String(text))]),
    }];
    let err = validate_patch(&ops).expect_err("oversized patch rejected");
    assert!(err[0].message.contains("128 KiB"));
    let ops = vec![NodeOp::Append {
        path: Vec::new(),
        field: PatchField::Lines,
        items: vec![PatchItem::Item(Json::String("ok".into()))],
    }];
    assert!(validate_patch(&ops).is_ok());
}

#[test]
fn chart_points_cap() {
    let points: Vec<[f64; 2]> = (0..10_001).map(|i| [f64::from(i), 1.0]).collect();
    let node = Node::Chart(
        Chart::new([Dataset::new(points)])
            .x_axis(Axis::bounds(0.0, 1.0))
            .y_axis(Axis::bounds(0.0, 1.0)),
    );
    let err = validate(&node).expect_err("> 10 000 points rejected");
    assert!(err[0].message.contains("10000"));
}

#[test]
fn span_cap_64_kib() {
    let text = "y".repeat(65 * 1024);
    let node = Node::Paragraph(Paragraph::new(text));
    let err = validate(&node).expect_err("64 KiB span rejected");
    assert!(err[0].message.contains("64 KiB"));
}

#[test]
fn custom_limits_are_honored() {
    let node = Node::Paragraph(Paragraph::new("hello"));
    let tiny = Limits {
        max_tree_bytes: 8,
        ..Limits::default()
    };
    let report = oino_ui::limits::validate_with(&node, &tiny);
    assert_ne!(report.rejects, Vec::<oino_ui::limits::Diagnostic>::new());
}

#[test]
fn markdown_cap_256_kib() {
    let node = Node::Markdown(Markdown::new("m".repeat(256 * 1024 + 1)));
    let err = validate(&node).expect_err("oversized markdown rejected");
    assert!(err[0].message.contains("256 KiB"));
}
