#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "standalone benchmark fixture"
)]
//! `cargo run --release -p oino-session --example wake_bench -- <file> [runs]`
//! Operates on a private copy, never changes the supplied fixture.
use oino_session::manager::{OffsetIndex, PageRequest, forget_offsets};
use oino_session::{CreateOptions, SessionManager};
use std::path::Path;
use std::time::Instant;

fn main() {
    let source = std::env::args().nth(1).expect("session file");
    let runs: usize = std::env::args()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.jsonl");
    std::fs::copy(&source, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let t = Instant::now();
    let parsed = oino_session::migrate::parse_session_text(&text)
        .unwrap()
        .unwrap();
    let parse = t.elapsed();
    let full =
        SessionManager::in_memory(Path::new("/tmp"), CreateOptions::default(), parsed.entries);
    let t = Instant::now();
    let expected = full.build_session_context();
    println!(
        "full_parse_ms={:.3} full_context_ms={:.3} entries={} bytes={}",
        parse.as_secs_f64() * 1000.,
        t.elapsed().as_secs_f64() * 1000.,
        full.entry_count(),
        text.len()
    );
    for run in 0..runs {
        forget_offsets(&path);
        if run == 0 {
            let _ = std::fs::remove_file(OffsetIndex::sidecar_path(&path));
        }
        let t = Instant::now();
        let index = OffsetIndex::load(&path, None).unwrap();
        let index_ms = t.elapsed().as_secs_f64() * 1000.;
        let page = index
            .page(
                &path,
                &PageRequest {
                    limit: 80,
                    max_bytes: 8 << 20,
                    ..PageRequest::default()
                },
            )
            .unwrap();
        let page_ms = t.elapsed().as_secs_f64() * 1000.;
        forget_offsets(&path); // Includes persisted-index load in every wake.
        let t = Instant::now();
        let manager = SessionManager::open(&path, dir.path(), None).unwrap();
        let open_ms = t.elapsed().as_secs_f64() * 1000.;
        let context = manager.build_session_context();
        let context_ms = t.elapsed().as_secs_f64() * 1000.;
        assert_eq!(context, expected, "lazy/full context parity");
        assert_eq!(manager.leaf_id(), full.leaf_id());
        println!(
            "run={run} persisted={} index_ms={index_ms:.3} last_page_ms={page_ms:.3} open_ms={open_ms:.3} open_context_ms={context_ms:.3} page_entries={} messages={}",
            run > 0,
            page.entries.len(),
            context.messages.len()
        );
    }
}
