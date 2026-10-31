//! `oino-core`: the coding-agent runtime for Oino.
//!
//! The composition root of the runtime crates: `oino-agent` (loop), `oino-tools` (built-ins),
//! `oino-settings`, `oino-resources`, `oino-models` and `oino-session`. Providers and
//! extensions are reached only through the [`extensions::api`] traits (implemented by the
//! WASM host); core contains no provider code and never depends on wasmtime.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`services`] | [`services::Services`] per cwd, [`services::ExtensionLoader`], [`services::HostRouter`] |
//! | [`runtime`] | [`runtime::AgentSessionRuntime`]: create, new/switch/fork/clone/import |
//! | [`safe_mode`] | [`safe_mode::SafeMode`]: failed-start counter, suspects, `-ne` plan |
//! | [`session`] | [`session::AgentSession`]: prompts, queues (+ single-item edits), models, tools, compaction, retry, bash (+ detach), tree, stats, export, file checkpoints |
//! | [`extensions`] | instance contract, host operations, registries, composition dispatch, turn arbiter |
//! | [`agents`] | nested `AgentSession`s behind `agents.spawn`: [`agents::ChildAgent`], [`agents::AgentSpec`] |
//! | [`provider`] | [`provider::ProviderStream`]: requests to provider components |
//! | [`compaction`] | cut points, summaries, branch summaries, overflow/retry classification |
//! | [`events`] | [`events::ClientEvent`] and friends (move to `oino-protocol`) |
//! | [`ui`] | [`ui::UiBridge`]: dialogs, fire-and-forget requests, dismissals |
//! | [`hub`] | [`hub::EventHub`]: ordered, bounded, coalescing fan-out |
//! | [`topics`] | [`topics::TopicStore`]: sticky bus topics mirrored to frontends (`_oino/topics`, `oino:git`) |
//! | [`builtin`] | built-in tools bound to session context |
//! | [`export`] | self-contained HTML export |
//! | `testing` (feature) | — | faux provider component, fake extensions, in-memory sessions |

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

pub mod agents;
pub mod builtin;
pub mod compaction;
pub mod events;
pub mod export;
pub mod extensions;
pub mod hub;
pub mod provider;
pub mod runtime;
pub mod safe_mode;
pub mod services;
pub mod session;
#[cfg(feature = "testing")]
pub mod testing;
pub mod topics;
pub mod ui;
