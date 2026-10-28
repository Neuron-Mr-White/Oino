//! `cargo xtask ti <cmd>`: the `ti` Ubuntu box as Oino Desktop's GUI test host
//! (docs/dev/v1/ti.md).
//!
//! `ti` runs other services and a logged-in desktop on `:0`/`:1`, so every command here works
//! only under `~/oino-test`, renders on a private `Xvfb :93` with its own dbus session, runs
//! at `nice -n 10`, and signals only the PIDs it wrote to `~/oino-test/run/*.pid`.
//!
//! | Command | Does |
//! |---|---|
//! | `sync` | `HEAD` as a git bundle + `extensions/dist` → `ti:~/oino-test/oino` |
//! | `build [--jobs 6]` | `cargo build --release` of `oino` + `oino-desktop` there |
//! | `start [--demo] [--faux] [--cwd <dir>] [--env K=V]` | scratch agent dir + `Xvfb :93` + dbus + app (extra env repeatable) |
//! | `shot [--name <n>] [--out <dir>]` | app-window PNG → `docs/dev/v1/design/shots/ti/<n>.png` |
//! | `status` | PIDs, build state, whether `:93` is free |
//! | `soak [--minutes 10] [--backend demo\|faux]` | `apps/oino-desktop/scripts/soak.sh` there |
//! | `stop` | kills only the recorded PIDs and removes the pid files |
//!
//! Credentials: `start` copies this PC's `~/.oino/auth.json` (mode 600) to the scratch agent
//! dir on ti; the file is never printed or written into the repository.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::remote::{self, Remote, mib, run_bounded, shell_quote};
use crate::root;

const HOST: &str = "ti@ti";
const CODE_DIR: &str = "oino-test/oino";
const RUN_DIR: &str = "oino-test/run";
const AGENT_DIR: &str = "oino-test/agent";
const PROJECT_DIR: &str = "oino-test/project";
const SOAK_DIR: &str = "oino-test/soak";
const DISPLAY: &str = ":93";
const SHOT_DIR: &str = "docs/dev/v1/design/shots/ti";
const DESKTOP_BIN: &str = "oino-test/oino/apps/target/release/oino-desktop";

