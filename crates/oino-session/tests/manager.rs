//! `SessionManager` behaviour against real files in temp dirs (session-v3 §6, acceptance 4).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use oino_ai::{
    AgentMessage, AssistantMessage, StopReason, SystemMessage, UserContentField, UserMessage,
    UserTextField,
};
use oino_session::manager::encode_cwd;
use oino_session::{
    BranchSummaryDraft, CompactionDraft, CreateOptions, EditableContent, EntryId, EntryKind,
    FileLine, ProgressSink, SessionError, SessionManager,
};

const CWD: &str = "/work/project";

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        author: None,
        content: UserContentField::Text(text.into()),
        timestamp: 1,
    })
}

fn assistant(text: &str) -> AgentMessage {
    let mut message = AssistantMessage::empty("faux", "faux", "faux-1", 2);
    message.content = vec![oino_ai::AssistantContent::text(text)];
    message.stop_reason = StopReason::Stop;
    AgentMessage::Assistant(message)
}

fn system(content: &str) -> AgentMessage {
    AgentMessage::System(SystemMessage {
        content: UserTextField::Text(content.into()),
        ..SystemMessage::default()
    })
}

fn lines(path: &Path) -> Vec<FileLine> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| FileLine::parse(l).unwrap())
        .collect()
}

#[test]
fn file_appears_only_with_conversation_then_appends_lines() {
    let root = common::tempdir();
    let mut sm =
        SessionManager::create(Path::new(CWD), root.path(), CreateOptions::default()).unwrap();
    let path = sm.session_file().unwrap().to_path_buf();
    assert_eq!(path.parent().unwrap(), root.path().join("--work-project--"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.ends_with(&format!("_{}.jsonl", sm.session_id())),
        "{name}"
    );
    assert_eq!(&name[..24], sm.header().timestamp.file_stem());

    sm.append_model_change("faux", "faux-1").unwrap();
    sm.append_message(system("prompt")).unwrap();
    assert!(!path.exists() && sm.is_persisted() && !sm.is_flushed());
    sm.append_message(user("hi")).unwrap();
    assert!(path.exists() && sm.is_flushed());
    assert_eq!(lines(&path).len(), 4);
    sm.append_thinking_level_change("high").unwrap();
    let written = lines(&path);
    assert_eq!(written.len(), 5);
    assert!(matches!(written[0], FileLine::Header(_)));
    let reopened = SessionManager::open(&path, root.path(), None).unwrap();
    assert_eq!(reopened.entries(), sm.entries());
    assert_eq!(reopened.leaf_id(), sm.leaf_id());
    assert_eq!(reopened.build_session_context().thinking_level, "high");
}

#[test]
fn ids_names_labels_and_tree() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    assert!(!sm.is_persisted() && sm.session_file().is_none());
    let u = sm.append_message(user("hi")).unwrap();
    assert_eq!(u.as_str().len(), 8);
    let a = sm.append_message(assistant("yo")).unwrap();
    assert_eq!(sm.entry(&a).unwrap().parent_id.as_ref(), Some(&u));
    sm.append_session_info(Some("  Line\r\n\none ")).unwrap();
    assert_eq!(sm.session_name().as_deref(), Some("Line one"));
    sm.append_label_change(&u, Some("start")).unwrap();
    assert_eq!(sm.label(&u), Some("start"));
    sm.append_label_change(&u, Some("")).unwrap();
    assert_eq!(sm.label(&u), None);
    sm.append_label_change(&a, Some("answer")).unwrap();
    assert!(matches!(
        sm.append_label_change(&"nope".into(), Some("x")),
        Err(SessionError::UnknownEntry(_))
    ));

    sm.set_leaf(&u).unwrap();
    let alt = sm.append_message(assistant("alt")).unwrap();
    assert_eq!(sm.children(Some(&u)).len(), 2);
    let tree = sm.tree();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].children.len(), 2);
    assert_eq!(tree[0].children[0].label.as_deref(), Some("answer"));
    assert_eq!(
        sm.branch(None)
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        [u.clone(), alt]
    );

    sm.reset_leaf();
    assert_eq!(sm.build_session_context().messages, []);
    let root2 = sm.append_message(user("second root")).unwrap();
    assert_eq!(sm.entry(&root2).unwrap().parent_id, None);
    assert_eq!(sm.children(None).len(), 2);
    assert!(matches!(
        sm.set_leaf(&"missing".into()),
        Err(SessionError::UnknownEntry(_))
    ));
    assert!(matches!(
        sm.append_message(oino_session::messages::compaction_summary_message(
            "s", 1, 0
        )),
        Err(SessionError::InvalidAppend(_))
    ));
}

