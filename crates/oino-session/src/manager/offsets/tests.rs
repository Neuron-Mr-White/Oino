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
        lines.push(message(&u, parent.as_deref(), "user", &format!("question {n}")));
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
    assert_eq!(index.entries()[0].flags, FLAG_USER);
    assert_eq!(index.entries()[1].flags, 0);
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
    let line = &text[pos.offset as usize..pos.offset as usize + pos.len as usize];
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
            .page(&path, &PageRequest { before: Some(before), ..request.clone() })
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
        index.page(&path, &PageRequest { before: Some("nope".into()), ..PageRequest::default() }),
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
    let big = "y".repeat(PERSIST_MIN as usize);
    let mut lines = conversation(3);
    lines.push(message("big", Some("a2"), "assistant", &big));
    write(&path, &lines);
    let offsets = offsets_dir(&root);
    assert_eq!(offsets, dir.path().join(".sessions.oino-offsets"));
    forget_offsets(&path);
    let built = OffsetIndex::load(&path, Some(&offsets)).unwrap();
    let files: Vec<_> = fs::read_dir(&offsets).unwrap().collect();
    assert_eq!(files.len(), 1);
    // A cold process (memory cache empty) reads the persisted index.
    forget_offsets(&path);
    let cold = OffsetIndex::load(&path, Some(&offsets)).unwrap();
    assert_eq!(cold.entries(), built.entries());
    // The sessions tree holds only the session file.
    assert_eq!(fs::read_dir(root.join("p")).unwrap().count(), 1);
    // A corrupt persisted index is ignored.
    let persisted = files[0].as_ref().unwrap().path();
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
