//! `cargo xtask ti <cmd>`: the `ti` Ubuntu box as a remote build host for the root
//! workspace (docs/dev/v1/ti.md is historical: its GUI-test-host commands — start, shot,
//! soak, status, stop — went away with the GPUI desktop app on v2).
//!
//! `ti` runs other services and a logged-in desktop on `:0`/`:1`, so every command here works
//! only under `~/oino-test` and runs at `nice -n 10`.
//!
//! | Command | Does |
//! |---|---|
//! | `sync` | `HEAD` as a git bundle + `extensions/dist` → `ti:~/oino-test/oino` |
//! | `build [--jobs 6]` | `cargo build --release` of `oino` + `oino-kanboard` there |

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::remote::{self, Remote, run_bounded};

const HOST: &str = "ti@ti";
const CODE_DIR: &str = "oino-test/oino";
const RUN_DIR: &str = "oino-test/run";

const USAGE: &str = "usage: cargo xtask ti <sync | build> \
[sync] [build --jobs <n>]";

fn remote() -> Remote {
    Remote::linux(HOST)
}

fn flag<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>> {
    match args.iter().position(|a| a == name) {
        Some(i) => args
            .get(i + 1)
            .map(String::as_str)
            .map(Some)
            .with_context(|| format!("{name} needs a value")),
        None => Ok(None),
    }
}

/// `HEAD` (git bundle) + `extensions/dist` into `~/oino-test/oino` on ti.
fn sync() -> Result<()> {
    remote::sync(&remote(), CODE_DIR)
}

/// Release build of the CLI and the kanboard component on ti, streamed line by line.
fn build(jobs: u32, timeout: Duration) -> Result<()> {
    let remote = remote();
    let script = format!(
        "export PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         mkdir -p ~/{RUN_DIR}\n\
         cd ~/{CODE_DIR} || exit 1\n\
         rm -f ~/{RUN_DIR}/build.exit\n\
         echo $$ > ~/{RUN_DIR}/build.pid\n\
         {{ nice -n 10 cargo build --release -j {jobs} -p oino -p oino-kanboard; }} \
> ~/{RUN_DIR}/build.log 2>&1\n\
         echo $? > ~/{RUN_DIR}/build.exit\n"
    );
    remote.spawn_detached("ti-build", &script)?;
    let file = remote.put_script(
        "ti-build-status",
        &format!(
            "cd ~/{RUN_DIR} 2>/dev/null || exit 0\n\
             if [ -f build.exit ]; then echo \"exit $(cat build.exit)\"; else echo running; fi\n\
             tail -n 1 build.log 2>/dev/null | tr -d '\\r' | cut -c1-160\n"
        ),
    )?;
    let start = Instant::now();
    let mut last = String::new();
    let mut failures = 0;
    loop {
        std::thread::sleep(Duration::from_secs(30));
        let elapsed = start.elapsed();
        let (ok, out) = run_bounded(
            &mut remote.ssh(&remote.bash_command(&file)),
            Duration::from_secs(90),
            true,
        )?;
        if !ok {
            failures += 1;
            if failures >= 10 {
                bail!("{HOST} unreachable; the build may still be running there");
            }
            continue;
        }
        failures = 0;
        let mut lines = out.lines();
        let state = lines.next().unwrap_or("running").to_owned();
        let tail = lines.next().unwrap_or("").to_owned();
        if tail != last {
            println!("[{:>4}m] {tail}", elapsed.as_secs() / 60);
            last = tail;
        }
        if let Some(code) = state.strip_prefix("exit ") {
            if code.trim() == "0" {
                println!("built ~/{CODE_DIR}/target/release/oino on {HOST}");
                return Ok(());
            }
            let log = remote.bash(
                "ti-build-tail",
                &format!("tail -n 30 ~/{RUN_DIR}/build.log 2>/dev/null || true\n"),
                Duration::from_secs(60),
                true,
            )?;
            bail!("build on {HOST} exited with {}:\n{log}", code.trim());
        }
        if elapsed > timeout {
            let _ = remote.bash(
                "ti-build-kill",
                &format!(
                    "p=$(head -n 1 ~/{RUN_DIR}/build.pid 2>/dev/null); \
                     [ -n \"$p\" ] && kill -TERM \"$p\" 2>/dev/null; true\n"
                ),
                Duration::from_secs(60),
                true,
            );
            bail!(
                "build on {HOST} exceeded {} min and was stopped",
                timeout.as_secs() / 60
            );
        }
    }
}

pub fn ti(args: &[String]) -> Result<()> {
    let (cmd, rest) = args.split_first().context(USAGE)?;
    if matches!(cmd.as_str(), "-h" | "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    match cmd.as_str() {
        "sync" => sync(),
        "build" => {
            let jobs = match flag(rest, "--jobs")? {
                Some(j) => j.parse().context("--jobs needs a number")?,
                None => 6,
            };
            let timeout = match flag(rest, "--timeout-mins")? {
                Some(m) => Duration::from_mins(m.parse().context("--timeout-mins")?),
                None => Duration::from_mins(180),
            };
            build(jobs, timeout)
        }
        other => bail!("unknown `ti` command {other}\n{USAGE}"),
    }
}
