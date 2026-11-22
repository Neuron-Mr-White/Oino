use std::fs::{self, OpenOptions};
use std::path::Path;

use serde_json::json;

use super::*;

const HEADER: &str = r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}"#;

fn message(id: &str, parent: Option<&str>, role: &str, text: &str) -> String {
    json!({
        "type": "message",
        "id": id,
        "parentId": parent,
        "timestamp": "2026-01-01T00:00:00.000Z",
        "message": {"role": role, "content": [{"type": "text", "text": text}]},
    })
    .to_string()
}

fn custom(id: &str, parent: Option<&str>) -> String {
    // `id` after a big field, as pi writes custom_message lines.
    json!({
        "type": "custom_message",
        "customType": "x",
        "content": "some text with \"escapes\" and\nnewlines",
        "id": id,
        "parentId": parent,
    })
    .to_string()
}

fn write(path: &Path, lines: &[String]) {
    let mut text = String::from(HEADER);
    text.push('\n');
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    fs::write(path, text).unwrap();
}

fn append(path: &Path, text: &str) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

/// A linear conversation of `turns` turns: user, custom, assistant (ids `u<n>`, `c<n>`, `a<n>`).
fn conversation(turns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut parent: Option<String> = None;
    for n in 0..turns {
        let (u, c, a) = (format!("u{n}"), format!("c{n}"), format!("a{n}"));
        lines.push(message(
            &u,
            parent.as_deref(),
            "user",
            &format!("question {n}"),
        ));
        lines.push(custom(&c, Some(&u)));
        lines.push(message(&a, Some(&c), "assistant", &format!("answer {n}")));
        parent = Some(a);
    }
    lines
}

fn ids(page: &EntryPage) -> Vec<String> {
    page.entries
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn indexes_lines_flags_and_branch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut lines = conversation(2);
    // A fork off a0 (abandoned branch), then the leaf continues from a1.
    lines.push(message("x0", Some("a0"), "user", "abandoned"));
    lines.push(
        json!({"type":"compaction","id":"k0","parentId":"a1","summary":"s","firstKeptEntryId":"u1","tokensBefore":1})
            .to_string(),
    );
    lines.push(message("u2", Some("k0"), "user", "after compaction"));
    write(&path, &lines);
    forget_offsets(&path);
    let index = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(index.entries().len(), 9);
    assert_eq!(index.leaf_id(), Some("u2"));
    assert_eq!(index.last_compaction(), Some(7));
    assert_ne!(index.entries()[0].flags & FLAG_USER, 0);
    assert_eq!(index.entries()[1].flags & FLAG_USER, 0);
    assert_eq!(index.entries()[7].flags, FLAG_COMPACTION);
    let branch: Vec<&str> = index
        .branch()
        .iter()
        .map(|i| index.entries()[*i as usize].id.as_str())
        .collect();
    assert_eq!(branch, ["u0", "c0", "a0", "u1", "c1", "a1", "k0", "u2"]);
    // Offsets point at the exact lines.
    let text = fs::read_to_string(&path).unwrap();
    let pos = &index.entries()[2];
    let line = &text[usize::try_from(pos.offset).unwrap()
        ..usize::try_from(pos.offset).unwrap() + pos.len as usize];
    assert_eq!(line, lines[2]);
}

#[test]
fn tail_page_is_turn_aligned_and_pages_back_to_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &conversation(10)); // 30 entries
    forget_offsets(&path);
    let index = OffsetIndex::load(&path, None).unwrap();
    let request = PageRequest {
        before: None,
        limit: 8,
        max_bytes: u64::MAX,
    };
    let page = index.page(&path, &request).unwrap();
    // 8 entries would start mid-turn (c7): the page starts at the next user message.
    assert_eq!(ids(&page), ["u8", "c8", "a8", "u9", "c9", "a9"]);
    assert_eq!(page.total, 30);
    assert!(page.has_more);
    assert_eq!(page.cursor.as_deref(), Some("u8"));
    assert_eq!(page.leaf_id.as_deref(), Some("a9"));
    // Paging with the cursor walks back to the root without gaps or repeats.
    let mut all = ids(&page);
    let mut cursor = page.cursor.clone();
    let mut pages = 1;
    while let Some(before) = cursor.take() {
        let older = index
            .page(
                &path,
                &PageRequest {
                    before: Some(before),
                    ..request.clone()
                },
            )
            .unwrap();
        let mut chunk = ids(&older);
        chunk.extend(all);
        all = chunk;
        pages += 1;
        if older.has_more {
            cursor = older.cursor.clone();
        } else {
            assert_eq!(older.cursor.as_deref(), Some("u0"));
        }
        assert!(pages < 20);
    }
    let expected: Vec<String> = (0..10)
        .flat_map(|n| [format!("u{n}"), format!("c{n}"), format!("a{n}")])
        .collect();
    assert_eq!(all, expected);
}

