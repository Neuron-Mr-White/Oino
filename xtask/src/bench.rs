//! `cargo xtask bench <suite> [--n N] [--out dir]` (WP-C11): drives `crates/oino-bench`
//! and lands its results under `bench/results/`.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::root;

pub(crate) fn bench(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("subagents") => subagents(&args[1..]),
        Some(other) => bail!("unknown bench suite '{other}' (known: subagents)"),
        None => bail!("usage: cargo xtask bench <subagents --n <1|10|50> [--out dir]>"),
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
