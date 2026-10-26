//! `oino-core`: the pi coding-agent runtime for Oino.
//!
//! Work in progress (core-runtime).

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

pub mod compaction;
pub mod events;
pub mod export;
pub mod extensions;
pub mod hub;
pub mod provider;
pub mod session;
#[cfg(feature = "testing")]
pub mod testing;
pub mod ui;
