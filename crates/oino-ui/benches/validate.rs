//! WP-U1 acceptance: `validate()` ≤ 1 ms on a 256 KiB tree.

#![allow(
    missing_docs,
    reason = "a bench file: the criterion macro generates undocumented items"
)]

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use oino_ui::limits::validate;
use oino_ui::node::Node;
use oino_ui::prelude::*;

/// A ~256 KiB tree of realistic shape: a layout of lists, a table, a log and a
/// chart, padded to just under the §4.8 byte bound.
fn build_256k_tree() -> Node {
    let filler: String = "lorem ipsum dolor sit amet ".repeat(36); // ~1 KiB
    let list_items: Vec<ListItem> = (0..50_u32)
        .map(|i| {
            ListItem::new(format!("task {i} — {filler}"))
                .key(format!("t-{i}"))
                .detail(format!("open · owner {i}"))
                .badge("P1")
        })
        .collect();
    let rows: Vec<Row> = (0..50_u32)
        .map(|i| Row::new([format!("row {i}"), filler.clone()]).key(format!("r-{i}")))
        .collect();
    let log: Log = Log::new(std::iter::repeat_n(filler.as_str(), 40));
    let paragraphs: Vec<Node> = (0..20)
        .map(|i| Node::Paragraph(Paragraph::new(format!("{i}: {filler}{filler}"))))
        .collect();

    let mut root = Layout::vertical([
        Constraint::Length(1),
        Constraint::Percentage(40),
        Constraint::Fill(1),
        Constraint::Min(20),
        Constraint::Length(3),
    ])
    .children([
        Node::Paragraph(Paragraph::new(Line::from(vec![
            "Board ".bold(),
            Span::from("50 open").fg(Color::Muted),
        ]))),
        Node::List(List::new(list_items).on_activate("open")),
        Node::Table(
            Table::new(rows, [Constraint::Length(20), Constraint::Fill(1)])
                .header(Row::new(["name", "detail"])),
        ),
        Node::Layout(
            Layout::vertical(std::iter::repeat_n(Constraint::Fill(1), 20)).children(paragraphs),
        ),
        Node::Log(log),
        Node::Chart(
            Chart::new([Dataset::new((0..500).map(|i| {
                #[allow(
                    clippy::cast_lossless,
                    reason = "kept explicit to mirror the point type"
                )]
                [f64::from(i), (i % 97) as f64]
            }))])
            .x_axis(Axis::bounds(0.0, 500.0))
            .y_axis(Axis::bounds(0.0, 97.0)),
        ),
    ])
    .into();

    // Pad with 2 KiB paragraphs until the encoded tree sits just under the bound.
    loop {
        let size = oino_ui::limits::encoded_size(&root);
        if size + 3 * 1024 > 256 * 1024 {
            break;
        }
        if let Node::Layout(layout) = &mut root {
            layout
                .children
                .push(Node::Paragraph(Paragraph::new(filler.repeat(2))));
        }
    }
    let size = oino_ui::limits::encoded_size(&root);
    assert!(
        size <= 256 * 1024,
        "bench tree must fit the §4.8 bound, got {size}"
    );
    root
}

/// Benchmarks [`validate`] on a ~256 KiB tree (WP-U1 acceptance).
fn bench_validate(c: &mut Criterion) {
    let tree = build_256k_tree();
    let size = oino_ui::limits::encoded_size(&tree);
    let mut group = c.benchmark_group("validate");
    group.throughput(criterion::Throughput::Bytes(
        u64::try_from(size).unwrap_or(u64::MAX),
    ));
    group.bench_function(format!("validate_256kib_tree_{size}B"), |b| {
        b.iter(|| {
            let result = validate(black_box(&tree));
            assert!(result.is_ok(), "bench tree must validate: {result:?}");
        });
    });
    group.finish();
}

criterion_group!(benches, bench_validate);
criterion_main!(benches);
