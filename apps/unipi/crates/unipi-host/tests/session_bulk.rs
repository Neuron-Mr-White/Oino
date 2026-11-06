//! [ux-sessions] (owner fix 2026-11-03): the batch ops and the sidebar query on a store of
//! session files written straight to disk (no runtimes): `archive_many` moves N files in one
//! op and announces them in `SESSIONS_EVENT_CHUNK` chunks, `restore_many` puts them back,
//! `query` matches title + first user message over live and archived chats with filters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::sync::Arc;

use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use unipi_host::sessions::hub::{HostHubOptions, HubEvent, SessionHub, SessionsQuery};
use unipi_proto::session_bulk::{SessionsQueryFilters, SessionsQueryScope, SESSIONS_EVENT_CHUNK};

struct Fixture {
    _dir: tempfile::TempDir,
    cwd: PathBuf,
    agent_dir: PathBuf,
    sessions: PathBuf,
    host_data: PathBuf,
}

impl Fixture {
    fn hub(&self) -> Arc<SessionHub> {
        let provider =
            FauxProvider::new((0..8).map(|_| FauxResponse::Text("done".into())).collect());
        SessionHub::create(HostHubOptions {
            cwd: self.cwd.clone(),
            agent_dir: self.agent_dir.clone(),
            docs_root: None,
            session_dir: Some(self.sessions.clone()),
            mode: ExtensionMode::Desktop,
            has_ui: false,
            approve: Some(false),
            flags: ResourceFlags {
                no_context_files: true,
                ..ResourceFlags::default()
            },
            loader: Arc::new(StaticLoader(vec![
                Arc::new(FauxComponent(provider)) as Arc<dyn ExtensionInstance>
            ])),
            models: {
                let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
                models.set_runtime_api_key("faux", "test-key");
                Some(models)
            },
            tools: ToolSelection::Default,
            exclude_tools: Vec::new(),
            services_cap: None,
            host_data_dir: Some(self.host_data.clone()),
        })
        .unwrap()
    }

    fn trash(&self) -> PathBuf {
        self.agent_dir.join("trash")
    }
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    let host_data = dir.path().join("host");
    for path in [&agent_dir, &sessions, &host_data] {
        std::fs::create_dir_all(path).unwrap();
    }
    Fixture {
        _dir: dir,
        cwd,
        agent_dir,
        sessions,
        host_data,
    }
}

/// Writes a session file (header + one user message) for `cwd`; returns its id.
fn write_session(f: &Fixture, n: usize, cwd: &str, name: Option<&str>, first: &str) -> String {
    let id = format!("0190{n:04}-0000-7000-8000-{n:012}");
    let dir = oino_session::manager::cwd_session_dir(&f.sessions, std::path::Path::new(cwd));
    std::fs::create_dir_all(&dir).unwrap();
    let ts = format!(
        "2026-10-{:02}T10:{:02}:{:02}.000Z",
        1 + n % 28,
        n / 60 % 60,
        n % 60
    );
    let stem = ts.replace([':', '.'], "-");
    let mut lines = vec![
        serde_json::json!({"type":"session","version":3,"id":id,"timestamp":ts,"cwd":cwd})
            .to_string(),
    ];
    if let Some(name) = name {
        lines.push(serde_json::json!({"type":"session_info","id":format!("i{n}"),"parentId":null,"timestamp":ts,"name":name}).to_string());
    }
    lines.push(serde_json::json!({"type":"message","id":format!("m{n}"),"parentId":null,"timestamp":ts,
        "message":{"role":"user","content":[{"type":"text","text":first}],"timestamp":1_790_000_000_000_i64 + n as i64}}).to_string());
    std::fs::write(
        dir.join(format!("{stem}_{id}.jsonl")),
        lines.join("\n") + "\n",
    )
    .unwrap();
    id
}

