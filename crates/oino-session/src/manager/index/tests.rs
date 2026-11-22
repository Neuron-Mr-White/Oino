#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use super::*;

const ID: &str = "019fcbea-df52-7491-b190-eed3ca0edd5a";

fn header(id: &str, cwd: &str) -> String {
    format!(
        r#"{{"type":"session","version":3,"id":"{id}","timestamp":"2026-08-04T08:36:35.282Z","cwd":"{cwd}"}}"#
    )
}

fn user(n: usize, text: &str) -> String {
    format!(
        r#"{{"type":"message","id":"u{n}","parentId":null,"timestamp":"2026-08-04T09:{:02}:00.000Z","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}],"timestamp":1}}}}"#,
        n % 60
    )
}

fn assistant(n: usize, ts: &str, pad: usize) -> String {
    format!(
        r#"{{"type":"message","id":"a{n}","parentId":null,"timestamp":"{ts}","message":{{"role":"assistant","content":[{{"type":"text","text":"{}"}}]}}}}"#,
        "x".repeat(pad)
    )
}

fn name(text: &str) -> String {
    format!(
        r#"{{"type":"session_info","id":"n1","parentId":null,"timestamp":"2026-08-04T09:00:00.000Z","name":"{text}"}}"#
    )
}

fn write_lines(path: &Path, lines: &[String]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut text = lines.join("\n");
    text.push('\n');
    fs::write(path, text).unwrap();
}

fn append(path: &Path, line: &str) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{line}").unwrap();
}

fn session_path(root: &Path, cwd: &str, id: &str) -> PathBuf {
    root.join(format!(
        "--{}--",
        cwd.trim_start_matches('/').replace('/', "-")
    ))
    .join(format!("2026-08-04T08-36-35-282Z_{id}.jsonl"))
}

fn basic(path: &Path) {
    write_lines(
        path,
        &[
            header(ID, "/work/a"),
            user(1, "hello there"),
            assistant(2, "2026-08-04T10:00:00.000Z", 10),
            name("My chat"),
            r#"{"type":"custom_message","id":"c1","parentId":null,"timestamp":"2026-08-04T10:00:01.000Z","customType":"oino.cwd_change","content":"moved","display":true,"details":{"cwd":"/work/b"}}"#.to_owned(),
        ],
    );
}

#[test]
fn card_matches_the_full_parse() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/a", ID);
    basic(&path);
    let index = CardIndex::new(dir.path().join(INDEX_FILE_NAME));
    let card = index
        .cards(std::slice::from_ref(&path), true)
        .pop()
        .unwrap();
    let full = crate::manager::session_info(&path).unwrap();
    assert_eq!(card.id, full.id);
    assert_eq!(card.cwd, full.cwd);
    assert_eq!(card.moved_cwd.as_deref(), Some("/work/b"));
    assert_eq!(card.moved_cwd, full.moved_cwd);
    assert_eq!(card.name, full.name);
    assert_eq!(card.message_count, full.message_count);
    assert_eq!(card.first_message, "hello there");
    assert_eq!(card.created, full.created);
    // The last activity: the assistant's entry timestamp.
    assert_eq!(
        crate::time::system_time_ms(card.modified),
        IsoTimestamp::from_text("2026-08-04T10:00:00.000Z")
            .to_ms()
            .unwrap()
    );
    assert!(index.path().is_file(), "the index is written");
}