#[test]
fn page_bounds_raw_bytes_but_keeps_one_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let big = "x".repeat(10_000);
    let lines = vec![
        message("u0", None, "user", "q"),
        message("a0", Some("u0"), "assistant", &big),
        message("a1", Some("a0"), "assistant", &big),
        message("a2", Some("a1"), "assistant", &big),
    ];
    write(&path, &lines);
    forget_offsets(&path);
    let index = OffsetIndex::load(&path, None).unwrap();
    let page = index
        .page(
            &path,
            &PageRequest {
                before: None,
                limit: 100,
                max_bytes: 25_000,
            },
        )
        .unwrap();
    assert_eq!(ids(&page), ["a1", "a2"]);
    assert!(page.has_more);
    let tiny = index
        .page(
            &path,
            &PageRequest {
                before: None,
                limit: 100,
                max_bytes: 1,
            },
        )
        .unwrap();
    assert_eq!(ids(&tiny), ["a2"]);
    // An unknown cursor is an error, not an empty page.
    assert!(matches!(
        index.page(
            &path,
            &PageRequest {
                before: Some("nope".into()),
                ..PageRequest::default()
            }
        ),
        Err(SessionError::UnknownEntry(_))
    ));
}

#[test]
fn appended_lines_extend_and_a_torn_tail_waits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &conversation(1));
    forget_offsets(&path);
    let first = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(first.entries().len(), 3);
    // A half-written line is not indexed yet.
    let next = message("u1", Some("a0"), "user", "more");
    let (head, tail) = next.split_at(10);
    append(&path, head);
    let torn = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(torn.entries().len(), 3);
    assert_eq!(torn.leaf_id(), Some("a0"));
    append(&path, &format!("{tail}\n"));
    let grown = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(grown.entries().len(), 4);
    assert_eq!(grown.leaf_id(), Some("u1"));
    assert_eq!(grown.entries()[3].parent, Some(2));
    // A rewritten file (fingerprint mismatch) is rebuilt, not extended.
    write(&path, &conversation(2)[..2]);
    let rebuilt = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(rebuilt.entries().len(), 2);
}

#[test]
fn big_files_persist_their_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sessions");
    fs::create_dir_all(root.join("p")).unwrap();
    let path = root.join("p").join("s.jsonl");
    let big = "y".repeat(usize::try_from(PERSIST_MIN).unwrap());
    let mut lines = conversation(3);
    lines.push(message("big", Some("a2"), "assistant", &big));
    write(&path, &lines);
    let offsets = offsets_dir(&root);
    assert_eq!(offsets, dir.path().join(".sessions.oino-offsets"));
    forget_offsets(&path);
    let built = OffsetIndex::load(&path, Some(&offsets)).unwrap();
    let persisted = OffsetIndex::sidecar_path(&path);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !persisted.is_file() && std::time::Instant::now() < deadline {
        OffsetIndex::persist_after_reply(&path);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(persisted.is_file());
    // A cold process (memory cache empty) reads the persisted index.
    forget_offsets(&path);
    let cold = OffsetIndex::load(&path, Some(&offsets)).unwrap();
    assert_eq!(cold.entries(), built.entries());
    assert_eq!(fs::read_dir(root.join("p")).unwrap().count(), 2);
    // A corrupt persisted index is ignored.
    fs::write(&persisted, b"{not json").unwrap();
    forget_offsets(&path);
    let rebuilt = OffsetIndex::load(&path, Some(&offsets)).unwrap();
    assert_eq!(rebuilt.entries(), built.entries());
}

