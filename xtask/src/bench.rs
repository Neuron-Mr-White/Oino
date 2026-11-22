//! `cargo xtask bench <suite> [--n N] [--out dir]` (WP-C11): drives `crates/oino-bench`
//! and lands its results under `bench/results/`.
//!
//! `cargo xtask bench <scenario…|all> [--gate] [--compare REC.json | --pair BASELINE_HOST] [--host-bin P]
//! [--label L] [--quick] [--load-secs N]` (v3-perf, docs/dev/v3/perf/00-spec.md §6): the
//! host scenarios (open, send, model, list, ping, l1, idle, crash) against a real
//! `unipi-host`, records in `bench/records/`. `--gate` fails on a budget
//! (`bench/budgets.toml`) or a > 10 % regression against `--compare` (default: the newest
//! `bench/records/baseline-*.json`).

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::root;

pub(crate) fn bench(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("subagents") => subagents(&args[1..]),
        Some(first) if first == "--gate" || is_host_scenario(first) => host(args),
        None if args.iter().any(|arg| arg == "--gate") => host(args),
        Some(other) => bail!(
            "unknown bench suite '{other}' (known: subagents, {})",
            HOST_SCENARIOS.join(", ")
        ),
        None => bail!(
            "usage: cargo xtask bench <subagents --n <1|10|50> [--out dir]> | \
             <{}…> [--gate] [--compare rec.json] [--host-bin path]",
            HOST_SCENARIOS.join("|")
        ),
    }
}

fn subagents(args: &[String]) -> Result<()> {
    let mut n: Option<u64> = None;
    let mut out: Option<PathBuf> = None;
    let mut flags = args.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--n" => {
                n = Some(
                    flags
                        .next()
                        .context("--n needs a value")?
                        .parse()
                        .context("--n must be an integer")?,
                );
            }
            "--out" => out = Some(PathBuf::from(flags.next().context("--out needs a value")?)),
            other => bail!(
                "unknown flag '{other}' (usage: cargo xtask bench subagents --n <1|10|50> \
                 [--out dir])"
            ),
        }
    }
    let n = n.ok_or_else(|| anyhow::anyhow!("--n is required (1, 10, 50 for the record)"))?;
    if n == 0 {
        bail!("--n must be at least 1");
    }
    let out = out.unwrap_or_else(|| root().join("bench/results"));
    let head = git_head();
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.args(["run", "--release", "-p", "oino-bench", "--"])
        .arg("subagents")
        .arg("--n")
        .arg(n.to_string())
        .arg("--out")
        .arg(&out);
    // The bench's temp agent dir must live on real disk (a tmpfs system temp dir would
    // zero the fsync barrier measurement); the repo target dir is the sane default.
    cmd.env("OINO_BENCH_WORKDIR", root().join("target/bench"));
    if let Some(head) = &head {
        cmd.env("OINO_BENCH_GIT_HEAD", head);
    }
    let status = cmd.status().context("running oino-bench")?;
    if !status.success() {
        bail!("oino-bench failed");
    }
    Ok(())
}

/// v3-perf host scenarios (`all` = every one but a soak).
const HOST_SCENARIOS: &[&str] = &[
    "all", "open", "send", "model", "list", "ping", "l1", "idle", "crash", "disk",
];

fn is_host_scenario(arg: &str) -> bool {
    let name = arg.strip_prefix("s-").unwrap_or(arg).to_ascii_lowercase();
    HOST_SCENARIOS.contains(&name.as_str())
}

fn host(args: &[String]) -> Result<()> {
    let mut forwarded: Vec<String> = Vec::new();
    let mut gate = false;
    let paired = args.iter().any(|arg| arg == "--pair");
    let mut compare: Option<PathBuf> = None;
    let mut host_bin: Option<PathBuf> = None;
    let mut flags = args.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--gate" => gate = true,
            "--compare" => {
                compare = Some(PathBuf::from(
                    flags.next().context("--compare needs a value")?,
                ));
            }
            "--host-bin" => {
                host_bin = Some(PathBuf::from(
                    flags.next().context("--host-bin needs a value")?,
                ));
            }
            "--pair" | "--out" | "--label" | "--load-secs" | "--real" | "--runs"
            | "--host-commit" => {
                forwarded.push(flag.clone());
                forwarded.push(
                    flags
                        .next()
                        .with_context(|| format!("{flag} needs a value"))?
                        .clone(),
                );
            }
            _ => forwarded.push(flag.clone()),
        }
    }
    let host_bin = host_bin
        .or_else(|| std::env::var_os("OINO_BENCH_HOST_BIN").map(PathBuf::from))
        .or_else(|| {
            [
                root().join("apps/unipi/target/release/unipi-host"),
                PathBuf::from("/usr/bin/unipi-oino-host"),
            ]
            .into_iter()
            .find(|p| p.is_file())
        })
        .context(
            "no unipi-host binary: pass --host-bin (or build apps/unipi `-p unipi-host --release`)",
        )?;
    if gate {
        forwarded.push("--gate".into());
        if compare.is_none() && !paired {
            compare = latest_baseline();
        }
    }
    if let Some(compare) = compare {
        forwarded.push("--compare".into());
        forwarded.push(compare.display().to_string());
    }
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.args(["run", "--release", "-q", "-p", "oino-bench", "--", "host"])
        .args(&forwarded)
        .arg("--host-bin")
        .arg(&host_bin)
        .env("OINO_BENCH_ROOT", root())
        .env("OINO_BENCH_WORKDIR", root().join("target/bench"));
    if let Some(head) = git_head() {
        cmd.env("OINO_BENCH_GIT_HEAD", head);
    }
    if !cmd.status().context("running oino-bench host")?.success() {
        bail!("oino-bench host failed (see the record and notes above)");
    }
    Ok(())
}

/// The newest committed `bench/records/baseline-*.json` (names sort by date).
fn latest_baseline() -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root().join("bench/records"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("baseline-"))
        })
        .collect();
    found.sort();
    found.pop()
}

/// HEAD commit id recorded into every result (best-effort; absent → `null` in the JSON).
fn git_head() -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}