#[test]
fn not_a_session_is_skipped_and_cached() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("--x--").join("junk.jsonl");
    write_lines(&path, &[r#"{"type":"message"}"#.into()]);
    let index = CardIndex::new(dir.path().join(INDEX_FILE_NAME));
    assert_eq!(index.cards(std::slice::from_ref(&path), true).len(), 0);
    assert!(
        lock(&index.state)
            .entries
            .get(&path)
            .is_some_and(|e| e.card.is_none())
    );
}

#[test]
fn unchanged_file_is_a_hit_and_changes_invalidate() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/a", ID);
    basic(&path);
    let index = CardIndex::new(dir.path().join(INDEX_FILE_NAME));
    let files = vec![path.clone()];
    assert_eq!(
        index.cards(&files, true)[0].name.as_deref(),
        Some("My chat")
    );

    // Same size, same mtime, different bytes: the index answers (no read).
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("My chat", "Xy chat");
    fs::write(&path, text).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_eq!(
        index.cards(&files, true)[0].name.as_deref(),
        Some("My chat")
    );

    // A new mtime: rescanned.
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime + Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        index.cards(&files, true)[0].name.as_deref(),
        Some("Xy chat")
    );

    // Appended (incremental scan from the indexed end).
    append(&path, &name("Renamed"));
    append(&path, &assistant(9, "2026-08-05T00:00:00.000Z", 3));
    let card = index.cards(&files, true).pop().unwrap();
    assert_eq!(card.name.as_deref(), Some("Renamed"));
    assert_eq!(card.message_count, 3);
    assert_eq!(card.moved_cwd.as_deref(), Some("/work/b"));
    assert_eq!(card.first_message, "hello there");
    assert_eq!(
        crate::time::system_time_ms(card.modified),
        IsoTimestamp::from_text("2026-08-05T00:00:00.000Z")
            .to_ms()
            .unwrap()
    );

    // Rewritten shorter: a full rescan.
    write_lines(&path, &[header(ID, "/work/c"), user(1, "again")]);
    let card = index.cards(&files, true).pop().unwrap();
    assert_eq!(card.cwd, "/work/c");
    assert_eq!(card.name, None);
    assert_eq!(card.moved_cwd, None);
    assert_eq!(card.message_count, 1);

    // Rewritten longer with other bytes before the indexed end: the fingerprint
    // catches it, rescanned rather than appended to.
    write_lines(
        &path,
        &[
            header(ID, "/work/d"),
            user(1, "other"),
            name("Fresh"),
            user(2, "more"),
        ],
    );
    let card = index.cards(&files, true).pop().unwrap();
    assert_eq!(card.cwd, "/work/d");
    assert_eq!(card.first_message, "other");
    assert_eq!(card.message_count, 2);

    // Deleted: gone from the list and pruned from the index.
    fs::remove_file(&path).unwrap();
    assert_eq!(index.cards(&[], true).len(), 0);
    assert!(lock(&index.state).entries.is_empty());
}

#[test]
fn index_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/a", ID);
    basic(&path);
    let file = dir.path().join(INDEX_FILE_NAME);
    CardIndex::new(file.clone()).cards(std::slice::from_ref(&path), true);
    // Same size + mtime with other bytes: a fresh process answers from the file.
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("My chat", "Xy chat");
    fs::write(&path, text).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let card = CardIndex::new(file).cards(&[path], true).pop().unwrap();
    assert_eq!(card.name.as_deref(), Some("My chat"));
}

#[test]
fn corrupt_or_foreign_index_is_ignored_and_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/a", ID);
    basic(&path);
    let file = dir.path().join(INDEX_FILE_NAME);
    for junk in [
        b"{not json".to_vec(),
        br#"{"version":999,"entries":{}}"#.to_vec(),
        br#"{"version":1,"entries":{"x":{"size":"no"}}}"#.to_vec(),
        vec![0xff, 0xfe, 0x00],
    ] {
        fs::write(&file, &junk).unwrap();
        let cards = CardIndex::new(file.clone()).cards(std::slice::from_ref(&path), true);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name.as_deref(), Some("My chat"));
        assert!(read_index(&file).is_some(), "rewritten as a valid index");
    }
}