#[test]
fn entries_by_id_and_old_formats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &conversation(3));
    forget_offsets(&path);
    let index = OffsetIndex::load(&path, None).unwrap();
    let found = index
        .entries_by_id(&path, &["a1".into(), "missing".into(), "u0".into()])
        .unwrap();
    let found: Vec<&str> = found.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(found, ["a1", "u0"]);

    let old = dir.path().join("old.jsonl");
    fs::write(
        &old,
        "{\"type\":\"session\",\"version\":2,\"id\":\"s\",\"timestamp\":\"t\",\"cwd\":\"/\"}\n",
    )
    .unwrap();
    assert!(matches!(
        OffsetIndex::load(&old, None),
        Err(SessionError::UnsupportedVersion(2))
    ));
    let junk = dir.path().join("junk.jsonl");
    fs::write(&junk, "not a session\n").unwrap();
    assert!(matches!(
        OffsetIndex::load(&junk, None),
        Err(SessionError::MissingHeader(_))
    ));
}

#[test]
fn latest_checkpoint_is_on_current_branch_and_same_size_rewrite_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let lines = vec![
        message("u0", None, "user", "first"),
        json!({"type":"compaction","id":"c0","parentId":"u0","firstKeptEntryId":"u0","summary":"summary"}).to_string(),
        message("u1", Some("u0"), "user", "other"),
    ];
    write(&path, &lines);
    let index = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(
        index.last_compaction(),
        None,
        "abandoned checkpoint is not current"
    );
    let text = fs::read_to_string(&path).unwrap().replace("first", "other");
    fs::write(&path, text).unwrap();
    let index = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(
        index.page(&path, &PageRequest::default()).unwrap().entries[0]["message"]["content"][0]["text"],
        "other"
    );
    // Corruption that is still syntactically valid JSON fails its checksum.
    forget_offsets(&path);
    fs::write(OffsetIndex::sidecar_path(&path), b"[0,\"{}\"]").unwrap();
    let rebuilt = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(rebuilt.entries(), index.entries());
}

#[test]
fn prefix_rewrite_plus_append_rebuilds_links() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rewritten.jsonl");
    write(
        &path,
        &[
            message("aaaa", None, "user", "first"),
            message("bbbb", Some("aaaa"), "user", "second"),
            message(
                "cccc",
                Some("bbbb"),
                "user",
                "tail padding padding padding padding padding padding padding padding",
            ),
        ],
    );
    OffsetIndex::load(&path, None).unwrap();
    let original = fs::read_to_string(&path).unwrap();
    let rewritten = original.replacen("aaaa", "zzzz", 1);
    fs::write(&path, rewritten).unwrap();
    append(
        &path,
        &(message("dddd", Some("cccc"), "user", "new") + "\n"),
    );
    forget_offsets(&path);
    let cached = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(cached.entries()[0].id, "zzzz");
    assert_eq!(cached.entries()[1].parent, None);
    forget_offsets(&path);
    let _ = fs::remove_file(OffsetIndex::sidecar_path(&path));
    let rebuilt = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(cached.entries(), rebuilt.entries());
}

#[test]
fn preserved_mtime_rewrite_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mtime.jsonl");
    write(&path, &[message("aaaa", None, "user", "first")]);
    OffsetIndex::load(&path, None).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path).unwrap().replace("aaaa", "zzzz");
    fs::write(&path, text).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(time)
        .unwrap();
    assert_eq!(
        OffsetIndex::load(&path, None).unwrap().entries()[0].id,
        "zzzz"
    );
    forget_offsets(&path);
    assert_eq!(
        OffsetIndex::load(&path, None).unwrap().entries()[0].id,
        "zzzz"
    );
}

#[test]
fn atomic_replacement_rebuilds_even_with_matching_size_and_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replace.jsonl");
    write(&path, &[message("aaaa", None, "user", "first")]);
    OffsetIndex::load(&path, None).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let replacement = dir.path().join("new.jsonl");
    write(&replacement, &[message("zzzz", None, "user", "first")]);
    File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_modified(time)
        .unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert_eq!(
        OffsetIndex::load(&path, None).unwrap().entries()[0].id,
        "zzzz"
    );
}

