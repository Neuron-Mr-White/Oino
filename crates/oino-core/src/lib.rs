//! `oino-core`: the pi 1.4.0 coding-agent runtime for Oino.
//!
//! The composition root of the runtime crates: `oino-agent` (loop), `oino-tools` (built-ins),
//! `oino-settings`, `oino-resources`, `oino-models` and `oino-session`. Providers and
//! extensions are reached only through the [`extensions::api`] traits (implemented by the
//! WASM host); core contains no provider code and never depends on wasmtime.
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`services`] | `agent-session-services.ts` | [`services::Services`] per cwd, [`services::ExtensionLoader`], [`services::HostRouter`] |
//! | [`runtime`] | `agent-session-runtime.ts`, `sdk.ts` | [`runtime::AgentSessionRuntime`]: create, new/switch/fork/clone/import |
//! | [`session`] | `agent-session.ts` | [`session::AgentSession`]: prompts, queues (+ single-item edits), models, tools, compaction, retry, bash (+ detach), tree, stats, export, file checkpoints |
//! | [`extensions`] | `extensions/runner.ts` | instance contract, host operations, registries, pi-composition dispatch, turn arbiter |
//! | [`agents`] | — (D8) | nested `AgentSession`s behind `agents.spawn`: [`agents::ChildAgent`], [`agents::AgentSpec`] |
//! | [`provider`] | `model-runtime.streamSimple` | [`provider::ProviderStream`]: requests to provider components |
//! | [`compaction`] | `compaction/*` | cut points, summaries, branch summaries, overflow/retry classification |
//! | [`events`] | `AgentSessionEvent` + RPC events | [`events::ClientEvent`] and friends (move to `oino-protocol`) |
//! | [`ui`] | `rpc-extension-ui` | [`ui::UiBridge`]: dialogs, fire-and-forget requests, dismissals |
//! | [`hub`] | — | [`hub::EventHub`]: ordered, bounded, coalescing fan-out |
//! | [`topics`] | — | [`topics::TopicStore`]: sticky bus topics mirrored to frontends (`_oino/topics`, `oino:git`) |
//! | [`builtin`] | `createAllToolDefinitions` | built-in tools bound to session context |
//! | [`export`] | `export-html` | self-contained HTML export |
//! | `testing` (feature) | — | faux provider component, fake extensions, in-memory sessions |

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
pub mod services;
pub mod session;
#[cfg(feature = "testing")]
pub mod testing;
pub mod topics;
pub mod ui;