#[test]
fn concurrent_writers_never_tear_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    for n in 0..40 {
        let id = format!("019fcbea-df52-7491-b190-{n:012}");
        let path = session_path(dir.path(), "/work/a", &id);
        write_lines(&path, &[header(&id, "/work/a"), user(n, "hi")]);
        files.push(path);
    }
    let file = dir.path().join(INDEX_FILE_NAME);
    let files = Arc::new(files);
    // Separate instances = separate processes sharing only the file.
    let writers: Vec<_> = (0..8)
        .map(|w| {
            let files = Arc::clone(&files);
            let file = file.clone();
            std::thread::spawn(move || {
                for round in 0..15 {
                    let index = CardIndex::new(file.clone());
                    let subset: Vec<PathBuf> = files
                        .iter()
                        .enumerate()
                        .filter(|(n, _)| (n + w + round) % 3 != 0)
                        .map(|(_, p)| p.clone())
                        .collect();
                    let cards = index.cards(&subset, true);
                    assert_eq!(cards.len(), subset.len());
                    // A reader at any moment sees a whole index (or none yet).
                    if let Ok(bytes) = fs::read(&file) {
                        assert!(serde_json::from_slice::<IndexFile>(&bytes).is_ok());
                    }
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert!(read_index(&file).is_some());
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            Path::new(&e.file_name())
                .extension()
                .is_some_and(|x| x == "tmp")
        })
        .collect();
    assert_eq!(leftovers.len(), 0, "temp files left: {leftovers:?}");
}

#[test]
fn big_file_reads_head_and_tail_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/big", ID);
    let mut lines = vec![
        header(ID, "/work/big"),
        name("Big one"),
        user(1, "the first ask"),
    ];
    let line = assistant(0, "2026-08-04T10:00:00.000Z", 2000);
    let count = usize::try_from((FULL_SCAN_MAX + (2 << 20)) / line.len() as u64).unwrap();
    for _ in 0..count {
        lines.push(line.clone());
    }
    lines.push(assistant(1, "2026-09-01T00:00:00.000Z", 10));
    write_lines(&path, &lines);
    assert!(fs::metadata(&path).unwrap().len() > FULL_SCAN_MAX);
    let index = CardIndex::new(dir.path().join(INDEX_FILE_NAME));
    let card = index
        .cards(std::slice::from_ref(&path), true)
        .pop()
        .unwrap();
    assert_eq!(card.name.as_deref(), Some("Big one"));
    assert_eq!(card.first_message, "the first ask");
    assert_eq!(
        crate::time::system_time_ms(card.modified),
        IsoTimestamp::from_text("2026-09-01T00:00:00.000Z")
            .to_ms()
            .unwrap()
    );
    let real = count + 2;
    assert!(
        card.message_count < real,
        "a lower bound: {}",
        card.message_count
    );
    let entry = lock(&index.state).entries[&path].clone();
    assert!(!entry.card.unwrap().exact);
    // Appending to a big file extends it without a rescan.
    append(&path, &name("Big renamed"));
    let card = index
        .cards(std::slice::from_ref(&path), true)
        .pop()
        .unwrap();
    assert_eq!(card.name.as_deref(), Some("Big renamed"));
}

#[test]
fn huge_first_user_line_still_gives_its_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/a", ID);
    let image = "A".repeat(PARSE_KEEP + 10);
    let first = format!(
        r#"{{"type":"message","id":"u1","parentId":null,"timestamp":"2026-08-04T09:00:00.000Z","message":{{"role":"user","content":[{{"type":"text","text":"look \"here\""}},{{"type":"image","data":"{image}"}}]}}}}"#
    );
    write_lines(&path, &[header(ID, "/w"), first]);
    let card = CardIndex::new(dir.path().join(INDEX_FILE_NAME))
        .cards(std::slice::from_ref(&path), true)
        .pop()
        .unwrap();
    assert_eq!(card.first_message, "look \"here\"");
    assert_eq!(card.message_count, 1);
}

#[test]
fn lines_in_another_key_order_are_parsed() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/a", ID);
    write_lines(
        &path,
        &[
            r#"{"id":"019fcbea-df52-7491-b190-eed3ca0edd5a","type":"session","version":3,"timestamp":"2026-08-04T08:36:35.282Z","cwd":"/w"}"#.into(),
            r#"{"id":"u1","type":"message","message":{"role":"user","content":"plain text","timestamp":1754300000000}}"#.into(),
            r#"{"name":"Odd","type":"session_info","id":"n"}"#.into(),
        ],
    );
    let card = CardIndex::new(dir.path().join(INDEX_FILE_NAME))
        .cards(std::slice::from_ref(&path), true)
        .pop()
        .unwrap();
    assert_eq!(card.id, ID);
    assert_eq!(card.first_message, "plain text");
    assert_eq!(card.name.as_deref(), Some("Odd"));
    assert_eq!(
        crate::time::system_time_ms(card.modified),
        1_754_300_000_000
    );
}

