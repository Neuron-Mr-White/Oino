//! `oino-bench` — measurement harness for core WP-C11 ([core.md §4][spec], B-1).
//!
//! Driven by `cargo xtask bench <suite>` (or `mise run bench -- <suite> --n N`); writes
//! `subagents-<n>-<timestamp>.json` plus a markdown summary table into the results dir
//! (default `bench/results/`). Baseline and post-WP records live under
//! `bench/results/baseline-v2-<date>/` and friends, committed with their tables.
//!
//! [spec]: docs/dev/v2/specs/core.md

mod barrier;
mod sample;
mod subagents;

use anyhow::{Result, bail};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("subagents") => subagents::run(&args[1..]).await,
        _ => bail!(
            "usage: oino-bench <subagents --n <N> [--out dir]> — drive via `cargo xtask bench`"
        ),
    }
}
