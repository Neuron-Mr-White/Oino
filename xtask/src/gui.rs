//! `cargo xtask gui suite <name>`: the regression suites of the GUI automation driver
//! (docs/dev/v1/gui-driver.md).
//!
//! A suite is a manifest under `docs/dev/v1/gui/suites/<name>.json` that lists scenario files
//! in run order, grouped by the feature area (for `unipi`: one group per ported package). The
//! runner is a thin wrapper around `scripts/gui-drive.py --ti run <file>`: every scenario
//! restarts the app on the `ti` GUI host itself, so the suite is a plain sequence of
//! independent runs, and the report is written to `target/gui-suite/<name>.json`.
//!
//! | Command | Does |
//! |---|---|
//! | `gui list <suite>` | prints the manifest, its groups and the scenario files |
//! | `gui suite <suite>` | runs every scenario in order, prints PASS/FAIL, one report |
//!
//! Options: `--only <group>[,<group>…]` limits the run to these groups (or scenario names);
//! `--backend demo\|faux\|real` overrides each scenario's own backend;
//! `--no-restart` reuses the app that is already running (a single scenario only).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::root;

const USAGE: &str = "usage: cargo xtask gui <list|suite> <name> [--only <group>[,<group>…]] \
[--backend demo|faux|real] [--no-restart]";

/// One scenario row of a suite manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct Scenario {
    /// Feature area / package the scenario covers (`core`, `kanban`, …).
    group: String,
    /// Scenario file, relative to the repository root.
    file: String,
    /// What the scenario proves, in one line (copied into the report).
    #[serde(default)]
    covers: String,
}

/// A suite manifest: the ordered scenarios that make up one regression run.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct Suite {
    /// Suite name (the file stem).
    name: String,
    /// What the suite is for.
    #[serde(default)]
    description: String,
    /// Scenarios in run order.
    scenarios: Vec<Scenario>,
}

/// One scenario result.
#[derive(Debug, Clone, Serialize)]
struct Outcome {
    group: String,
    file: String,
    covers: String,
    ok: bool,
    seconds: f64,
    /// Last non-empty lines of output (`failure` keeps more than `pass`).
    tail: Vec<String>,
}

fn suites_dir() -> PathBuf {
    root().join("docs/dev/v1/gui/suites")
}

fn load(name: &str) -> Result<Suite> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("suite name {name:?} must be alphanumeric with - or _");
    }
    let path = suites_dir().join(format!("{name}.json"));
    let bytes =
        std::fs::read(&path).with_context(|| format!("no suite manifest at {}", path.display()))?;
    let mut suite: Suite =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    if suite.name.is_empty() {
        name.clone_into(&mut suite.name);
    }
    if suite.scenarios.is_empty() {
        bail!("suite {name} lists no scenarios");
    }
    for scenario in &suite.scenarios {
        let file = root().join(&scenario.file);
        if !file.is_file() {
            bail!(
                "suite {name}: scenario {} does not exist ({})",
                scenario.file,
                file.display()
            );
        }
    }
    Ok(suite)
}

/// `--only a,b` filters by group name or by scenario file stem.
fn selected(suite: &Suite, only: Option<&str>) -> Vec<Scenario> {
    let Some(only) = only else {
        return suite.scenarios.clone();
    };
    let wanted: Vec<&str> = only
        .split(',')
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();
    suite
        .scenarios
        .iter()
        .filter(|s| {
            let stem = Path::new(&s.file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            wanted.iter().any(|w| *w == s.group || *w == stem)
        })
        .cloned()
        .collect()
}

fn list(name: &str) -> Result<()> {
    let suite = load(name)?;
    println!("{} — {}", suite.name, suite.description);
    let mut current = String::new();
    let mut groups = 0_usize;
    for scenario in &suite.scenarios {
        if scenario.group != current {
            current.clone_from(&scenario.group);
            groups += 1;
            println!("\n{}", scenario.group);
        }
        println!("  {:<44} {}", scenario.file, scenario.covers);
    }
    println!("\n{} scenarios in {groups} groups", suite.scenarios.len());
    Ok(())
}

/// Keeps the last lines of a scenario's output for the report.
fn tail(output: &str, keep: usize) -> Vec<String> {
    let mut lines: Vec<String> = output
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if lines.len() > keep {
        lines.drain(..lines.len() - keep);
    }
    lines
}

fn run_one(path: &Path, backend: Option<&str>, no_restart: bool) -> Result<(bool, f64, String)> {
    let driver = root().join("scripts/gui-drive.py");
    let mut command = Command::new("python3");
    command
        .arg(&driver)
        .arg("--ti")
        .arg("run")
        .arg(path)
        .current_dir(root());
    if let Some(backend) = backend {
        command.arg("--backend").arg(backend);
    }
    if no_restart {
        command.arg("--no-restart");
    }
    let started = Instant::now();
    let output = command
        .output()
        .with_context(|| format!("running {}", driver.display()))?;
    let seconds = started.elapsed().as_secs_f64();
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), seconds, text))
}