#[test]
fn deep_trees_do_not_overflow() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    for i in 0..20_000 {
        sm.append_custom_entry("tick", Some(serde_json::json!(i)))
            .unwrap();
    }
    let tree = sm.tree();
    assert_eq!(tree.len(), 1);
    drop(tree);
    assert_eq!(sm.branch(None).len(), 20_000);
}

/// Runs `work` on a 256 KiB stack: the `GetTree` worker's. The old nested
/// tree serialization recursed one serde frame per entry and overflowed it
/// on long chats (3918 entries); the flat wire form must not.
fn on_a_256kib_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn deep_linear_tree_serializes_on_a_small_stack() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    for i in 0..5000 {
        sm.append_custom_entry("tick", Some(serde_json::json!(i)))
            .unwrap();
    }
    let root = sm.entries()[0].id.clone();
    sm.append_label_change(&root, Some("root")).unwrap();
    let flat = sm.tree_flat();
    assert_eq!(flat.len(), 5001); // the chain plus its label entry
    assert_eq!(flat[1].entry.parent_id.as_ref(), Some(&flat[0].entry.id));
    assert_eq!(flat[0].label.as_deref(), Some("root"));
    // The exact GetTree shape: `json!` (to_value) and the string.
    let (value, text) = on_a_256kib_stack(move || {
        let value = serde_json::to_value(&flat).unwrap();
        let text = serde_json::to_string(&flat).unwrap();
        (value, text)
    });
    assert_eq!(value.as_array().map(Vec::len), Some(5001));
    assert!(text.contains("\"label\":\"root\""));
    assert_eq!(text.matches("\"entry\":").count(), 5001);
}

#[test]
fn branched_tree_serializes_on_a_small_stack() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    let mut chain = Vec::new();
    for i in 0..2500 {
        chain.push(
            sm.append_custom_entry("tick", Some(serde_json::json!(i)))
                .unwrap(),
        );
    }
    // A side branch off every chain entry: 5000 entries, forks throughout,
    // 2500 deep.
    for id in &chain {
        sm.set_leaf(id).unwrap();
        sm.append_custom_entry("tick", Some(serde_json::json!("side")))
            .unwrap();
    }
    let flat = sm.tree_flat();
    assert_eq!(flat.len(), 5000);
    let (value, text) = on_a_256kib_stack(move || {
        let value = serde_json::to_value(&flat).unwrap();
        let text = serde_json::to_string(&flat).unwrap();
        (value, text)
    });
    assert_eq!(value.as_array().map(Vec::len), Some(5000));
    assert_eq!(text.matches("\"entry\":").count(), 5000);
}

#[test]
fn compaction_stores_validated_state_without_system_checkpoint() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    sm.append_message(system("base")).unwrap();
    sm.append_message(user("a")).unwrap();
    sm.append_message(assistant("A")).unwrap();
    let keep = sm.append_message(user("next")).unwrap();
    let id = sm
        .append_compaction(CompactionDraft {
            summary: "sum".into(),
            first_kept_entry_id: Some(keep.clone()),
            tokens_before: 9,
            ..Default::default()
        })
        .unwrap();
    let EntryKind::CompactionState(state) = &sm.entry(&id).unwrap().kind else {
        panic!("not v3 state")
    };
    assert_eq!(state.anchor_message.id, keep);
    let wire = serde_json::to_value(sm.entry(&id).unwrap()).unwrap();
    assert_eq!(wire["type"], "compaction_state");
    assert!(wire.get("systemMessage").is_none());
    assert!(sm.append_compaction(CompactionDraft::default()).is_err());
}

#[test]
fn context_edit_rules() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    let u = sm.append_message(user("orig")).unwrap();
    let a = sm.append_message(assistant("long")).unwrap();
    let info = sm.append_session_info(Some("n")).unwrap();
    assert!(matches!(
        sm.append_context_edit(&info, None),
        Err(SessionError::InvalidAppend(_))
    ));
    assert!(matches!(
        sm.append_context_edit(&"x".into(), None),
        Err(SessionError::UnknownEntry(_))
    ));
    let edit = sm
        .append_context_edit(&a, Some(EditableContent::Text("short".into())))
        .unwrap();
    let wire = serde_json::to_value(sm.entry(&edit).unwrap()).unwrap();
    assert_eq!(
        wire["replacement"],
        serde_json::json!({"content": [{"type": "text", "text": "short"}]})
    );
    sm.set_leaf(&u).unwrap();
    let side = sm.append_message(assistant("side")).unwrap();
    assert!(matches!(
        sm.append_context_edit(&a, None),
        Err(SessionError::InvalidAppend(_))
    ));
    sm.append_context_edit(&side, None).unwrap();
    assert_eq!(sm.build_session_context().messages.len(), 1);
}

