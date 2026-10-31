//! `oino-rpc`: the JSONL RPC protocol (`--mode rpc`) for Oino.
//!
//! Strict JSONL over any byte stream: commands in, replies and client events out, with the
//! extension UI sub-protocol (`extension_ui_request` / `extension_ui_response`) and the Oino
//! extensions behind `_oino/hello` (modes.md §3–§4).
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`jsonl`] | [`jsonl::LineReader`], [`jsonl::serialize_line`] |
//! | [`wire`] | [`wire::session_event_json`] (also JSON mode), [`wire::rpc_event`] filtering |
//! | [`server`] | [`RpcServer`] over any `SessionBackend` |
//! | [`client`] | [`RpcClient`]: typed methods, [`Transport::StdioChild`] |
//! | [`backend`] | [`RpcBackend`]: `SessionBackend` over an [`RpcClient`]; [`backend::EventDecoder`] |
//!
//! `RpcBackend` lives here rather than in `oino-client` because `oino-rpc` already depends
//! on `oino-client` (deviations/modes.md).

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-256.png",
    html_favicon_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/oino.ico"
)]
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
