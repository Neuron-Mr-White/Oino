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

/// A 1 000-node panel: a `List` of 1 000 items (items count as nodes, §4.8).
fn panel() -> Node {
    let items: Vec<serde_json::Value> = (0..1_000)
        .map(|index| {
            serde_json::json!({
                "key": format!("task-{index}"),
                "content": format!("task {index}: render a row of the panel"),
                "detail": "open · p1",
                "badge": "new"
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "t": "List",
        "highlightSymbol": "› ",
        "items": items
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
