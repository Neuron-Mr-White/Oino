//! `oino-acp`: Oino as an [Agent Client Protocol](https://agentclientprotocol.com) v1 agent
//! (`oino --mode acp`, modes.md §6).
//!
//! The adapter sits on the in-process [`oino_client::InProcessBackend`] of one
//! `AgentSessionRuntime` per ACP session — never on RPC JSON (research-acp §9.1) — and speaks
//! ACP through the official `agent-client-protocol` SDK. Nothing else in the workspace sees ACP
//! types.
//!
//! # Method coverage
//!
//! | ACP | Oino |
//! |---|---|
//! | `initialize` | capabilities of research-acp §5 (+ `sessionCapabilities.fork`), client caps recorded |
//! | `authenticate`, `logout` | accepted (no auth methods: provider auth is extension-owned) |
//! | `session/new`, `session/load` (replay), `session/resume`, `session/fork` | one runtime per session ([`SessionFactory`]) |
//! | `session/list`, `session/close`, `session/delete` | `SessionManager` listing, dispose, move to `<agent>/trash/` |
//! | `session/prompt`, `session/cancel` | `prompt` (follow-up while streaming) resolved on `agent_settled`; `abort` |
//! | `session/set_config_option`, `session/set_mode` | model, thinking, queue modes, auto-compaction/retry; mode = thinking level |
//! | `_oino/rpc`, `_oino/<command>` | any pi / `_oino/*` RPC command on the session's backend |
//!
//! Agent → client: `session/update` (message/thought chunks, tool calls with kinds, locations,
//! diffs and terminals, plans, usage, compaction, notices, commands, config, session info),
//! `session/request_permission` (approval dialogs, `confirm` during a tool call),
//! `elicitation/create` (select/input/editor/confirm/multiSelect), `fs/*` and `terminal/*`
//! through [`AcpEnv`], `_oino/queue_update`, `_oino/run_state`, `_oino/extension_ui`.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`agent`] | [`AcpAgent`]: request dispatch, [`AgentInfo`] |
//! | [`factory`] | [`SessionFactory`], [`CoreSessionFactory`], [`SessionDefaults`] |
//! | [`mod@env`] | [`AcpEnv`]: client-hosted files and terminals |
//! | [`client`] | [`ClientLink`], [`ClientCaps`]: typed agent → client calls |
//! | [`tools`], [`convert`], [`config`], [`commands`], [`ui`] | event, prompt, config, slash-command and dialog mappings |
//! | [`error`] | [`AcpError`] |

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

pub mod agent;
pub mod client;
pub mod commands;
pub mod config;
pub mod convert;
pub mod env;
pub mod error;
pub mod factory;
mod session;
pub mod tools;
pub mod ui;

use std::path::PathBuf;
use std::sync::Arc;

pub use agent::{AcpAgent, AgentInfo};
pub use client::{ClientCaps, ClientLink};
pub use env::AcpEnv;
pub use error::AcpError;
pub use factory::{
    CoreSessionFactory, ModelChoice, ModelResolver, SessionDefaults, SessionFactory, SessionRequest,
};

/// Serves ACP on stdin/stdout until the client closes stdin (`oino --mode acp`).
///
/// # Errors
///
/// Transport and protocol failures reported by the SDK.
pub async fn serve_stdio(
    factory: Arc<dyn SessionFactory>,
    info: AgentInfo,
    default_cwd: PathBuf,
) -> Result<(), agent_client_protocol::Error> {
    AcpAgent::new(factory, info, default_cwd)
        .serve(agent_client_protocol::Stdio::new())
        .await
}