#[test]
fn owned_appends_roll_digest_and_trust_unchanged_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owned.jsonl");
    write(&path, &conversation(3));
    let first = OffsetIndex::load(&path, None).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &OffsetIndex::load(&path, None).unwrap()
    ));
    let before = append_stamp(&path).unwrap();
    append(
        &path,
        &(message("next", first.leaf_id(), "user", "new") + "\n"),
    );
    record_owned_append(&path, before);
    let grown = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(grown.generation, 1);
    assert_eq!(grown.leaf_id(), Some("next"));
    assert_eq!(
        grown.fingerprint,
        fingerprint(&mut File::open(&path).unwrap(), grown.end).unwrap()
    );
    let before = append_stamp(&path).unwrap();
    append(
        &path,
        &(message("last", Some("next"), "user", "tail") + "\n"),
    );
    record_owned_append(&path, before);
    let last = OffsetIndex::load(&path, None).unwrap();
    assert_eq!(last.generation, 2);
    assert_eq!(
        last.fingerprint,
        fingerprint(&mut File::open(&path).unwrap(), last.end).unwrap()
    );
    // Ownership does not exempt subsequent external rewrites from validation.
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path).unwrap().replace("next", "xxxx");
    fs::write(&path, text).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(time)
        .unwrap();
    let rewritten = OffsetIndex::load(&path, None).unwrap();
    assert!(rewritten.position("xxxx").is_some());
    assert!(rewritten.position("next").is_none());
}

#[test]
fn external_edit_after_owned_write_is_not_blessed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edit-window.jsonl");
    // The edit is beyond the first checkpoint and outside the final 4096 bytes.
    write(
        &path,
        &[
            message("root", None, "user", &"x".repeat(5000)),
            message("aaaa", Some("root"), "user", "edit me"),
            message("last", Some("aaaa"), "user", &"y".repeat(5000)),
        ],
    );
    OffsetIndex::load(&path, None).unwrap();
    let before = append_stamp(&path).unwrap();
    append(
        &path,
        &(message("tail", Some("last"), "user", "tail") + "\n"),
    );
    record_owned_append(&path, before);
    let text = fs::read_to_string(&path).unwrap().replace("aaaa", "zzzz");
    fs::write(&path, text).unwrap();
    let cached = OffsetIndex::load(&path, None).unwrap();
    assert!(cached.position("zzzz").is_some());
    assert!(cached.position("aaaa").is_none());
    assert_eq!(
        cached.entries(),
        OffsetIndex::build(&mut File::open(&path).unwrap(), &path)
            .unwrap()
            .entries()
    );
}

#[test]
fn replacement_during_owned_append_rejects_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replace-window.jsonl");
    write(&path, &conversation(2));
    OffsetIndex::load(&path, None).unwrap();
    let before = append_stamp(&path).unwrap();
    let replacement = dir.path().join("replacement");
    write(&replacement, &conversation(3));
    fs::rename(replacement, &path).unwrap();
    record_owned_append(&path, before);
    let stamp = file_stamp(&File::open(&path).unwrap()).unwrap();
    assert!(owned_growth(&path, &stamp).is_none());
    assert_eq!(OffsetIndex::load(&path, None).unwrap().entries().len(), 9);
}

#[test]
#[ignore = "diagnostic throughput: run release with --ignored --nocapture"]
#[allow(clippy::print_stderr, reason = "manual diagnostic measurement")]
fn owned_append_reader_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measure.jsonl");
    write(
        &path,
        &[message("root", None, "user", &"x".repeat(72_000_000))],
    );
    OffsetIndex::load(&path, None).unwrap();
    let mut samples = Vec::new();
    for n in 0..60 {
        let before = append_stamp(&path).unwrap();
        append(
            &path,
            &(message(&format!("tail{n}"), Some("root"), "user", "new") + "\n"),
        );
        record_owned_append(&path, before);
        let start = std::time::Instant::now();
        OffsetIndex::load(&path, None).unwrap();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let _ = writeln!(
        io::stderr(),
        "72MB_append_reader_n=60 median_ms={:.3} p95_ms={:.3} max_ms={:.3}",
        samples[30],
        samples[56],
        samples[59]
    );
}

#[test]
fn sampled_interior_block_rewrite_plus_growth_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sampled.jsonl");
    write(
        &path,
        &[
            message("root", None, "user", &"x".repeat(2 << 20)),
            message("aaaa", Some("root"), "user", "interior"),
            message("last", Some("aaaa"), "user", &"y".repeat(2 << 20)),
        ],
    );
    OffsetIndex::load(&path, None).unwrap();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, original.replace("aaaa", "zzzz")).unwrap();
    append(
        &path,
        &(message("tail", Some("last"), "user", "new") + "\n"),
    );
    let grown = OffsetIndex::load(&path, None).unwrap();
    assert!(grown.position("zzzz").is_some());
    assert!(grown.position("aaaa").is_none());
}

