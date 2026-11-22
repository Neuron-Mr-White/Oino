//! UNI-303 FIX 4: the browser CDP endpoint a chat's scripts get (`AGENT_BROWSER_CDP`).
//!
//! The host registers one resolver (`set_browser_cdp_resolver`) at serve
//! time; every session's bash env reads it at call time: the chat's own
//! profile (`…?profile=<name>`), and no entry / no running browser keeps
//! the stable global address. (Subagents inherit their parent chat's
//! profile — that mapping is the host's `root_session`, asserted in the
//! `unipi-host` browser tests.)

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use oino_core::session::{AgentSession, set_browser_cdp_resolver};
use oino_core::testing::{FauxProvider, TestSession};

/// The one resolver this test binary registers: session id → address from a
/// shared map (first registration wins; every test installs the same closure,
/// then routes its own ids).
fn routing() -> &'static Mutex<HashMap<String, String>> {
    static MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| {
        let _ = set_browser_cdp_resolver(Arc::new(|session: &str| {
            MAP.get()
                .and_then(|m| m.lock().ok())
                .and_then(|m| m.get(session).cloned())
                .unwrap_or_default()
        }));
        Mutex::new(HashMap::new())
    })
}

fn route(session: &str, address: &str) {
    routing()
        .lock()
        .unwrap()
        .insert(session.to_owned(), address.to_owned());
}

async fn session() -> AgentSession {
    TestSession::default()
        .build(FauxProvider::new(vec![]))
        .await
}

fn session_env(session: &AgentSession) -> Vec<(String, String)> {
    oino_tools::SessionEnv {
        session_id: session.session_id(),
        session_file: session.session_file().map(|p| p.display().to_string()),
        provider: None,
        model: None,
        reasoning_level: None,
        browser_cdp: session.browser_cdp(),
        browser_session: Some(format!("oino-{}", short(session.session_id().as_str()))),
    }
    .vars()
}

fn short(id: &str) -> &str {
    id.rsplit('-').next().unwrap_or(id).get(..8).unwrap_or(id)
}

fn env_value(env: &[(String, String)], name: &str) -> Option<String> {
    env.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
}

#[tokio::test]
async fn no_browser_means_no_cdp_export_but_a_private_daemon_name() {
    let _ = routing();
    let session = session().await;
    // No entry for this chat = its browser isn't running.
    assert_eq!(session.browser_cdp(), None);
    let env = session_env(&session);
    assert_eq!(env_value(&env, "AGENT_BROWSER_CDP"), None);
    assert_eq!(env_value(&env, "OINO_BROWSER_CDP"), None);
    // The per-chat agent-browser daemon name is always set: two chats never
    // share an agent-browser state dir.
    let daemon = env_value(&env, "AGENT_BROWSER_SESSION").expect("daemon name");
    assert_eq!(
        daemon,
        format!("oino-{}", short(session.session_id().as_str()))
    );
    assert_eq!(
        env_value(&env, "OINO_BROWSER_SESSION").as_deref(),
        Some(daemon.as_str())
    );
}

#[tokio::test]
async fn the_chat_gets_its_own_profile_address() {
    let _ = routing();
    let session = session().await;
    let id = session.session_id();
    route(id.as_str(), "ws://127.0.0.1:41001/?profile=shop");
    assert_eq!(
        session.browser_cdp().as_deref(),
        Some("ws://127.0.0.1:41001/?profile=shop")
    );
    let env = session_env(&session);
    let cdp = env_value(&env, "AGENT_BROWSER_CDP");
    assert_eq!(cdp.as_deref(), Some("ws://127.0.0.1:41001/?profile=shop"));
    assert_eq!(
        env_value(&env, "OINO_BROWSER_CDP").as_deref(),
        cdp.as_deref()
    );
    assert!(env_value(&env, "AGENT_BROWSER_SESSION").is_some());
}

#[tokio::test]
async fn two_chats_resolve_to_their_own_addresses() {
    let _ = routing();
    let a = session().await;
    let b = session().await;
    route(a.session_id().as_str(), "ws://127.0.0.1:41003/?profile=x");
    route(b.session_id().as_str(), "ws://127.0.0.1:41004/?profile=y");
    assert_eq!(
        a.browser_cdp().as_deref(),
        Some("ws://127.0.0.1:41003/?profile=x")
    );
    assert_eq!(
        b.browser_cdp().as_deref(),
        Some("ws://127.0.0.1:41004/?profile=y")
    );
    let (ea, eb) = (session_env(&a), session_env(&b));
    assert_eq!(
        env_value(&ea, "AGENT_BROWSER_CDP").as_deref(),
        Some("ws://127.0.0.1:41003/?profile=x")
    );
    assert_eq!(
        env_value(&eb, "AGENT_BROWSER_CDP").as_deref(),
        Some("ws://127.0.0.1:41004/?profile=y")
    );
    assert_ne!(
        env_value(&ea, "AGENT_BROWSER_SESSION"),
        env_value(&eb, "AGENT_BROWSER_SESSION")
    );
}