#[test]
fn branch_summary_and_branched_sessions() {
    let root = common::tempdir();
    let mut sm =
        SessionManager::create(Path::new(CWD), root.path(), CreateOptions::default()).unwrap();
    let u = sm.append_message(user("start")).unwrap();
    sm.append_label_change(&u, Some("lbl")).unwrap();
    let a = sm.append_message(assistant("a")).unwrap();
    sm.append_message(user("explore")).unwrap();
    let from = sm.leaf_id().cloned().unwrap();
    let summary = sm
        .branch_with_summary(
            Some(&u),
            BranchSummaryDraft {
                from_id: from.clone(),
                summary: "explored".into(),
                details: None,
                from_hook: None,
                usage: None,
            },
        )
        .unwrap();
    assert_eq!(sm.entry(&summary).unwrap().parent_id.as_ref(), Some(&u));
    let roles: Vec<&str> = sm
        .build_session_context()
        .messages
        .iter()
        .map(AgentMessage::role)
        .collect();
    assert_eq!(roles, ["user", "branchSummary"]);

    let clone = sm.create_branched_session(&a).unwrap();
    let cloned = SessionManager::open(&clone, root.path(), None).unwrap();
    assert_eq!(
        cloned.header().parent_session.as_deref(),
        sm.session_file().and_then(Path::to_str)
    );
    assert_eq!(cloned.entries().len(), 3, "u, a, re-emitted label");
    assert_eq!(cloned.label(&u), Some("lbl"));
    assert_ne!(cloned.session_id(), sm.session_id());

    let original_file = sm.session_file().unwrap().to_path_buf();
    let new_path = sm.branch_into_new_session(&a).unwrap().unwrap();
    assert_ne!(new_path, original_file);
    assert!(new_path.exists() && sm.is_flushed());
    assert_eq!(sm.leaf_id(), Some(&sm.entries().last().unwrap().id));

    let mut memory =
        SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    let m = memory.append_message(user("x")).unwrap();
    assert!(memory.create_branched_session(&m).is_err());
    assert_eq!(memory.branch_into_new_session(&m).unwrap(), None);
    assert_eq!(memory.entries().len(), 1);
}

struct Counter(std::sync::Mutex<Vec<(usize, usize)>>);
impl ProgressSink for Counter {
    fn progress(&self, done: usize, total: usize) {
        self.0.lock().unwrap().push((done, total));
    }
}

