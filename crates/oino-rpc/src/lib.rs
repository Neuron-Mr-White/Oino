//! `oino-rpc`: pi's RPC protocol (`--mode rpc`) for Oino.
//!
//! Strict JSONL over any byte stream: commands in, replies and client events out, with the
//! extension UI sub-protocol (`extension_ui_request` / `extension_ui_response`) and the Oino
//! extensions behind `_oino/hello` (modes.md §3–§4).
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`jsonl`] | `modes/rpc/jsonl.ts` | [`jsonl::LineReader`], [`jsonl::serialize_line`] |
//! | [`wire`] | `modes/json-event.ts` | [`wire::session_event_json`] (also JSON mode), [`wire::rpc_event`] filtering |
//! | [`server`] | `modes/rpc/rpc-mode.ts` | [`RpcServer`] over any `SessionBackend` |
//! | [`client`] | `modes/rpc/rpc-client.ts` | [`RpcClient`]: typed methods, [`Transport::StdioChild`] |
//! | [`backend`] | — | [`RpcBackend`]: `SessionBackend` over an [`RpcClient`]; [`backend::EventDecoder`] |
//!
//! `RpcBackend` lives here rather than in `oino-client` because `oino-rpc` already depends
//! on `oino-client` (deviations/modes.md).

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
pub mod client;
pub mod jsonl;
pub mod server;
pub mod wire;

pub use backend::RpcBackend;
pub use client::{ChildCommand, RpcClient, Transport};
pub use server::{RpcServer, RpcServerError};
