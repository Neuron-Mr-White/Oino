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