#[test]
fn listing_find_fork_and_continue() {
    let root = common::tempdir();
    let mut first = SessionManager::create(
        Path::new(CWD),
        root.path(),
        CreateOptions {
            id: Some("abcd-1111".into()),
            parent_session: None,
        },
    )
    .unwrap();
    first.append_message(user("first question")).unwrap();
    first.append_message(assistant("first answer")).unwrap();
    first.append_session_info(Some("Named")).unwrap();
    let mut second = SessionManager::create(
        Path::new(CWD),
        root.path(),
        CreateOptions {
            id: Some("abcd-2222".into()),
            parent_session: None,
        },
    )
    .unwrap();
    second
        .append_message(AgentMessage::User(UserMessage {
            author: None,
            content: UserContentField::Text("second".into()),
            timestamp: 9_999_999_999_999,
        }))
        .unwrap();
    let mut other = SessionManager::create(
        Path::new("/elsewhere"),
        root.path(),
        CreateOptions::default(),
    )
    .unwrap();
    other.append_message(user("elsewhere")).unwrap();

    let counter = Counter(std::sync::Mutex::new(Vec::new()));
    let listed = SessionManager::list(Path::new(CWD), root.path(), Some(&counter), None).unwrap();
    assert_eq!(
        listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["abcd-2222", "abcd-1111"]
    );
    assert_eq!(listed[1].name.as_deref(), Some("Named"));
    assert_eq!(listed[1].message_count, 2);
    assert_eq!(listed[1].first_message, "first question");
    assert_eq!(listed[1].all_messages_text, "first question first answer");
    assert_eq!(counter.0.lock().unwrap().last(), Some(&(2, 2)));
    assert_eq!(
        SessionManager::list_all(root.path(), None, None)
            .unwrap()
            .len(),
        3
    );
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        SessionManager::list_all(root.path(), None, Some(&cancel)),
        Err(SessionError::Cancelled)
    ));

    let find = |id: &str| SessionManager::find_by_id(Path::new(CWD), id, root.path());
    assert_eq!(find("abcd-1111").unwrap().as_deref(), first.session_file());
    assert_eq!(find("abcd-2").unwrap().as_deref(), second.session_file());
    assert!(matches!(find("abcd"), Err(SessionError::AmbiguousId(paths)) if paths.len() == 2));
    assert_eq!(find("abc").unwrap(), None);
    assert_eq!(
        find(other.session_id()).unwrap().as_deref(),
        other.session_file(),
        "falls back to all directories"
    );

    let forked = SessionManager::fork_from(
        first.session_file().unwrap(),
        Path::new("/target"),
        root.path(),
        CreateOptions::default(),
    )
    .unwrap();
    assert_eq!(forked.entries(), first.entries());
    assert_eq!(forked.cwd(), Path::new("/target"));
    assert!(
        forked
            .session_file()
            .unwrap()
            .starts_with(root.path().join(encode_cwd(Path::new("/target"))))
    );
    assert_eq!(
        forked.header().parent_session.as_deref(),
        first.session_file().and_then(Path::to_str)
    );

    let recent = SessionManager::continue_recent(Path::new("/elsewhere"), root.path()).unwrap();
    assert_eq!(recent.session_id(), other.session_id());
    let fresh = SessionManager::continue_recent(Path::new("/nothing-here"), root.path()).unwrap();
    assert!(fresh.entries().is_empty() && !fresh.is_flushed());
}

#[test]
fn open_edge_cases() {
    let root = common::tempdir();
    let dir = root.path();
    let v1 = dir.join("v1.jsonl");
    fs::write(
        &v1,
        "{\"type\":\"session\",\"id\":\"s1\",\"timestamp\":\"2025-01-01T00:00:00.000Z\",\"cwd\":\"/w\"}\n{\"type\":\"message\",\"timestamp\":\"2025-01-01T00:00:01.000Z\",\"message\":{\"role\":\"user\",\"content\":\"a\",\"timestamp\":1}}\n",
    )
    .unwrap();
    let sm = SessionManager::open(&v1, dir, None).unwrap();
    assert_eq!(sm.header().version, 3);
    assert_eq!(sm.cwd(), Path::new("/w"));
    assert_eq!(
        fs::read_to_string(&v1).unwrap(),
        "{\"type\":\"session\",\"id\":\"s1\",\"timestamp\":\"2025-01-01T00:00:00.000Z\",\"cwd\":\"/w\"}\n{\"type\":\"message\",\"timestamp\":\"2025-01-01T00:00:01.000Z\",\"message\":{\"role\":\"user\",\"content\":\"a\",\"timestamp\":1}}\n",
        "migration must not replace source bytes"
    );
    assert!(!dir.join("v1.jsonl.tmp").exists());

    let future = dir.join("future.jsonl");
    fs::write(
        &future,
        "{\"type\":\"session\",\"version\":4,\"id\":\"s\",\"timestamp\":\"t\",\"cwd\":\"/w\"}\n",
    )
    .unwrap();
    assert!(matches!(
        SessionManager::open(&future, dir, None),
        Err(SessionError::UnsupportedVersion(4))
    ));
    let garbage = dir.join("garbage.jsonl");
    fs::write(&garbage, "not json\n").unwrap();
    assert!(matches!(
        SessionManager::open(&garbage, dir, Some(Path::new("/w"))),
        Err(SessionError::MissingHeader(_))
    ));
    let empty = dir.join("empty.jsonl");
    fs::write(&empty, "").unwrap();
    let opened = SessionManager::open(&empty, dir, Some(Path::new("/w"))).unwrap();
    assert!(opened.is_flushed());
    assert_eq!(lines(&empty).len(), 1, "header written");
    let missing = dir.join("new.jsonl");
    let mut fresh = SessionManager::open(&missing, dir, Some(Path::new("/w"))).unwrap();
    fresh.append_message(user("x")).unwrap();
    assert_eq!(fresh.session_file(), Some(missing.as_path()));
    assert_eq!(lines(&missing).len(), 2);

    let err = SessionManager::create(
        Path::new(CWD),
        dir,
        CreateOptions {
            id: Some("-bad".into()),
            parent_session: None,
        },
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character"
    );
}