#[test]
fn find_by_id_uses_names_never_contents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sessions");
    let a = session_path(&root, "/work/a", "019fcbea-aaaa-7491-b190-000000000001");
    let b = session_path(&root, "/work/b", "019fcbea-bbbb-7491-b190-000000000002");
    write_lines(
        &a,
        &[
            header("019fcbea-aaaa-7491-b190-000000000001", "/work/a"),
            user(1, "a"),
        ],
    );
    write_lines(
        &b,
        &[
            header("019fcbea-bbbb-7491-b190-000000000002", "/work/b"),
            user(1, "b"),
        ],
    );
    // A file whose name carries no id.
    let odd = root.join("--work-c--").join("imported.jsonl");
    write_lines(&odd, &[header("custom-odd-id", "/work/c"), user(1, "c")]);

    let found = find_card(&root, "019fcbea-bbbb-7491-b190-000000000002")
        .unwrap()
        .unwrap();
    assert_eq!(found.path, b);
    assert_eq!(find_card(&root, "019fcbea-aaaa").unwrap().unwrap().path, a);
    assert!(
        matches!(find_card(&root, "019fcbea"), Err(SessionError::AmbiguousId(p)) if p.len() == 2)
    );
    assert_eq!(
        find_card(&root, "custom-odd-id").unwrap().unwrap().path,
        odd
    );
    assert_eq!(
        find_card(&root, a.to_str().unwrap()).unwrap().unwrap().id,
        "019fcbea-aaaa-7491-b190-000000000001"
    );
    assert!(find_card(&root, "/elsewhere/x.jsonl").unwrap().is_none());
    assert!(
        find_card(&root, "abc").unwrap().is_none(),
        "short prefixes never match"
    );

    // A missing id: answered from names alone, fast.
    let start = Instant::now();
    assert!(
        find_card(&root, "019fcbea-dead-7491-b190-00000000beef")
            .unwrap()
            .is_none()
    );
    assert!(
        start.elapsed() < Duration::from_millis(50),
        "{:?}",
        start.elapsed()
    );

    // A file renamed so its name lies: found through the index.
    let _ = list_all_cards(&root);
    let liar = root
        .join("--work-a--")
        .join("2026-08-04T08-36-35-282Z_not-the-id.jsonl");
    write_lines(
        &liar,
        &[header("019fcbea-cccc-7491-b190-000000000003", "/work/a")],
    );
    let _ = list_all_cards(&root);
    assert_eq!(
        find_card(&root, "019fcbea-cccc-7491-b190-000000000003")
            .unwrap()
            .unwrap()
            .path,
        liar
    );
}

#[test]
fn archived_copies_with_a_number_are_found() {
    let dir = tempfile::tempdir().unwrap();
    let trash = dir.path().join("trash");
    let path = trash.join(format!("2026-08-04T08-36-35-282Z_{ID}.1.jsonl"));
    write_lines(&path, &[header(ID, "/w"), user(1, "x")]);
    assert_eq!(find_card_in_dir(&trash, ID).unwrap().unwrap().path, path);
    assert_eq!(list_dir_cards(&trash).len(), 1);
    assert!(id_matches(&format!("{ID}.12"), ID));
    assert!(!id_matches(&format!("{ID}x"), ID));
    assert!(!id_matches(&format!("{ID}.x1"), ID));
}

#[test]
fn listing_is_newest_first_and_skips_dot_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let old = session_path(root, "/a", "019f-old");
    let new = session_path(root, "/b", "019f-new");
    write_lines(
        &old,
        &[
            header("019f-old", "/a"),
            assistant(1, "2026-01-01T00:00:00.000Z", 1),
        ],
    );
    write_lines(
        &new,
        &[
            header("019f-new", "/b"),
            assistant(1, "2026-02-01T00:00:00.000Z", 1),
        ],
    );
    write_lines(
        &root.join(".legacy").join("x_019f-legacy.jsonl"),
        &[header("019f-legacy", "/c")],
    );
    let ids: Vec<String> = list_all_cards(root).into_iter().map(|i| i.id).collect();
    assert_eq!(ids, ["019f-new", "019f-old"]);
    let _ = SystemTime::now();
}

