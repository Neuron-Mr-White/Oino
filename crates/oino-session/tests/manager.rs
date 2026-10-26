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

#[test]
fn compaction_stores_system_checkpoint() {
    let mut sm = SessionManager::in_memory(Path::new(CWD), CreateOptions::default(), Vec::new());
    sm.append_message(system("base")).unwrap();
    sm.append_message(user("a")).unwrap();
    let keep = sm.append_message(assistant("A")).unwrap();
    sm.append_message(system("patch")).unwrap();
    let id = sm
        .append_compaction(CompactionDraft {
            summary: "sum".into(),
            first_kept_entry_id: Some(keep),
            tokens_before: 9,
            ..CompactionDraft::default()
        })
        .unwrap();
    let EntryKind::Compaction(compaction) = &sm.entry(&id).unwrap().kind else {
        panic!("not a compaction")
    };
    let checkpoint = compaction.system_message.as_ref().unwrap();
    assert_eq!(
        checkpoint.content,
        UserTextField::Text("base\n\npatch".into())
    );
    assert_eq!(
        Some(checkpoint.timestamp),
        sm.entry(&id).unwrap().timestamp.to_ms()
    );
    let wire = serde_json::to_value(sm.entry(&id).unwrap()).unwrap();
    assert_eq!(wire["systemMessage"]["role"], "system");
    let roles: Vec<&str> = sm
        .build_session_context()
        .messages
        .iter()
        .map(AgentMessage::role)
        .collect();
    assert_eq!(roles, ["system", "compactionSummary", "assistant"]);

    let none = sm.append_compaction(CompactionDraft::default()).unwrap();
    let EntryKind::Compaction(retain_none) = &sm.entry(&none).unwrap().kind else {
        panic!("not a compaction")
    };
    assert_eq!(retain_none.first_kept_entry_id, none);
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
    assert!(
        fs::read_to_string(&v1).unwrap().contains("\"version\":3"),
        "rewritten in place"
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