#[test]
fn old_sidecar_version_is_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-cache.jsonl");
    write(&path, &conversation(2));
    let mut old = (*OffsetIndex::load(&path, None).unwrap()).clone();
    old.version = FORMAT_VERSION - 1;
    old.entries.clear();
    write_persisted(&OffsetIndex::sidecar_path(&path), &old).unwrap();
    forget_offsets(&path);
    assert_eq!(OffsetIndex::load(&path, None).unwrap().entries().len(), 6);
}

#[test]
#[ignore = "diagnostic realistic entry-count throughput: release --ignored --nocapture"]
#[allow(clippy::print_stderr, reason = "manual diagnostic measurement")]
fn owned_append_many_entries_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.jsonl");
    let content = "x".repeat(1700);
    let lines: Vec<_> = (0u32..40_000)
        .map(|n| {
            message(
                &format!("entry{n}"),
                n.checked_sub(1).map(|n| format!("entry{n}")).as_deref(),
                "user",
                &content,
            )
        })
        .collect();
    write(&path, &lines);
    // A real retained-range checkpoint must not reintroduce an O(entries) branch walk.
    append(
        &path,
        &(json!({"type":"compaction", "id":"checkpoint", "parentId":"entry39999",
        "firstKeptEntryId":"entry39999", "summary":"summary"})
        .to_string()
            + "\n"),
    );
    OffsetIndex::load(&path, None).unwrap();
    let bytes = fs::metadata(&path).unwrap().len();
    let mut samples = Vec::new();
    for n in 0..60 {
        let before = append_stamp(&path).unwrap();
        append(
            &path,
            &(message(&format!("tail{n}"), Some("checkpoint"), "user", "new") + "\n"),
        );
        record_owned_append(&path, before);
        let start = std::time::Instant::now();
        OffsetIndex::load(&path, None).unwrap();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let _ = writeln!(
        io::stderr(),
        "many_entries_bytes={bytes} entries=40000 n=60 median_ms={:.3} p95_ms={:.3} max_ms={:.3}",
        samples[30],
        samples[56],
        samples[59]
    );
}

/// R6-01: QA's unsampled block-2 rewrite, including derived metadata changes.
#[test]
fn unsampled_rewrite_growth_invalidates_memory_and_persisted_readers() {
    for persisted in [false, true] {
        for growth in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("unsampled.jsonl");
            let mut lines = Vec::new();
            for n in 0..8 {
                lines.push(message(
                    &format!("id{n:02}"),
                    None,
                    "user",
                    &"p".repeat(1 << 20),
                ));
                if n == 2 {
                    lines.push(message("link", Some("id02"), "user", "linked"));
                    lines.push(
                        json!({"type":"compaction", "id":"comp", "parentId":"link",
                        "firstKeptEntryId":"id02", "summary":"summary"})
                        .to_string(),
                    );
                    lines.push(
                        json!({"type":"session_info", "id":"info", "parentId":"comp",
                        "name":"old title"})
                        .to_string(),
                    );
                }
            }
            write(&path, &lines);
            let old = OffsetIndex::load(&path, None).unwrap();
            assert_eq!(old.entries[4].checkpoint, Some(2));
            let time = fs::metadata(&path).unwrap().modified().unwrap();
            let text = fs::read_to_string(&path)
                .unwrap()
                .replace("id02", "zzzz")
                .replace("\"parentId\":\"zzzz\"", "\"parentId\":\"id00\"")
                .replace(
                    "\"firstKeptEntryId\":\"zzzz\"",
                    "\"firstKeptEntryId\":\"id00\"",
                )
                .replace("old title", "new title");
            fs::write(&path, text).unwrap();
            if growth {
                append(
                    &path,
                    &(message("tail", Some("zzzz"), "user", "new") + "\n"),
                );
            }
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
            if persisted {
                write_persisted(&OffsetIndex::sidecar_path(&path), &old).unwrap();
                forget_offsets(&path);
            }
            let cached = OffsetIndex::load(&path, None).unwrap();
            let fresh = OffsetIndex::build(&mut File::open(&path).unwrap(), &path).unwrap();
            assert_eq!(cached.entries(), fresh.entries());
            assert_eq!(cached.branch(), fresh.branch());
            assert_eq!(cached.title(), Some("new title"));
            assert!(cached.position("id02").is_none());
            assert_eq!(cached.entries[3].parent, Some(0));
            assert_eq!(cached.entries[4].checkpoint, Some(0));
        }
    }
}