const USAGE: &str = "usage: cargo xtask ti <sync | build | start | shot | status | soak | stop> \
[sync] [build --jobs <n>] [start --demo|--faux] [start --cwd <dir>] [start --env K=V ...] \
[shot --name <n>] [shot --out <dir>] [soak --minutes <n> --backend demo|faux]";

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

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Every `--env K=V` value (repeatable; `scripts/gui-drive.py` passes the automation socket
/// and the demo scene this way).
fn envs(args: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--env" {
            let value = args.get(i + 1).context("--env needs KEY=VALUE")?;
            if !value.contains('=') {
                bail!("--env needs KEY=VALUE, got {value}");
            }
            out.push(value.clone());
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(out)
}

/// `HEAD` (git bundle) + `extensions/dist` into `~/oino-test/oino` on ti.
fn sync() -> Result<()> {
    remote::sync(&remote(), CODE_DIR)
}

/// Release build of the CLI and the desktop app on ti, streamed line by line.
fn build(jobs: u32, timeout: Duration) -> Result<()> {
    let remote = remote();
    let script = format!(
        "export PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         mkdir -p ~/{RUN_DIR}\n\
         cd ~/{CODE_DIR} || exit 1\n\
         rm -f ~/{RUN_DIR}/build.exit\n\
         echo $$ > ~/{RUN_DIR}/build.pid\n\
         {{ nice -n 10 cargo build --release -j {jobs} -p oino && cd apps && \
nice -n 10 cargo build --release -j {jobs} -p oino-desktop; }} \
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
                println!("built ~/{DESKTOP_BIN} on {HOST}");
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

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Uploads this PC's `~/.oino/auth.json` (plus a `faux` key when asked) to the scratch agent
/// dir on ti, mode 600. The file is never printed or copied into the repository.
fn upload_auth(remote: &Remote, faux: bool) -> Result<()> {
    let local =
        std::env::var_os("OINO_AUTH").map_or_else(|| home().join(".oino/auth.json"), PathBuf::from);
    if !local.is_file() {
        bail!("no auth file at {} (set OINO_AUTH)", local.display());
    }
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&local).context("reading the auth file")?)?;
    if faux && let Some(map) = doc.as_object_mut() {
        map.entry("faux")
            .or_insert_with(|| serde_json::json!({"type": "api_key", "key": "k"}));
    }
    let tmp = root().join("target/tmp").join("ti-auth.json");
    std::fs::create_dir_all(tmp.parent().context("tmp dir")?)?;
    std::fs::write(&tmp, serde_json::to_vec(&doc)?)?;
    let result = remote.upload(
        &tmp,
        &format!("{AGENT_DIR}/auth.json"),
        Duration::from_mins(2),
    );
    let _ = std::fs::remove_file(&tmp);
    result?;
    let out = remote.bash(
        "ti-auth-mode",
        &format!(
            "chmod 600 ~/{AGENT_DIR}/auth.json && stat -c '%A %s bytes' ~/{AGENT_DIR}/auth.json\n"
        ),
        Duration::from_secs(60),
        true,
    )?;
    println!("agent dir auth.json: {}", out.trim());
    Ok(())
}

/// Prepares the scratch agent dir (auth, optional faux fixture) and starts `Xvfb :93` + a
/// private dbus session + the app. Records every PID in `~/oino-test/run/`.
fn start(faux: bool, demo: bool, app_args: &[String], extra_env: &[String]) -> Result<()> {
    let remote = remote();
    // A faux run needs the fixture and the provider defaults soak.sh sets up; `--demo` needs
    // neither, and a real run reads the copied auth.json.
    let faux_setup = if faux {
        format!(
            "mkdir -p ~/{AGENT_DIR}/extensions\n\
             rm -rf ~/{AGENT_DIR}/extensions/provider-faux\n\
             cp -r ~/{CODE_DIR}/extensions/dist/oino.fixture.provider-faux \
                ~/{AGENT_DIR}/extensions/provider-faux\n\
             printf '%s\\n' '{{\"defaultProvider\":\"faux\",\"defaultModel\":\"faux-1\"}}' \
                > ~/{AGENT_DIR}/settings.json\n\
             printf '%s\\n' '{{\"onboarded\":true}}' > ~/{AGENT_DIR}/desktop.json\n"
        )
    } else {
        String::new()
    };
    remote.bash(
        "ti-agent",
        &format!(
            "mkdir -p ~/{AGENT_DIR}/logs ~/{RUN_DIR} ~/{PROJECT_DIR}\n\
             printf 'ti project\\n' > ~/{PROJECT_DIR}/README.md\n{faux_setup}"
        ),
        Duration::from_secs(120),
        true,
    )?;
    if !demo {
        upload_auth(&remote, faux)?;
    }
    let args: Vec<String> = app_args.iter().map(|a| shell_quote(a)).collect();
    // Extra environment for the app (repeatable `--env K=V`).
    let mut env_line = String::new();
    for pair in extra_env {
        let (key, value) = pair.split_once('=').context("--env needs KEY=VALUE")?;
        let quoted = shell_quote(value);
        write!(env_line, "{key}={quoted} ").context("building the --env line")?;
    }
    let script = format!(
        "set -u\n\
         export PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         run=~/{RUN_DIR} agent=~/{AGENT_DIR} bin=~/{DESKTOP_BIN}\n\
         for f in xvfb dbus app; do\n\
           if [ -f \"$run/$f.pid\" ]; then echo \"$f already running (pid $(cat \"$run/$f.pid\")); \
run: cargo xtask ti stop\"; exit 3; fi\n\
         done\n\
         [ -x \"$bin\" ] || {{ echo \"no binary at $bin (run: cargo xtask ti build)\"; exit 1; }}\n\
         if [ -e /tmp/.X11-unix/X93 ] || [ -e /tmp/.X93-lock ]; then \
echo 'display {DISPLAY} is busy on this host'; exit 3; fi\n\
         Xvfb {DISPLAY} -screen 0 1600x1000x24 -nolisten tcp >\"$run/xvfb.log\" 2>&1 &\n\
         echo $! > \"$run/xvfb.pid\"\n\
         for _ in $(seq 50); do [ -e /tmp/.X11-unix/X93 ] && break; sleep 0.2; done\n\
         eval \"$(dbus-launch --sh-syntax)\"\n\
         echo \"$DBUS_SESSION_BUS_PID\" > \"$run/dbus.pid\"\n\
         echo \"$DBUS_SESSION_BUS_ADDRESS\" > \"$run/dbus.address\"\n\
         cd ~/{PROJECT_DIR} || exit 1\n\
         nohup env DISPLAY={DISPLAY} DBUS_SESSION_BUS_ADDRESS=\"$DBUS_SESSION_BUS_ADDRESS\" \
OINO_AGENT_DIR=\"$agent\" OINO_DESKTOP_SIZE=1600x1000 OINO_REDUCE_MOTION=1 {env_line}\
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
\"$bin\" {args} >\"$run/app.log\" 2>&1 </dev/null &\n\
         echo $! > \"$run/app.pid\"\n\
         sleep 2\n\
         app=$(cat \"$run/app.pid\")\n\
         if ! kill -0 \"$app\" 2>/dev/null; then echo 'app exited; log tail:'; \
tail -n 20 \"$run/app.log\"; exit 1; fi\n\
         echo \"app pid $app on {DISPLAY} (xvfb $(cat \"$run/xvfb.pid\"), \
dbus $(cat \"$run/dbus.pid\"))\"\n",
        args = args.join(" "),
        env_line = env_line
    );
    let out = remote.bash("ti-start", &script, Duration::from_secs(180), true)?;
    print!("{out}");
    Ok(())
}

/// Screenshots the app window from `:93` and fetches the PNG to `<out>/<name>.png`.
fn shot(name: &str, out: &Path) -> Result<()> {
    let remote = remote();
    let script = format!(
        "run=~/{RUN_DIR}\n\
         [ -f \"$run/app.pid\" ] && kill -0 \"$(cat \"$run/app.pid\")\" 2>/dev/null || \
{{ echo 'no app running on {DISPLAY}'; exit 1; }}\n\
         export DISPLAY={DISPLAY}\n\
         id=$(xdotool search --onlyvisible --class dev.oino.desktop 2>/dev/null | head -1)\n\
         [ -n \"$id\" ] || id=$(xdotool search --onlyvisible --name Oino 2>/dev/null | head -1)\n\
         window=root\n\
         [ -n \"$id\" ] && window=$id\n\
         import -window \"$window\" \"$run/shot.png\"\n\
         identify -format '%wx%h\\n' \"$run/shot.png\" | tr -d '\\n'\n\
         echo \" $window\"\n"
    );
    let geometry = remote.bash("ti-shot", &script, Duration::from_secs(120), true)?;
    let mut parts = geometry.split_whitespace();
    let size = parts.next().unwrap_or("?");
    let window = parts.next().unwrap_or("?");
    let dir = if out.is_absolute() {
        out.to_path_buf()
    } else {
        root().join(out)
    };
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{name}.png"));
    remote.download(
        &format!("{RUN_DIR}/shot.png"),
        &target,
        Duration::from_mins(2),
    )?;
    let bytes = std::fs::metadata(&target).map_or(0, |m| m.len());
    println!(
        "{} ({size}, {} MiB, window {window})",
        target.display(),
        mib(bytes)
    );
    Ok(())
}

fn status() -> Result<()> {
    let remote = remote();
    let out = remote.bash(
        "ti-status",
        &format!(
            "run=~/{RUN_DIR}\n\
             for f in xvfb dbus app; do\n\
               p=\"$run/$f.pid\"\n\
               if [ -f \"$p\" ]; then pid=$(cat \"$p\"); \
echo \"$f pid $pid $(kill -0 \"$pid\" 2>/dev/null && echo running || echo dead)\"; \
else echo \"$f not started\"; fi\n\
             done\n\
             [ -e /tmp/.X11-unix/X93 ] && echo '{DISPLAY} in use' || echo '{DISPLAY} free'\n\
             if [ -f \"$run/build.exit\" ]; then echo \"build exit $(cat \"$run/build.exit\")\"; \
else echo 'build not run'; fi\n\
             ls -lh ~/{DESKTOP_BIN} 2>/dev/null || echo 'no desktop binary (cargo xtask ti build)'\n"
        ),
        Duration::from_secs(120),
        true,
    )?;
    print!("{out}");
    Ok(())
}

/// `apps/oino-desktop/scripts/soak.sh` on ti; the summary is fetched to `target/ti-soak/`.
fn soak(minutes: u32, backend: &str, timeout: Duration) -> Result<()> {
    let remote = remote();
    let script = format!(
        "export PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         run=~/{RUN_DIR} out=~/{SOAK_DIR}\n\
         mkdir -p \"$run\" \"$out\"\n\
         rm -f \"$run/soak.exit\"\n\
         echo $$ > \"$run/soak.pid\"\n\
         cd ~/{CODE_DIR} || exit 1\n\
         nice -n 10 apps/oino-desktop/scripts/soak.sh --minutes {minutes} --backend {backend} \
--bin ~/{DESKTOP_BIN} --out \"$out\" > \"$run/soak.log\" 2>&1\n\
         echo $? > \"$run/soak.exit\"\n"
    );
    remote.spawn_detached("ti-soak", &script)?;
    let file = remote.put_script(
        "ti-soak-status",
        &format!(
            "cd ~/{RUN_DIR} 2>/dev/null || exit 0\n\
             if [ -f soak.exit ]; then echo \"exit $(cat soak.exit)\"; else echo running; fi\n\
             tail -n 1 ~/{SOAK_DIR}/summary.txt 2>/dev/null || \
tail -n 1 soak.log 2>/dev/null | tr -d '\\r' | cut -c1-160\n"
        ),
    )?;
    let start = Instant::now();
    let mut last = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(30));
        let elapsed = start.elapsed();
        let (ok, out) = run_bounded(
            &mut remote.ssh(&remote.bash_command(&file)),
            Duration::from_secs(90),
            true,
        )?;
        if !ok {
            if elapsed > timeout + Duration::from_mins(20) {
                bail!("{HOST} unreachable; the soak may still be running there");
            }
            continue;
        }
        let mut lines = out.lines();
        let state = lines.next().unwrap_or("running").to_owned();
        let tail = lines.next().unwrap_or("").to_owned();
        if tail != last {
            println!("[{:>4}m] {tail}", elapsed.as_secs() / 60);
            last = tail;
        }
        if let Some(code) = state.strip_prefix("exit ") {
            let local = root().join("target/ti-soak");
            std::fs::create_dir_all(&local)?;
            for name in ["summary.txt", "samples.tsv", "app.out", "xvfb.log"] {
                let from = format!("{SOAK_DIR}/{name}");
                let _ = remote.download(&from, &local.join(name), Duration::from_mins(5));
            }
            match std::fs::read_to_string(local.join("summary.txt")) {
                Ok(summary) => {
                    println!("\n{summary}");
                    println!("fetched to {}", local.display());
                }
                Err(error) => println!("no summary.txt ({error}); logs in {}", local.display()),
            }
            let _ = remote.bash(
                "ti-soak-clean",
                &format!("rm -f ~/{RUN_DIR}/soak.pid ~/{RUN_DIR}/soak.exit\n"),
                Duration::from_secs(60),
                true,
            );
            if code.trim() != "0" {
                bail!("soak on {HOST} exited with {}", code.trim());
            }
            return Ok(());
        }
        if elapsed > timeout {
            println!("soak is still running after {timeout:?}; it keeps going on ti");
        }
    }
}

