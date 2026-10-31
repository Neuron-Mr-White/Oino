//! `oino-extension-api`: the contract between `oino-core` and any extension runtime
//! (extension-api.md §6). It knows nothing about wasmtime.
//!
//! # Boundary
//!
//! Core implements [`HostServices`] (what extensions may do to sessions, UI, models, agents);
//! a runtime (the WASM host) implements [`ExtensionRuntime`] / [`ExtensionInstance`] (what core
//! may ask an extension). Data-model values cross as [`RawJson`] with exact shapes (D5);
//! capability calls use the typed records of the WIT package `oino:extension@2.0.0`.
//!
//! # API map
//!
//! | Module | WIT | Contents |
//! |---|---|---|
//! | [`json`] | `types.json` | [`RawJson`] |
//! | [`types`] | `types` | [`ExtensionId`], [`CallContext`], [`Mode`], [`CallKind`], [`HostError`], [`ExtensionError`] |
//! | [`registry`] | `registry` | [`EventKind`] (wire names), tool/command/shortcut/flag/renderer specs, [`Registration`] |
//! | [`guest`] | `guest` | [`InitInput`], [`ToolInvocation`], [`CompletionItem`] |
//! | [`ops`] | `session`, `control`, `models`, `ui`, `tasks`, `secrets`, `provider-host` | one enum variant per WIT function |
//! | [`runtime`] | — | [`ExtensionRuntime`], [`ExtensionInstance`], [`ProviderInstance`], [`HostServices`], [`AgentHandle`] |
//! | [`manifest`] | — | `extension.json`, [`manifest::Permissions`], package validation |
//! | [`host_service`] | `host-service` | [`host_service::HostService`] (capabilities an embedding host registers) |
//! | `testing` (feature) | — | `RecordingHostServices`, `FakeExtensionRuntime` |

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

pub mod guest;
pub mod host_service;
pub mod json;
pub mod manifest;
pub mod ops;
pub mod registry;
pub mod runtime;
#[cfg(feature = "testing")]
pub mod testing;
pub mod types;

pub use guest::{CompletionItem, InitInput, ToolInvocation};
pub use host_service::HostService;
pub use json::RawJson;
pub use manifest::{ExtensionKind, Manifest};
pub use registry::{EventKind, Registration};
pub use runtime::{
    AgentHandle, BoxFut, ComponentSource, DetachedBash, ExtensionInstance, ExtensionRuntime,
    HostServices, LoadedPackage, ProviderEventStream, ProviderInstance,
};
pub use tokio_util::sync::CancellationToken;
pub use types::{CallContext, CallKind, ExtensionError, ExtensionId, HostError, LogLevel, Mode};