#[test]
fn legacy_directory_migration() {
    let root = common::tempdir();
    let sessions = root.path();
    for name in ["rich-v2.jsonl", "real-v1.jsonl", "real-v2-model-only.jsonl"] {
        fs::copy(
            common::fixtures().join("oino-legacy").join(name),
            sessions.join(name),
        )
        .unwrap();
    }
    let migrated = SessionManager::migrate_legacy_dir(sessions).unwrap();
    assert_eq!(migrated.len(), 3);
    assert_eq!(
        migrated.iter().filter(|m| m.path.is_some()).count(),
        2,
        "model-only session is not migrated"
    );
    for item in &migrated {
        assert!(item.legacy_path.starts_with(sessions.join(".legacy")));
        if let Some(path) = &item.path {
            assert!(path.starts_with(sessions.join("--work-project--")));
            let sm = SessionManager::open(path, sessions, None).unwrap();
            assert!(sm.session_name().is_some());
        }
    }
    assert_eq!(SessionManager::migrate_legacy_dir(sessions).unwrap(), []);
    assert_eq!(
        SessionManager::list(Path::new(CWD), sessions, None, None)
            .unwrap()
            .len(),
        2
    );

    // Opening a legacy file directly migrates it too.
    fs::copy(
        common::fixtures().join("oino-legacy/v1-replaces.jsonl"),
        sessions.join("direct.jsonl"),
    )
    .unwrap();
    let sm = SessionManager::open(&sessions.join("direct.jsonl"), sessions, None).unwrap();
    assert_eq!(sm.header().id, "aaaaaaaa-2222-4333-8444-555555555555");
    assert!(
        sm.session_file()
            .unwrap()
            .starts_with(sessions.join("--work-project--"))
    );
    assert!(!sessions.join("direct.jsonl").exists());
    let _ = EntryId::new("unused");
}

// ── D-WRITER / D-FLUSH (core spec §2.1, WP-C1) ──

/// How many open file descriptors point at `file` (Linux `/proc/self/fd`).
#[cfg(target_os = "linux")]
fn fd_count(file: &Path) -> usize {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            path.read_link()
                .ok()
                .and_then(|target| target.to_str().map(str::to_owned))
        })
        .filter(|target| {
            let prefix = file.display().to_string();
            target.as_str() == prefix.as_str() || target.starts_with(&format!("{prefix} "))
        })
        .count()
}

/// D-WRITER: one open append handle per session file, reused for every appended line.
#[cfg(target_os = "linux")]
#[test]
fn appends_reuse_one_open_handle() {
    let root = common::tempdir();
    let mut sm =
        SessionManager::create(Path::new(CWD), root.path(), CreateOptions::default()).unwrap();
    for i in 0..3 {
        sm.append_message(user(&format!("m{i}"))).unwrap();
    }
    let file = sm.session_file().unwrap().to_path_buf();
    assert_eq!(
        fd_count(&file),
        1,
        "first flush leaves the append handle open"
    );
    for i in 0..3 {
        sm.append_message(assistant(&format!("a{i}"))).unwrap();
    }
    assert_eq!(
        fd_count(&file),
        1,
        "appends reuse the handle instead of reopening per line"
    );
    sm.sync().unwrap();
    assert_eq!(fd_count(&file), 1, "sync uses the same handle");
    assert_eq!(sm.entry_count(), 6, "six appended messages");
    drop(sm);
    assert_eq!(fd_count(&file), 0, "handle closes with the manager");
}

/// D-FLUSH: `sync()` is fail-closed — a session whose file cannot be persisted reports the
/// I/O error at the barrier instead of pretending durability.
#[test]
fn sync_fails_closed_when_file_cannot_be_written() {
    let root = common::tempdir();
    let mut sm =
        SessionManager::create(Path::new(CWD), root.path(), CreateOptions::default()).unwrap();
    // No conversation yet: nothing to make durable, and the first flush is retried lazily.
    sm.sync().unwrap();
    sm.append_message(user("u1")).unwrap();
    let file = sm.session_file().unwrap().to_path_buf();
    sm.sync().unwrap();

    // Writer invalidated (reset_leaf per D-WRITER) + file gone: sync must report Io.
    sm.reset_leaf();
    fs::remove_file(&file).unwrap();
    let err = sm.sync().unwrap_err();
    assert!(matches!(err, SessionError::Io { .. }), "{err}");
    // A later append also fails closed instead of silently skipping the line.
    let err = sm.append_message(user("u2")).unwrap_err();
    assert!(matches!(err, SessionError::Io { .. }), "{err}");
}

