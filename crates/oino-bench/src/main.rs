//! `oino-bench` — measurement harness for core WP-C11 ([core.md §4][spec], B-1).
//!
//! Driven by `cargo xtask bench <suite>` (or `mise run bench -- <suite> --n N`); writes
//! `subagents-<n>-<timestamp>.json` plus a markdown summary table into the results dir
//! (default `bench/results/`). Baseline and post-WP records live under
//! `bench/results/baseline-v2-<date>/` and friends, committed with their tables.
//!
//! `host` runs the v3-perf scenarios of docs/dev/v3/perf/00-spec.md §6 against a real
//! `unipi-host` (budgets in `bench/budgets.toml`, records in `bench/records/`); `fixture`
//! writes one generated session (100 / 5k / 40k entries, compactions and branches).
//!
//! [spec]: docs/dev/v2/specs/core.md

mod barrier;
mod faux;
mod fixture;
mod host;
mod host_bench;
mod load;
mod record;
mod sample;
mod scenario;
mod soak;
mod subagents;
mod wire;

use anyhow::{Result, bail};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("subagents") => subagents::run(&args[1..]).await,
        // The v3-perf host scenarios are blocking socket drivers: off the async runtime.
        Some("host") => {
            let rest = args[1..].to_vec();
            tokio::task::spawn_blocking(move || host_bench::run(&rest)).await?
        }
        Some("fixture") => fixture_cmd(&args[1..]),
        _ => bail!(
            "usage: oino-bench <subagents --n <N> [--out dir] | host <scenario…|all> [--gate] | \
             fixture <agent_dir> <cwd> <entries>> — drive via `cargo xtask bench`"
        ),
    }
}

/// `fixture <agent_dir> <cwd> <entries> [seed]`: writes one generated session, prints its path.
#[allow(clippy::print_stdout, reason = "the path is the product")]
fn fixture_cmd(args: &[String]) -> Result<()> {
    let [agent, cwd, entries, rest @ ..] = args else {
        bail!("usage: oino-bench fixture <agent_dir> <cwd> <entries> [seed]");
    };
    let seed = rest.first().map_or(Ok(1), |s| s.parse())?;
    let f = fixture::generate(agent.as_ref(), cwd.as_ref(), entries.parse()?, seed)?;
    println!(
        "{} ({} entries, session {})",
        f.path.display(),
        f.entries,
        f.session_id
    );
    Ok(())
}

/// Approximate metric arithmetic, separate from exact counter storage.
#[allow(
    clippy::cast_precision_loss,
    reason = "Floating-point measurement statistics intentionally round very large counters."
)]
fn count_float(value: usize) -> f64 {
    value as f64
}
#[allow(
    clippy::cast_precision_loss,
    reason = "Floating-point measurement statistics intentionally round very large counters."
)]
fn counter_float(value: u64) -> f64 {
    value as f64
}
#[allow(
    clippy::cast_precision_loss,
    reason = "Floating-point measurement statistics intentionally round very large counters."
)]
fn signed_float(value: i64) -> f64 {
    value as f64
}