#[test]
#[ignore = "release full-prefix revalidation measurement"]
#[allow(clippy::print_stderr, reason = "manual diagnostic measurement")]
fn full_revalidation_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("full.jsonl");
    write(
        &path,
        &[message("root", None, "user", &"x".repeat(72_000_000))],
    );
    let index = OffsetIndex::load(&path, None).unwrap();
    drop(index);
    for cold in [false, true] {
        let mut samples = Vec::new();
        for _ in 0..60 {
            forget_offsets(&path);
            if cold {
                // Evict only this fixture, never the machine's global page cache.
                // sync + DONTNEED is advisory; this is not a power-on disk test.
                assert!(std::process::Command::new("python3")
                    .args(["-c", "import os,sys; f=os.open(sys.argv[1],os.O_RDONLY); os.fsync(f); os.posix_fadvise(f,0,0,os.POSIX_FADV_DONTNEED); os.close(f)"])
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success());
            }
            let start = std::time::Instant::now();
            let loaded = OffsetIndex::load(&path, None).unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(loaded.leaf_id(), Some("root"));
        }
        samples.sort_by(f64::total_cmp);
        let _ = writeln!(
            io::stderr(),
            "72MB_disk_load_validation cold={cold} n=60 median_ms={:.3} p95_ms={:.3} max_ms={:.3}",
            samples[30],
            samples[56],
            samples[59]
        );
    }
}

#[test]
fn external_unsampled_edit_before_owned_append_breaks_identity_chain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("before-owned.jsonl");
    let lines: Vec<_> = (0..8)
        .map(|n| message(&format!("id{n:02}"), None, "user", &"p".repeat(1 << 20)))
        .collect();
    write(&path, &lines);
    OffsetIndex::load(&path, None).unwrap();
    let text = fs::read_to_string(&path).unwrap().replace("id02", "zzzz");
    fs::write(&path, text).unwrap();
    let before = append_stamp(&path).unwrap();
    append(
        &path,
        &(message("tail", Some("zzzz"), "user", "new") + "\n"),
    );
    record_owned_append(&path, before);
    let cached = OffsetIndex::load(&path, None).unwrap();
    assert!(cached.position("zzzz").is_some());
    assert_eq!(cached.branch(), vec![2, 8]);
}

/// R7-02: metadata equality is not evidence of unchanged bytes. Model the
/// exact tuple returned by a coarse-stat filesystem without relying on timing.
#[test]
fn equal_metadata_rewrite_validates_memory_and_disk() {
    for disk in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coarse.jsonl");
        write(&path, &conversation(3));
        let mut stale = (*OffsetIndex::load(&path, None).unwrap()).clone();
        let text = fs::read_to_string(&path).unwrap().replace("u2", "zz");
        assert!(text.contains("\"zz\""));
        fs::write(&path, text).unwrap();
        stale.identity = file_stamp(&File::open(&path).unwrap()).unwrap();
        stale.mtime_ns = stale.identity.mtime;
        if disk {
            write_persisted(&OffsetIndex::sidecar_path(&path), &stale).unwrap();
            forget_offsets(&path);
        } else {
            memory_put(&path, Arc::new(stale));
        }
        let loaded = OffsetIndex::load(&path, None).unwrap();
        assert!(loaded.position("u2").is_none());
        assert!(loaded.position("zz").is_some());
    }
}

/// A writer's process-local chain must never authenticate a disk sidecar.
#[test]
fn owned_growth_does_not_trust_deserialized_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk-owned.jsonl");
    write(&path, &conversation(3));
    let mut stale = (*OffsetIndex::load(&path, None).unwrap()).clone();
    stale.fingerprint[0] ^= 1;
    stale.entries[0].id = "corrupt".to_owned();
    write_persisted(&OffsetIndex::sidecar_path(&path), &stale).unwrap();
    let before = append_stamp(&path).unwrap();
    append(&path, &(message("tail", None, "user", "new") + "\n"));
    record_owned_append(&path, before);
    forget_offsets(&path);
    let loaded = OffsetIndex::load(&path, None).unwrap();
    assert!(loaded.position("corrupt").is_none());
    assert_eq!(loaded.leaf_id(), Some("tail"));
}