#[test]
fn spaced_json_lines_are_classified() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/a", ID);
    let image = "A".repeat(PARSE_KEEP + 10);
    write_lines(
        &path,
        &[
            format!(r#"{{"type": "session", "version": 3, "id": "{ID}", "timestamp": "2026-08-04T08:36:35.282Z", "cwd": "/sp"}}"#),
            format!(r#"{{"type": "message", "id": "u1", "parentId": null, "timestamp": "2026-08-04T09:00:00.000Z", "message": {{"role": "user", "content": [{{"type": "text", "text": "spaced ask"}}, {{"type": "image", "data": "{image}"}}]}}}}"#),
            r#"{"type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-08-04T11:00:00.000Z", "message": {"role": "assistant", "content": "ok"}}"#.into(),
            r#"{"type": "session_info", "id": "n", "name": "Spaced"}"#.into(),
        ],
    );
    let card = CardIndex::new(dir.path().join(INDEX_FILE_NAME))
        .cards(std::slice::from_ref(&path), true)
        .pop()
        .unwrap();
    assert_eq!(card.cwd, "/sp");
    assert_eq!(card.first_message, "spaced ask");
    assert_eq!(card.name.as_deref(), Some("Spaced"));
    assert_eq!(card.message_count, 2);
    assert_eq!(
        crate::time::system_time_ms(card.modified),
        IsoTimestamp::from_text("2026-08-04T11:00:00.000Z")
            .to_ms()
            .unwrap()
    );
}

#[test]
fn refine_reads_sampled_files_whole_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(dir.path(), "/work/big", ID);
    let small = session_path(dir.path(), "/work/small", "019f-small");
    write_lines(&small, &[header("019f-small", "/work/small"), user(1, "s")]);
    let filler = assistant(0, "2026-08-04T10:00:00.000Z", 2000);
    let half = usize::try_from(FULL_SCAN_MAX / filler.len() as u64).unwrap();
    let mut lines = vec![header(ID, "/work/big"), name("Early"), user(1, "ask")];
    lines.extend(std::iter::repeat_n(filler.clone(), half));
    // A rename in the unread middle.
    lines.push(name("Middle"));
    lines.extend(std::iter::repeat_n(filler, half));
    write_lines(&path, &lines);
    let index = CardIndex::new(dir.path().join(INDEX_FILE_NAME));
    let files = vec![path.clone(), small.clone()];
    let sampled = index.cards(&files, true);
    assert_eq!(
        sampled[0].name.as_deref(),
        Some("Early"),
        "the middle is not read"
    );

    let mut changed = Vec::new();
    assert_eq!(
        index.refine(|info| changed.push(info)),
        1,
        "only the sampled file"
    );
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].name.as_deref(), Some("Middle"));
    assert_eq!(changed[0].message_count, 2 * half + 1);
    let listed = index.cards(&files, true);
    assert_eq!(listed[0].name.as_deref(), Some("Middle"));
    assert_eq!(listed[0].message_count, 2 * half + 1);
    // Exact now: nothing left to refine, and appends keep it exact.
    assert_eq!(index.refine(|_| panic!("nothing changed")), 0);
    append(&path, &user(2, "more"));
    assert_eq!(index.cards(&files, true)[0].message_count, 2 * half + 2);
    assert_eq!(index.refine(|_| {}), 0);
    // A fresh process reads the refined card from the file.
    let again = CardIndex::new(dir.path().join(INDEX_FILE_NAME)).cards(&files, true);
    assert_eq!(again[0].name.as_deref(), Some("Middle"));
}

#[test]
fn shared_index_lives_beside_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let trash = dir.path().join("trash");
    let path = trash.join(format!("2026-08-04T08-36-35-282Z_{ID}.jsonl"));
    write_lines(&path, &[header(ID, "/w"), user(1, "x")]);
    assert_eq!(list_dir_cards(&trash).len(), 1);
    assert_eq!(
        card_index_path(&trash),
        dir.path().join(".trash.oino-cards.json")
    );
    assert!(card_index_path(&trash).is_file());
    let names: Vec<_> = fs::read_dir(&trash)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names.len(), 1, "only the session file: {names:?}");
}
