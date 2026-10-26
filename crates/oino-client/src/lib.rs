//! `oino-client`: how frontends talk to a session.
//!
//! Every frontend (RPC server, TUI, GPUI, ACP adapter) drives a session through
//! [`SessionBackend`]: pi/Oino [`RpcCommand`]s in, one ordered stream of
//! [`oino_core::events::ClientEvent`]s out, dialog answers via
//! [`SessionBackend::ui_respond`]. The in-process backend passes the enums without
//! serialisation; `oino-rpc`'s `RpcBackend` serialises them over JSONL, so a frontend cannot
//! tell the two apart.
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`protocol`] | `rpc-types.ts` | [`RpcCommand`] ([`PiCommand`], [`OinoCommand`]), [`RpcRequest`], [`RpcResponse`], [`RpcSessionState`] |
//! | [`backend`] | — | [`SessionBackend`], [`ClientError`] |
//! | [`frontend`] | — | [`FrontendApi`]: typed `_oino/*` frontend commands (completions, panel actions, shortcuts, editor intents, settings, services, sign-in) for every backend |
//! | [`in_process`] (feature `in-process`) | `rpc-mode.ts` `handleCommand` | [`InProcessBackend`] over `AgentSessionRuntime` |
//! | [`fake`] | — | [`fake::FakeBackend`]: scripted events and replies |
//!
//! The protocol records live here until `oino-protocol` exists (deviations/modes.md).

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

pub mod backend;
pub mod fake;
#[cfg(feature = "in-process")]
mod follow;
pub mod frontend;
#[cfg(feature = "in-process")]
pub mod in_process;
pub mod protocol;

pub use backend::{ClientError, SessionBackend};
#[cfg(feature = "in-process")]
pub use follow::FollowStream;
pub use frontend::{FrontendApi, ShortcutInfo};
#[cfg(feature = "in-process")]
pub use in_process::{InProcessBackend, ServerInfo};
pub use protocol::{
    OinoCommand, PiCommand, RejectedCommand, RpcCommand, RpcRequest, RpcResponse, RpcResponseData,
    RpcSessionState,
};