fn suite(name: &str, only: Option<&str>, backend: Option<&str>, no_restart: bool) -> Result<()> {
    let suite = load(name)?;
    let scenarios = selected(&suite, only);
    if scenarios.is_empty() {
        bail!("suite {name}: --only selected no scenario");
    }
    if no_restart && scenarios.len() > 1 {
        bail!("--no-restart needs a single scenario (use --only)");
    }
    let display = root().join("target/gui-suite");
    std::fs::create_dir_all(&display)?;

    println!(
        "{}: {} scenarios{}",
        suite.name,
        scenarios.len(),
        backend.map_or_else(String::new, |b| format!(" (backend {b})"))
    );
    let mut outcomes = Vec::new();
    for (index, scenario) in scenarios.iter().enumerate() {
        let path = root().join(&scenario.file);
        print!("[{}/{}] {:<44}", index + 1, scenarios.len(), scenario.file);
        std::io::Write::flush(&mut std::io::stdout())?;
        let (ok, seconds, output) = run_one(&path, backend, no_restart)?;
        println!(" {} {:.0}s", if ok { "PASS" } else { "FAIL" }, seconds);
        if !ok {
            for line in tail(&output, 24) {
                println!("      {line}");
            }
        }
        outcomes.push(Outcome {
            group: scenario.group.clone(),
            file: scenario.file.clone(),
            covers: scenario.covers.clone(),
            ok,
            seconds,
            tail: tail(&output, if ok { 3 } else { 24 }),
        });
    }

    let passed = outcomes.iter().filter(|o| o.ok).count();
    let failed: Vec<&Outcome> = outcomes.iter().filter(|o| !o.ok).collect();
    let report = serde_json::json!({
        "suite": suite.name,
        "backend": backend,
        "passed": passed,
        "failed": failed.len(),
        "outcomes": outcomes,
    });
    let out = display.join(format!("{}.json", suite.name));
    std::fs::write(&out, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "\n{}: {passed}/{} passed — report {}{}",
        suite.name,
        outcomes.len(),
        out.display(),
        if failed.is_empty() {
            String::new()
        } else {
            format!(
                "; failed: {}",
                failed
                    .iter()
                    .map(|o| o.file.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    if !failed.is_empty() {
        bail!("{} of {} scenarios failed", failed.len(), outcomes.len());
    }
    Ok(())
}

/// `cargo xtask gui …` entry point (see the module docs for the commands).
pub fn gui(args: &[String]) -> Result<()> {
    let Some(command) = args.first() else {
        bail!("{USAGE}");
    };
    let name = args.get(1).with_context(|| USAGE)?;
    let rest = &args[2..];
    let flag = |what: &str| -> Result<Option<&str>> {
        match rest.iter().position(|a| a == what) {
            Some(i) => rest
                .get(i + 1)
                .map(|v| Some(v.as_str()))
                .with_context(|| format!("{what} needs a value")),
            None => Ok(None),
        }
    };
    let no_restart = rest.iter().any(|a| a == "--no-restart");
    match command.as_str() {
        "list" => list(name),
        "suite" => {
            let backend = flag("--backend")?;
            if let Some(backend) = backend
                && !matches!(backend, "demo" | "faux" | "real")
            {
                bail!("--backend must be demo, faux or real");
            }
            suite(name, flag("--only")?, backend, no_restart)
        }
        other => bail!("unknown gui command {other:?}; {USAGE}"),
    }
}
