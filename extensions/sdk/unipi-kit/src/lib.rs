//! Shared guest-side kit for the unipi 3.0.0 ports (unipi `packages/core` + the umbrella).
//!
//! unipi's core is a library of process-wide registries on `globalThis`; WASM components share
//! nothing, so each registry becomes either a host feature, a service convention or a
//! sticky bus topic (unipi-port §2). This crate is the guest half of that mapping: names,
//! payload shapes and pure logic every unipi port reuses. It depends only on the
//! `oino-extension` SDK; pure functions are unit-tested natively, host-calling helpers are thin.
//!
//! | Module | unipi source | Provides |
//! |---|---|---|
//! | [`suite`] | `packages/unipi` (umbrella) | extension ids, load order, namespace owners, `unipi:` prefix |
//! | [`topics`] | `events.ts`, `bus.ts` types | bus topic names, sticky set, payload structs |
//! | [`bus`] | `bus.ts` | typed publish/get/subscribe, `oino_bus` decoding, event routing |
//! | [`services`] | `Symbol.for` holders | service names (fan-out + point) and payloads |
//! | [`fanout`] | evidence/compaction/move registries | time-boxed call-every-provider |
//! | [`settings`] | `src/settings/*` | schema builder (ui-protocol §6.6), layered values, store |
//! | [`jev`] | `src/jev/*` | decision-model settings, transports, fail-open `ask` |
//! | [`provenance`] | `harness-messages.ts` | provenance records + delivery mapping |
//! | [`bounded`] | `bounded-output.ts` | head/tail bounded output with private artifact |
//! | [`attach`] | `src/attach/detect.ts` | path detection, `[Image #N]`/`[File #N]` tokens |
//! | [`savings`] | `src/compaction-savings.ts` | compaction savings math |
//! | [`tray`] | `src/work/*` | work-tray tab contract, hint line, work items |
//! | [`paths`] | `src/workspace/*`, `utils.ts` | preopen state paths, host-path mapping, JSON files |
//! | [`fmt`] | `utils.ts` | tokens/bytes/version/args formatting, ISO time, widths |
//! | [`ous`] | — | OUS node builders |
//!
//! # Errors
//!
//! Fallible helpers fail only with the SDK [`oino_extension::Error`] of the host import they
//! wrap (or an I/O message naming the path); fail-open helpers (`jev::ask`, `fanout`) return
//! `None`/empty instead.
//!
//! Not here (host features in Oino): turn arbiter, pending-work monitor, command echo, command
//! runners (`control.command-invoke`), child-agent runtime, remote dialogs, child usage,
//! detachable bash, model cache, TUI kits (frontends).

#![allow(
    clippy::missing_errors_doc,
    reason = "fallible helpers only forward SDK errors (host-error text or JSON), documented here once"
)]

pub mod attach;
pub mod bounded;
pub mod bus;
pub mod fanout;
pub mod fmt;
pub mod jev;
pub mod ous;
pub mod paths;
pub mod provenance;
pub mod savings;
pub mod services;
pub mod settings;
pub mod suite;
pub mod topics;
pub mod tray;