/// Kills only the PIDs recorded in `~/oino-test/run/`, after checking they are still ours.
fn stop() -> Result<()> {
    let remote = remote();
    let out = remote.bash(
        "ti-stop",
        &format!(
            "set -u\n\
             run=~/{RUN_DIR}\n\
             for f in app dbus xvfb; do\n\
               p=\"$run/$f.pid\"\n\
               [ -f \"$p\" ] || continue\n\
               pid=$(cat \"$p\")\n\
               cmd=$(tr '\\0' ' ' < \"/proc/$pid/cmdline\" 2>/dev/null || true)\n\
               case \"$f:$cmd\" in\n\
                 app:*{CODE_DIR}*|dbus:*dbus-daemon*|xvfb:*Xvfb*{DISPLAY}*)\n\
                   kill -TERM \"$pid\" 2>/dev/null || true\n\
                   for _ in $(seq 20); do kill -0 \"$pid\" 2>/dev/null || break; sleep 0.25; done\n\
                   kill -0 \"$pid\" 2>/dev/null && kill -KILL \"$pid\" 2>/dev/null || true\n\
                   echo \"stopped $f (pid $pid)\" ;;\n\
                 *) echo \"$f pid $pid is not ours ('$cmd'); leaving it alone\" ;;\n\
               esac\n\
               rm -f \"$p\"\n\
             done\n\
             rm -f \"$run/dbus.address\" \"$run/shot.png\"\n\
             sleep 1\n\
             left=0\n\
             for f in xvfb dbus app; do [ -f \"$run/$f.pid\" ] && left=$((left + 1)); done\n\
             echo \"$left of 3 test PIDs still recorded\"\n"
        ),
        Duration::from_secs(180),
        true,
    )?;
    print!("{out}");
    Ok(())
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
        "start" => {
            let mut app_args: Vec<String> = Vec::new();
            if has(rest, "--demo") {
                app_args.push("--demo".into());
            }
            if let Some(cwd) = flag(rest, "--cwd")? {
                app_args.push("--cwd".into());
                app_args.push(cwd.into());
            }
            start(
                has(rest, "--faux"),
                has(rest, "--demo"),
                &app_args,
                &envs(rest)?,
            )
        }
        "shot" => {
            let name = flag(rest, "--name")?.unwrap_or("desktop");
            let out = flag(rest, "--out")?.unwrap_or(SHOT_DIR);
            shot(name, Path::new(out))
        }
        "status" => status(),
        "soak" => {
            let minutes = match flag(rest, "--minutes")? {
                Some(m) => m.parse().context("--minutes needs a number")?,
                None => 10,
            };
            let backend = flag(rest, "--backend")?.unwrap_or("demo");
            if !matches!(backend, "demo" | "faux") {
                bail!("unknown --backend {backend}");
            }
            soak(
                minutes,
                backend,
                Duration::from_secs(u64::from(minutes) * 60 + 600),
            )
        }
        "stop" => stop(),
        other => bail!("unknown `ti` command {other}\n{USAGE}"),
    }
}