/// D-FLUSH: a failed first flush is retried by `sync()` so barriers fail on the same error.
#[cfg(unix)]
#[test]
fn sync_retries_failed_first_flush() {
    use std::os::unix::fs::PermissionsExt;
    let root = common::tempdir();
    let sessions = root.path().join("sessions");
    fs::create_dir(&sessions).unwrap();
    let mut sm =
        SessionManager::create(Path::new(CWD), &sessions, CreateOptions::default()).unwrap();
    // No conversation entry yet: sync is a no-op even on a read-only directory.
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o555)).unwrap();
    sm.sync().unwrap();
    // The custom entry is not conversation: still no file, still Ok.
    sm.append_session_info(Some("name")).unwrap();
    sm.sync().unwrap();
    assert!(!sm.session_file().unwrap().exists());
    // The first conversation entry cannot be flushed: sync returns the error (fail-closed).
    let err = sm.append_message(user("u1")).unwrap_err();
    assert!(matches!(err, SessionError::Io { .. }), "{err}");
    let err = sm.sync().unwrap_err();
    assert!(matches!(err, SessionError::Io { .. }), "{err}");
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o755)).unwrap();
    // With the directory writable again the same manager recovers.
    sm.sync().unwrap();
    assert!(sm.session_file().unwrap().exists());
}

/// D-FLUSH: in-memory sessions have nothing to sync.
#[test]
fn sync_is_noop_for_in_memory_sessions() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), vec![]);
    sm.append_message(user("u1")).unwrap();
    sm.sync().unwrap();
    assert!(!sm.is_persisted());
}

/// D-WRITER: the first flush (header + buffered entries) is itself durable — reopening a
/// freshly flushed session sees every entry.
#[test]
fn first_flush_is_durable_and_reopens() {
    let root = common::tempdir();
    let mut sm =
        SessionManager::create(Path::new(CWD), root.path(), CreateOptions::default()).unwrap();
    sm.append_message(user("u1")).unwrap();
    sm.append_message(assistant("a1")).unwrap();
    sm.sync().unwrap();
    let file = sm.session_file().unwrap().to_path_buf();
    let reopened = SessionManager::open(&file, root.path(), None).unwrap();
    assert_eq!(reopened.entry_count(), 2);
    assert!(matches!(
        reopened.entries()[1].kind,
        EntryKind::Message { .. }
    ));
}

#[test]
fn cwd_change_is_append_only_and_listed_as_moved_cwd() {
    let root = common::tempdir();
    let mut sm =
        SessionManager::create(Path::new(CWD), root.path(), CreateOptions::default()).unwrap();
    sm.append_message(user("hello")).unwrap();
    sm.append_message(assistant("hi")).unwrap();
    let file = sm.session_file().unwrap().to_path_buf();
    let before = fs::read_to_string(&file).unwrap();
    assert_eq!(sm.moved_cwd(), None);
    assert_eq!(
        SessionManager::list_all(root.path(), None, None).unwrap()[0].moved_cwd,
        None
    );

    sm.append_custom_message_entry(
        oino_session::CWD_CHANGE_CUSTOM_TYPE,
        UserContentField::Text("The workspace changed".into()),
        true,
        Some(serde_json::json!({"cwd": "/work/real", "previousCwd": CWD})),
    )
    .unwrap();
    let after = fs::read_to_string(&file).unwrap();
    // Append-only: every earlier byte (header + conversation) is untouched.
    assert!(after.starts_with(&before), "earlier lines were rewritten");
    assert_eq!(sm.cwd(), Path::new(CWD), "the header cwd stays");
    assert_eq!(sm.moved_cwd().as_deref(), Some(Path::new("/work/real")));
    let listed = SessionManager::list_all(root.path(), None, None).unwrap();
    assert_eq!(
        listed.len(),
        1,
        "the file stays in the old cwd's dir and is still listed"
    );
    assert_eq!(listed[0].cwd, CWD);
    assert_eq!(listed[0].moved_cwd.as_deref(), Some("/work/real"));
    // Reopened: the move survives.
    let reopened = SessionManager::open(&file, root.path(), None).unwrap();
    assert_eq!(
        reopened.moved_cwd().as_deref(),
        Some(Path::new("/work/real"))
    );
}
