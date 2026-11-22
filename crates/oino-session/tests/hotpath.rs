//! Projection/frontier parity and isolated bookkeeping cost (not provider encoding).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "tests"
)]
use oino_ai::{AgentMessage, UserContentField, UserMessage};
use oino_session::{
    CompactionDraft, CreateOptions, EditableContent, EntryIndex, EntryKind, SessionManager,
};
use std::{hint::black_box, path::Path, time::Instant};

fn user(n: usize) -> AgentMessage {
    AgentMessage::User(UserMessage {
        author: None,
        content: UserContentField::Text(format!("message {n}")),
        timestamp: i64::try_from(n).unwrap(),
    })
}
fn manager() -> SessionManager {
    SessionManager::in_memory(
        Path::new("/tmp/p1a-fixture"),
        CreateOptions::default(),
        vec![],
    )
}
fn parity(m: &SessionManager) {
    let index = EntryIndex::new(m.entries().to_vec());
    assert_eq!(
        m.session_projection(),
        &oino_session::build_session_projection(&index, m.leaf_id())
    );
    assert_eq!(m.branch(None), m.branch(m.leaf_id()));
    let branch = m.branch(None);
    let first = branch
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Custom {
                custom_type,
                data: Some(data),
            } if custom_type == "oino.sent-frontier" => branch
                .iter()
                .position(|e| Some(e.id.as_str()) == data["entryId"].as_str())
                .map(|n| n + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    assert_eq!(
        m.unsent_ids(),
        branch[first..].iter().map(|e| e.id.clone()).collect()
    );
}
#[test]
fn compact_cache_ranges_share_one_payload_and_preserve_wire_projection() {
    let mut m = manager();
    for n in 0..100 {
        m.append_message(user(n)).unwrap();
    }
    let cached = m.session_projection();
    let mut end = 0;
    for entry in &cached.entries {
        assert_eq!(entry.message_range.start, end);
        let slice = cached.entry_messages(entry);
        assert_eq!(
            slice.as_ptr(),
            cached.messages[entry.message_range.clone()].as_ptr()
        );
        end = entry.message_range.end;
    }
    assert_eq!(end, cached.messages.len());
    let expected = oino_session::build_session_projection(m.index(), m.leaf_id());
    assert_eq!(
        serde_json::to_value(m.build_session_projection()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn clearing_idle_caches_preserves_entries_leaf_and_context() {
    let mut m = manager();
    for n in 0..100 {
        m.append_message(user(n)).unwrap();
    }
    m.mark_sent(&[user(99)]).unwrap();
    let projection = m.build_session_projection();
    let entries = m.entries().to_vec();
    let leaf = m.leaf_id().cloned();
    let unsent = m.unsent_ids().clone();
    m.clear_caches();
    assert_eq!(m.entries(), entries);
    assert_eq!(m.leaf_id(), leaf.as_ref());
    assert_eq!(m.build_session_projection(), projection);
    assert_eq!(m.unsent_ids(), unsent);
}

#[test]
fn cached_projection_tracks_mutations_and_forks() {
    let mut m = manager();
    for n in 0..80 {
        parity(&m);
        let id = m.append_message(user(n)).unwrap();
        if n % 7 == 0 {
            m.mark_sent(&[user(n)]).unwrap();
        }
        if n % 11 == 0 {
            m.append_thinking_level_change("high").unwrap();
            m.set_leaf(&id).unwrap();
            parity(&m);
            m.append_message(user(1000 + n)).unwrap();
        }
        if n % 13 == 0 {
            m.reset_leaf();
        }
        parity(&m);
        parity(&m.clone());
    }
}
#[test]
#[ignore = "release CPU guard: cargo test -p oino-session --release --test hotpath -- --ignored --nocapture"]
fn incremental_bookkeeping_is_history_independent() {
    fn measure(size: usize) -> f64 {
        let mut m = manager();
        for n in 0..size {
            m.append_message(user(n)).unwrap();
        }
        // Warm caches; model encoding and full message copies are intentionally excluded.
        black_box(m.session_projection());
        m.mark_sent(&[user(size - 1)]).unwrap();
        black_box(m.unsent_ids());
        let start = Instant::now();
        for n in size..size + 2000 {
            m.append_message(user(n)).unwrap();
            black_box(m.session_projection());
            m.mark_sent(&[user(n)]).unwrap();
            black_box(m.unsent_ids());
        }
        let ns = start.elapsed().as_secs_f64() * 1e9 / 2000.;
        println!("history={size} bookkeeping_ns_per_round={ns:.0}");
        ns
    }
    let small = measure(100);
    let large = measure(40000);
    println!("ratio={:.3}", large / small);
    assert!(
        large <= small * 2.,
        "history dependent bookkeeping: {small} -> {large}"
    );
}

#[test]
fn cached_projection_tracks_edits_and_compaction() {
    let mut m = manager();
    let a = m.append_message(user(1)).unwrap();
    let b = m.append_message(user(2)).unwrap();
    parity(&m);
    m.append_context_edit(&a, Some(EditableContent::Text("edited".into())))
        .unwrap();
    parity(&m);
    m.append_message(user(3)).unwrap();
    parity(&m);
    m.append_compaction(CompactionDraft {
        summary: "summary".into(),
        first_kept_entry_id: Some(b.clone()),
        tokens_before: 100,
        ..Default::default()
    })
    .unwrap();
    parity(&m);
    m.append_message(user(4)).unwrap();
    parity(&m);
    m.set_leaf(&b).unwrap();
    parity(&m);
    m.append_context_edit(&b, None).unwrap();
    parity(&m);
}

/// Includes real JSONL writes and the derived-index update, unlike the CPU-only guard.
#[test]
#[ignore = "release persisted append guard"]
fn persisted_append_is_history_independent() {
    fn measure(size: usize) -> f64 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let entries: Vec<_> = (0..size)
            .map(|n| {
                serde_json::json!({
                    "type":"message", "id":format!("e{n}"),
                    "parentId": n.checked_sub(1).map(|p| format!("e{p}")),
                    "timestamp":"2026-01-01T00:00:00.000Z",
                    "message":{"role":"user","content":"fixture", "timestamp":1}
                })
                .to_string()
            })
            .collect();
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                serde_json::json!({
                    "type":"session","version":3,"id":"fixture",
                    "timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"
                }),
                entries.join("\n")
            ),
        )
        .unwrap();
        let mut m = SessionManager::open(&path, dir.path(), None).unwrap();
        // Warm the append handle and amortized Vec/HashMap growth independently
        // of the steady-state per-entry cost being guarded.
        m.append_message(user(size)).unwrap();
        let start = Instant::now();
        for n in 1..1001 {
            m.append_message(user(size + n)).unwrap();
        }
        let ns = start.elapsed().as_secs_f64() * 1e9 / 1000.0;
        m.sync().unwrap();
        eprintln!("persisted append {size}: {ns:.0} ns/append");
        ns
    }
    // Interleaved independent fixtures distinguish scaling from scheduler stalls.
    let mut small = Vec::new();
    let mut large = Vec::new();
    let mut ratios = Vec::new();
    for _ in 0..7 {
        let baseline = measure(100);
        let history = measure(40_000);
        small.push(baseline);
        large.push(history);
        ratios.push(history / baseline);
    }
    ratios.sort_by(f64::total_cmp);
    small.sort_by(f64::total_cmp);
    large.sort_by(f64::total_cmp);
    let (small, large) = (small[3], large[3]);
    eprintln!("median persisted append: 100={small:.0}, 40k={large:.0} ns/append");
    eprintln!("paired median ratio={:.3}", ratios[3]);
    assert!(
        ratios[3] <= 2.0,
        "100={small:.0}, 40k={large:.0} ns/append, paired median ratio={:.3}",
        ratios[3]
    );
}
