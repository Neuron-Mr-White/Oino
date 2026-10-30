//! Render cost of a 1 000-node v2 panel (WP-U4 acceptance: ≤ 2 ms per frame).
//!
#![allow(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "the bench reports its measurement on stdout"
)]
//!
//! Run with `cargo bench -p oino-tui --bench ui_render`. The timing assertion is skipped in
//! debug builds; `cargo bench` uses the optimized bench profile.

use std::time::Instant;

use oino_tui::render::{measure_v2_node, render_v2_node};
use oino_tui::theme::DARK;
use oino_ui::node::Node;

/// A 1 000-node panel: 170 rows of a `Layout` with three nodes (Block title + Paragraph +
/// `KeyHints`) ≈ 1 000 nodes.
fn panel() -> Node {
    let mut children = Vec::new();
    for index in 0..170 {
        children.push(serde_json::json!({
            "t": "Layout",
            "constraints": [{"Length": 1}, {"Fill": 1}],
            "children": [
                {"t": "Block", "title": format!("row {index}"), "borders": ["TOP"],
                 "child": {"t": "Paragraph", "text": format!("row {index} body text")}},
                {"t": "KeyHints", "hints": [{"keys": "r", "label": "refresh"},
                                            {"keys": "enter", "label": "open"}]}
            ]
        }));
    }
    let constraints: Vec<serde_json::Value> = (0..children.len())
        .map(|_| serde_json::json!({"Length": 3}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "t": "Layout",
        "constraints": constraints,
        "children": children
    }))
    .expect("panel tree parses")
}

fn main() {
    let node = panel();
    let width = 120;
    let height = measure_v2_node(&node, width, &DARK).min(300);
    // Warm up caches and the allocator.
    for _ in 0..5 {
        let _ = render_v2_node(&node, width, height, &DARK);
    }
    let mut samples = Vec::new();
    for _ in 0..50 {
        let start = Instant::now();
        let lines = render_v2_node(&node, width, height, &DARK);
        let elapsed = start.elapsed();
        std::hint::black_box(lines);
        samples.push(elapsed);
    }
    samples.sort();
    let median = samples[samples.len() / 2];
    let p95 = samples[samples.len() * 95 / 100];
    println!(
        "1000-node panel: median {:.2} ms, p95 {:.2} ms at {width}x{height}",
        median.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3
    );
    if !cfg!(debug_assertions) {
        assert!(
            median.as_secs_f64() * 1e3 <= 2.0,
            "median render cost {:.2} ms exceeds the 2 ms budget",
            median.as_secs_f64() * 1e3
        );
    }
}