fn files_in(dir: &std::path::Path) -> usize {
    walk(dir).len()
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else if p.extension().is_some_and(|x| x == "jsonl") {
                out.push(p);
            }
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_many_moves_every_file_in_one_op_with_chunked_events() {
    let f = fixture();
    let ids: Vec<String> = (0..450)
        .map(|n| {
            write_session(
                &f,
                n,
                &format!("/work/p{}", n % 7),
                Some(&format!("chat {n}")),
                "hello",
            )
        })
        .collect();
    let hub = f.hub();
    assert_eq!(hub.list().await.unwrap().len(), 450);
    let mut events = hub.subscribe();
    let mut targets = ids.clone();
    targets.push("no-such-session".into());
    let started = std::time::Instant::now();
    let data = hub.archive_many(&targets).await.unwrap();
    assert_eq!(data.done.len(), 450);
    assert_eq!(data.failed.len(), 1);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "batch took {:?}",
        started.elapsed()
    );
    assert_eq!(files_in(&f.sessions), 0);
    assert_eq!(files_in(&f.trash()), 450);
    assert!(hub.list().await.unwrap().is_empty());
    assert_eq!(hub.archived().await.unwrap().len(), 450);
    let mut chunks = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let HubEvent::SessionsRemoved {
            session_ids,
            archived,
        } = event
        {
            assert!(archived);
            chunks.push(session_ids.len());
        }
    }
    assert_eq!(chunks, vec![SESSIONS_EVENT_CHUNK, SESSIONS_EVENT_CHUNK, 50]);

    let back = hub.restore_many(&ids[..300]).await.unwrap();
    assert_eq!(back.done.len(), 300);
    assert_eq!(hub.list().await.unwrap().len(), 300);
    assert_eq!(hub.archived().await.unwrap().len(), 150);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_matches_title_and_first_message_across_live_and_archived() {
    let f = fixture();
    let a = write_session(
        &f,
        1,
        "/work/parser",
        None,
        "please fix the flaky tokenizer test",
    );
    let b = write_session(
        &f,
        2,
        "/work/web",
        Some("Deploy the landing page"),
        "ship it",
    );
    let c = write_session(&f, 3, "/work/web", Some("Old tokenizer notes"), "notes");
    for n in 10..130 {
        write_session(&f, n, "/work/bulk", Some(&format!("bulk {n}")), "filler");
    }
    let hub = f.hub();
    hub.archive_many(std::slice::from_ref(&c)).await.unwrap();
    let q = |text: &str, scope: SessionsQueryScope| SessionsQuery {
        text: text.into(),
        scope,
        ..SessionsQuery::default()
    };

    // First user message + title, archived included under `all`, marked archived.
    let page = hub
        .query(&q("TOKENIZER", SessionsQueryScope::All))
        .await
        .unwrap();
    let mut got: Vec<(String, bool)> = page
        .hits
        .iter()
        .map(|h| (h.card.session_id.clone(), h.archived))
        .collect();
    got.sort();
    assert_eq!(got, vec![(a.clone(), false), (c.clone(), true)]);
    let hit_a = page.hits.iter().find(|h| h.card.session_id == a).unwrap();
    assert!(hit_a.snippet.as_deref().unwrap().contains("tokenizer"));
    // Live only / archived only.
    assert_eq!(
        hub.query(&q("tokenizer", SessionsQueryScope::Live))
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        hub.query(&q("tokenizer", SessionsQueryScope::Archived))
            .await
            .unwrap()
            .total,
        1
    );
    // Every word must match (title OR message); a folder-only title counts ("parser").
    assert_eq!(
        hub.query(&q("parser flaky", SessionsQueryScope::All))
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        hub.query(&q("landing nope", SessionsQueryScope::All))
            .await
            .unwrap()
            .total,
        0
    );
    // Workspace filter + paging.
    let mut req = q("bulk", SessionsQueryScope::Live);
    req.filters = SessionsQueryFilters {
        workspaces: vec!["/work/bulk".into()],
        ..SessionsQueryFilters::default()
    };
    req.limit = Some(50);
    let first = hub.query(&req).await.unwrap();
    assert_eq!((first.hits.len(), first.total), (50, 120));
    assert_eq!(first.next_cursor.as_deref(), Some("50"));
    req.cursor = Some("100".into());
    let last = hub.query(&req).await.unwrap();
    assert_eq!(last.hits.len(), 20);
    assert!(last.next_cursor.is_none());
    req.filters.workspaces = vec!["/work/web".into()];
    req.cursor = None;
    req.text = String::new();
    let web = hub.query(&req).await.unwrap();
    assert_eq!(
        web.hits
            .iter()
            .map(|h| h.card.session_id.clone())
            .collect::<Vec<_>>(),
        vec![b]
    );
}