#[test]
fn deferred_build_hashes_only_complete_bytes_and_waits_for_reply() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deferred.jsonl");
    let mut lines = conversation(3);
    lines.push(message(
        "big",
        Some("a2"),
        "assistant",
        &"x".repeat(5 << 20),
    ));
    write(&path, &lines);
    append(&path, "{incomplete");
    let index = OffsetIndex::load_deferred(&path).unwrap();
    assert_eq!(
        index.fingerprint,
        fingerprint(&mut File::open(&path).unwrap(), index.end).unwrap()
    );
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(!OffsetIndex::sidecar_path(&path).exists());
    // Persist this exact snapshot deterministically; writer scheduling/saturation
    // is covered by big_files_persist_their_index.
    write_persisted(&OffsetIndex::sidecar_path(&path), &index).unwrap();
    forget_offsets(&path);
    let cold = OffsetIndex::load_deferred(&path).unwrap();
    assert_eq!(cold.entries(), index.entries());
    assert_eq!(cold.fingerprint, index.fingerprint);
}

#[test]
fn binary_sidecar_roundtrip_corruption_and_legacy_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wire.jsonl");
    write(&path, &conversation(8));
    let mut index = Arc::unwrap_or_clone(OffsetIndex::load_deferred(&path).unwrap());
    index.entries[0].pending = vec!["tool\u{0}☃".into()];
    index.entries[0].custom_type = Some("custom".into());
    index.entries[0].compaction_summary_id = Some("summary".into());
    index.by_id.take();
    index.title = Some("title\n☃".into());
    index.moved_cwd = Some("/tmp/project".into());
    let bytes = binary::encode(&index);
    assert_eq!(binary::decode(&bytes), Some(index.clone()));
    for end in 0..bytes.len() {
        assert!(binary::decode(&bytes[..end]).is_none());
    }
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(binary::decode(&corrupt).is_none());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(binary::decode(&trailing).is_none());
    // Historical JSON envelope remains readable and the next snapshot migrates it.
    let sidecar = OffsetIndex::sidecar_path(&path);
    let payload = serde_json::to_string(&index).unwrap();
    fs::write(
        &sidecar,
        serde_json::to_vec(&(fnv1a(payload.as_bytes()), payload)).unwrap(),
    )
    .unwrap();
    assert_eq!(read_persisted(&sidecar), Some(index.clone()));
    write_persisted(&sidecar, &index).unwrap();
    assert!(fs::read(&sidecar).unwrap().starts_with(binary::MAGIC));
    assert_eq!(read_persisted(&sidecar), Some(index));
}

#[test]
#[ignore = "real fixture codec/load diagnostic"]
fn persisted_fixture_measurement() {
    let source = std::env::var("OINO_BENCH_SESSION").expect("set OINO_BENCH_SESSION");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.jsonl");
    fs::copy(source, &path).unwrap();
    let index = OffsetIndex::load_deferred(&path).unwrap();
    write_persisted(&OffsetIndex::sidecar_path(&path), &index).unwrap();
    let bytes = fs::read(OffsetIndex::sidecar_path(&path)).unwrap();
    let mut decode = Vec::new();
    let mut load = Vec::new();
    for _ in 0..60 {
        let start = std::time::Instant::now();
        let decoded = binary::decode(&bytes).unwrap();
        decode.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(decoded.entries(), index.entries());
        forget_offsets(&path);
        let start = std::time::Instant::now();
        let loaded = OffsetIndex::load_deferred(&path).unwrap();
        load.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(loaded.entries(), index.entries());
    }
    decode.sort_by(f64::total_cmp);
    load.sort_by(f64::total_cmp);
    let _ = writeln!(
        io::stderr(),
        "fixture bytes={} sidecar={} decode_median_ms={} decode_p95_ms={} load_median_ms={} load_p95_ms={}",
        index.size,
        bytes.len(),
        decode[30],
        decode[56],
        load[30],
        load[56]
    );
}
