//! Card-index performance (`list_all_cards` / `find_card`) on a big store: 2,000
//! generated session files, three of them 100 MB. Ignored by default (it writes ~400 MB);
//! run with
//!
//! ```text
//! cargo test -p oino-session --release --test perf_cards -- --ignored --nocapture
//! ```
//!
//! `OINO_PERF_SESSIONS_ROOT=<sessions root>` measures an existing store instead
//! (read-only apart from its `.oino-cards.json`; point it at a copy, never `~/.oino`).
//! `OINO_PERF_COMPARE=1` also times the old full-parse `list_all`.
//!
//! Budgets are generous (debug builds, loaded machines): the targets are a warm list
//! < 300 ms, a cold one < 5 s and a missing id < 50 ms in release.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a perf test asserts by unwrapping and prints its numbers"
)]

use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oino_session::SessionManager;

const FILES: usize = 2_000;
const BIG_FILES: usize = 3;
const BIG_BYTES: u64 = 100 << 20;

fn write_session(path: &Path, id: &str, cwd: &str, turns: usize, pad: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path).unwrap());
    writeln!(
        out,
        r#"{{"type":"session","version":3,"id":"{id}","timestamp":"2026-08-04T08:36:35.282Z","cwd":"{cwd}"}}"#
    )
    .unwrap();
    writeln!(out, r#"{{"type":"session_info","id":"n0","parentId":null,"timestamp":"2026-08-04T08:36:36.000Z","name":"Chat {id}"}}"#).unwrap();
    let pad = "x".repeat(pad);
    for turn in 0..turns {
        writeln!(
            out,
            r#"{{"type":"message","id":"u{turn}","parentId":null,"timestamp":"2026-08-04T09:00:00.000Z","message":{{"role":"user","content":[{{"type":"text","text":"question {turn}"}}],"timestamp":1754298000000}}}}"#
        )
        .unwrap();
        writeln!(
            out,
            r#"{{"type":"message","id":"a{turn}","parentId":"u{turn}","timestamp":"2026-08-04T09:00:01.000Z","message":{{"role":"assistant","content":[{{"type":"text","text":"{pad}"}}],"timestamp":1754298001000}}}}"#
        )
        .unwrap();
    }
    out.flush().unwrap();
}

fn generate(root: &Path) {
    for n in 0..FILES {
        let id = format!("019fcbea-df52-7491-b190-{n:012}");
        let cwd = format!("/work/project-{}", n % 140);
        let dir = root.join(format!("--work-project-{}--", n % 140));
        let path = dir.join(format!("2026-08-04T08-36-35-282Z_{id}.jsonl"));
        if n < BIG_FILES {
            // ~100 MB: 4 KB answers.
            let turns = usize::try_from(BIG_BYTES / 4_300).unwrap();
            write_session(&path, &id, &cwd, turns, 4_096);
        } else {
            // Mixed small/medium sessions (2 KB – 2 MB).
            let turns = 2 + (n * 7919) % 120;
            let pad = 200 + (n * 104_729) % 8_000;
            write_session(&path, &id, &cwd, turns, pad);
        }
    }
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    let took = start.elapsed();
    println!("{label}: {took:?}");
    (out, took)
}

#[test]
#[ignore = "writes ~400 MB of sessions; run explicitly (see the module docs)"]
fn cards_list_and_lookup_budgets() {
    let generated;
    let root: PathBuf = if let Some(root) = std::env::var_os("OINO_PERF_SESSIONS_ROOT") {
        PathBuf::from(root)
    } else {
        generated = tempfile::tempdir().unwrap();
        let root = generated.path().join("sessions");
        time("generate", || generate(&root));
        root
    };
    let _ = fs::remove_file(oino_session::manager::card_index_path(&root));

    let (cold, cold_took) = time("cold list_all_cards", || {
        SessionManager::list_all_cards(&root)
    });
    let (warm, warm_took) = time("warm list_all_cards", || {
        SessionManager::list_all_cards(&root)
    });
    assert_eq!(cold.len(), warm.len());
    println!("sessions: {}", warm.len());
    if std::env::var_os("OINO_PERF_SESSIONS_ROOT").is_none() {
        assert_eq!(warm.len(), FILES);
    }
    let (_, missing_took) = time("find_card (missing id)", || {
        SessionManager::find_card(&root, "019fcbea-dead-7491-b190-00000000beef").unwrap()
    });
    let some = &warm[warm.len() / 2];
    let (found, found_took) = time("find_card (existing id)", || {
        SessionManager::find_card(&root, &some.id).unwrap()
    });
    assert_eq!(found.unwrap().path, some.path);
    let (refined, _) = time(
        "refine (background pass, reads sampled big files whole)",
        || oino_session::manager::refine_all_cards(&root, |_| {}),
    );
    println!("refined files: {refined}");
    let (warm, _) = time("warm list_all_cards after refine", || {
        SessionManager::list_all_cards(&root)
    });
    if std::env::var_os("OINO_PERF_COMPARE").is_some() {
        let (full, _) = time("old list_all (full parse)", || {
            SessionManager::list_all(&root, None, None).unwrap()
        });
        // How far the bounded cards are from the full parse.
        let by_path: std::collections::HashMap<&Path, &oino_session::SessionInfo> = warm
            .iter()
            .map(|info| (info.path.as_path(), info))
            .collect();
        let (mut names, mut moved, mut firsts, mut counts, mut times) = (0, 0, 0, 0, 0);
        for info in &full {
            let Some(card) = by_path.get(info.path.as_path()) else {
                println!("missing card: {}", info.path.display());
                continue;
            };
            names += usize::from(card.name != info.name);
            moved += usize::from(card.moved_cwd != info.moved_cwd);
            let first = |text: &str| text.chars().take(600).collect::<String>();
            firsts += usize::from(card.first_message != first(&info.first_message));
            counts += usize::from(card.message_count != info.message_count);
            let skew = card
                .modified
                .duration_since(info.modified)
                .or_else(|_| info.modified.duration_since(card.modified))
                .unwrap_or_default();
            times += usize::from(skew > Duration::from_secs(2));
            if std::env::var_os("OINO_PERF_VERBOSE").is_some()
                && (card.name != info.name
                    || card.first_message != info.first_message
                    || skew > Duration::from_secs(120))
            {
                let first = |text: &str| text.chars().take(50).collect::<String>();
                println!(
                    "{}\n  name {:?} vs {:?}\n  first {:?} vs {:?}\n  skew {skew:?} count {} vs {}",
                    info.path.display(),
                    card.name,
                    info.name,
                    first(&card.first_message),
                    first(&info.first_message),
                    card.message_count,
                    info.message_count
                );
            }
        }
        println!(
            "vs full parse of {}: name differs {names}, moved cwd {moved}, first message {firsts}, \
             message count {counts}, activity > 2 s off {times}",
            full.len()
        );
    }
    // Release targets ×4 (debug / CI headroom).
    assert!(cold_took < Duration::from_secs(20), "cold {cold_took:?}");
    assert!(
        warm_took < Duration::from_millis(1_200),
        "warm {warm_took:?}"
    );
    assert!(
        missing_took < Duration::from_millis(200),
        "missing {missing_took:?}"
    );
    assert!(
        found_took < Duration::from_millis(200),
        "found {found_took:?}"
    );
}
