//! `cargo xtask remote-build <host>`: release builds on a build host over SSH (quality §6
//! `release` job).
//!
//! 1. **sync**: the committed `HEAD` goes over as a `git bundle` (incremental when the host
//!    already has an ancestor) into `~/<dir>` (detached checkout, `core.autocrlf=false`), and
//!    the local `extensions/dist` as a tarball (hosts have no wasm toolchain; `build-host.sh`
//!    verifies it with `cargo xtask checksums`).
//! 2. **build**: `scripts/release/build-host.sh` runs detached on the host (nohup on Unix,
//!    `Win32_Process.Create` on Windows, so a dropped SSH session does not kill it) and is
//!    polled until it writes its exit code or `--timeout-mins` passes.
//! 3. **fetch**: `~/<dir>/target/release-artifacts/*` lands in
//!    `target/release-artifacts/<target>/`; the build log in `target/release-artifacts/logs/`.
//!
//! `local` runs `build-host.sh` in this checkout instead. Unix hosts run scripts through the
//! login shell (`$SHELL -lc`, for rustup/Homebrew PATH); Windows hosts (PowerShell over SSH)
//! through Git Bash (`OINO_REMOTE_BASH`, default `C:\Program Files\Git\bin\bash.exe`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::root;

const USAGE: &str = "usage: cargo xtask remote-build <host|local> [--os linux|macos|windows] \
[--dir <remote dir>] [--jobs <n>] [--timeout-mins <n>] [--no-sync] [--fetch-only] \
[-- <build-host.sh args: --universal --formats <list> --no-shot --android --serial <s> …>]";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Os {
    Linux,
    Macos,
    Windows,
}

struct Opts {
    host: String,
    os: Option<Os>,
    dir: String,
    jobs: u32,
    timeout: Duration,
    sync: bool,
    build: bool,
    script_args: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts> {
    let mut it = args.iter();
    let host = it.next().context(USAGE)?.clone();
    if host.starts_with('-') {
        bail!(USAGE);
    }
    let mut opts = Opts {
        host,
        os: None,
        dir: "oino-release".into(),
        jobs: 2,
        timeout: Duration::from_mins(240),
        sync: true,
        build: true,
        script_args: Vec::new(),
    };
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--os" => {
                opts.os = Some(match value()?.as_str() {
                    "linux" => Os::Linux,
                    "macos" => Os::Macos,
                    "windows" => Os::Windows,
                    other => bail!("unknown --os {other}"),
                });
            }
            "--dir" => opts.dir.clone_from(value()?),
            "--jobs" => opts.jobs = value()?.parse()?,
            "--timeout-mins" => opts.timeout = Duration::from_mins(value()?.parse::<u64>()?),
            "--no-sync" => opts.sync = false,
            "--fetch-only" => {
                opts.sync = false;
                opts.build = false;
            }
            "--" => {
                opts.script_args = it.by_ref().cloned().collect();
            }
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    if opts.dir.contains(['\'', '"', ' ', '\\']) {
        bail!("--dir must be a plain path relative to the remote home");
    }
    Ok(opts)
}

#[allow(clippy::cast_precision_loss, reason = "display only")]
pub(crate) fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_048_576.0)
}

pub(crate) fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@+".contains(c))
    {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Runs `cmd`, killing it after `limit`. Returns (success, stdout).
pub(crate) fn run_bounded(
    cmd: &mut Command,
    limit: Duration,
    capture: bool,
) -> Result<(bool, String)> {
    if capture {
        cmd.stdout(Stdio::piped());
    }
    cmd.stdin(Stdio::null());
    let mut child = cmd.spawn().with_context(|| format!("spawning {cmd:?}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut out = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                std::io::Read::read_to_string(&mut stdout, &mut out)?;
            }
            return Ok((status.success(), out));
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Ok((false, String::new()));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub(crate) struct Remote {
    host: String,
    os: Os,
    bash: String,
}

const SSH_OPTS: &[&str] = &[
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=20",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=4",
];

impl Remote {
    /// A Linux build host reached with the same SSH plumbing (`cargo xtask ti`).
    pub(crate) fn linux(host: &str) -> Self {
        Self {
            host: host.into(),
            os: Os::Linux,
            bash: String::new(),
        }
    }

    pub(crate) fn ssh(&self, command: &str) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.args(SSH_OPTS).arg(&self.host).arg(command);
        cmd
    }

    /// Copies `local` to `~/<name>` on the host.
    pub(crate) fn upload(&self, local: &Path, name: &str, limit: Duration) -> Result<()> {
        let mut cmd = Command::new("scp");
        cmd.arg("-q")
            .args(SSH_OPTS)
            .arg(local)
            .arg(format!("{}:{name}", self.host));
        let (ok, _) = run_bounded(&mut cmd, limit, false)?;
        if !ok {
            bail!("scp {} → {}:{name} failed", local.display(), self.host);
        }
        Ok(())
    }

    /// Copies `~/<name>` on the host to `local`.
    pub(crate) fn download(&self, name: &str, local: &Path, limit: Duration) -> Result<()> {
        let mut cmd = Command::new("scp");
        cmd.arg("-q")
            .args(SSH_OPTS)
            .arg(format!("{}:{name}", self.host))
            .arg(local);
        let (ok, _) = run_bounded(&mut cmd, limit, false)?;
        if !ok {
            bail!("scp {}:{} → {} failed", self.host, name, local.display());
        }
        Ok(())
    }

    /// Uploads `script` as `~/oino-release-<name>.sh`.
    pub(crate) fn put_script(&self, name: &str, script: &str) -> Result<String> {
        let file = format!("oino-release-{name}.sh");
        let local = root()
            .join("target/tmp")
            .join(format!("{}-{file}", self.host));
        std::fs::create_dir_all(local.parent().context("tmp dir")?)?;
        std::fs::write(&local, script)?;
        self.upload(&local, &file, Duration::from_mins(2))?;
        Ok(file)
    }

    /// The SSH command line that runs `~/<file>` with bash.
    pub(crate) fn bash_command(&self, file: &str) -> String {
        match self.os {
            Os::Windows => format!("& '{}' -l \"$HOME/{file}\"", self.bash),
            Os::Linux | Os::Macos => format!("$SHELL -lc 'bash ~/{file}'"),
        }
    }

    /// Runs a bash script on the host, bounded by `limit`.
    pub(crate) fn bash(
        &self,
        name: &str,
        script: &str,
        limit: Duration,
        capture: bool,
    ) -> Result<String> {
        let file = self.put_script(name, script)?;
        let (ok, out) = run_bounded(&mut self.ssh(&self.bash_command(&file)), limit, capture)?;
        if !ok {
            bail!("{name} on {} failed (or timed out)", self.host);
        }
        Ok(out)
    }

    /// Starts a bash script on the host without waiting for it.
    pub(crate) fn spawn_detached(&self, name: &str, script: &str) -> Result<()> {
        let file = self.put_script(name, script)?;
        let command = match self.os {
            // WMI creates the process outside the SSH session's job object.
            Os::Windows => format!(
                "$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments \
                 @{{CommandLine = ('\"{}\" -l \"' + $HOME + '\\{file}\"'); CurrentDirectory = $HOME}}; \
                 if ($r.ReturnValue -ne 0) {{ exit 1 }}; 'pid ' + $r.ProcessId",
                self.bash
            ),
            Os::Linux | Os::Macos => {
                format!("nohup $SHELL -lc 'bash ~/{file}' >/dev/null 2>&1 </dev/null & echo pid $!")
            }
        };
        let (ok, out) = run_bounded(&mut self.ssh(&command), Duration::from_secs(90), true)?;
        if !ok {
            bail!("could not start {name} on {}", self.host);
        }
        println!("started {name} on {} ({})", self.host, out.trim());
        Ok(())
    }
}

fn detect_os(host: &str) -> Result<Os> {
    let mut cmd = Command::new("ssh");
    cmd.args(SSH_OPTS).arg(host).arg("uname -s");
    let (_, out) = run_bounded(&mut cmd, Duration::from_secs(60), true)?;
    Ok(match out.trim() {
        "Darwin" => Os::Macos,
        "Linux" => Os::Linux,
        _ => Os::Windows,
    })
}

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .current_dir(root())
        .args(args)
        .output()
        .context("git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub(crate) fn sync(remote: &Remote, dir: &str) -> Result<()> {
    let head = git(&["rev-parse", "HEAD"])?;
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() {
        println!("warning: uncommitted changes are not synced (only HEAD {head:.10} is)");
    }
    let remote_head = remote
        .bash(
            "head",
            &format!("git -C ~/{dir} rev-parse HEAD 2>/dev/null || true\n"),
            Duration::from_secs(90),
            true,
        )?
        .trim()
        .to_owned();
    let tmp = root().join("target/tmp");
    std::fs::create_dir_all(&tmp)?;
    let mut fetch = String::new();
    if remote_head == head {
        println!("{} already has {head:.10}", remote.host);
    } else {
        let bundle = tmp.join(format!("{}-oino.bundle", remote.host));
        let incremental = !remote_head.is_empty()
            && git(&["merge-base", "--is-ancestor", &remote_head, "HEAD"]).is_ok();
        let range = if incremental {
            format!("{remote_head}..HEAD")
        } else {
            "HEAD".into()
        };
        git(&["bundle", "create", &bundle.to_string_lossy(), &range])?;
        let size = std::fs::metadata(&bundle)?.len();
        println!("uploading git bundle {range} ({} MiB)", mib(size));
        remote.upload(&bundle, "oino-release.bundle", Duration::from_mins(30))?;
        fetch = "git fetch -q ~/oino-release.bundle HEAD\ngit checkout -q -f --detach FETCH_HEAD\n"
            .into();
    }
    let dist = tmp.join(format!("{}-dist.tgz", remote.host));
    let status = Command::new("tar")
        .current_dir(root().join("extensions"))
        .args(["-czf"])
        .arg(&dist)
        .arg("dist")
        .status()?;
    if !status.success() {
        bail!("tar extensions/dist failed");
    }
    println!(
        "uploading extensions/dist ({} MiB)",
        mib(std::fs::metadata(&dist)?.len())
    );
    remote.upload(&dist, "oino-release-dist.tgz", Duration::from_mins(30))?;
    let script = format!(
        "set -euo pipefail\n\
         dir=~/{dir}\n\
         if [ ! -d \"$dir/.git\" ]; then git init -q \"$dir\"; fi\n\
         cd \"$dir\"\n\
         git config core.autocrlf false\n\
         {fetch}\
         rm -rf extensions/dist\n\
         mkdir -p extensions\n\
         tar -xzf ~/oino-release-dist.tgz -C extensions\n\
         echo synced \"$(git rev-parse --short HEAD)\"\n"
    );
    let out = remote.bash("sync", &script, Duration::from_mins(15), true)?;
    print!("{out}");
    Ok(())
}

fn build(remote: &Remote, opts: &Opts) -> Result<()> {
    let args: Vec<String> = opts.script_args.iter().map(|a| shell_quote(a)).collect();
    let dir = &opts.dir;
    let script = format!(
        "cd ~/{dir} || exit 1\n\
         mkdir -p target\n\
         rm -f target/release-build.exit\n\
         echo $$ > target/release-build.pid\n\
         JOBS={jobs} bash scripts/release/build-host.sh {args} > target/release-build.log 2>&1\n\
         echo $? > target/release-build.exit\n",
        jobs = opts.jobs,
        args = args.join(" ")
    );
    remote.spawn_detached("build", &script)?;
    let status = format!(
        "cd ~/{dir}/target 2>/dev/null || exit 0\n\
         if [ -f release-build.exit ]; then echo \"exit $(cat release-build.exit)\"; else echo running; fi\n\
         tail -n 1 release-build.log 2>/dev/null | tr -d '\\r' | cut -c1-160\n"
    );
    let file = remote.put_script("status", &status)?;
    let start = Instant::now();
    let mut failures = 0;
    let mut last = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(60));
        let elapsed = start.elapsed();
        let (ok, out) = run_bounded(
            &mut remote.ssh(&remote.bash_command(&file)),
            Duration::from_secs(90),
            true,
        )?;
        if !ok {
            failures += 1;
            println!(
                "[{:>4}m] status poll failed ({failures})",
                elapsed.as_secs() / 60
            );
            if failures >= 15 {
                bail!(
                    "{} unreachable; the build may still be running",
                    remote.host
                );
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
            if code.trim() != "0" {
                println!("build-host.sh exited with {code}; fetching what was produced");
            }
            return Ok(());
        }
        if elapsed > opts.timeout {
            let kill = match remote.os {
                Os::Windows => format!(
                    "$p = Get-Content \"$HOME\\{dir}\\target\\release-build.pid\"; \
                     Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*oino-release-build*' }} | \
                     ForEach-Object {{ taskkill /T /F /PID $_.ProcessId }}; $p"
                ),
                Os::Linux | Os::Macos => {
                    "pkill -TERM -f oino-release-build.sh; pkill -TERM -f build-host.sh".to_owned()
                }
            };
            let _ = run_bounded(&mut remote.ssh(&kill), Duration::from_secs(60), false);
            bail!(
                "build on {} exceeded {} min and was stopped",
                remote.host,
                opts.timeout.as_secs() / 60
            );
        }
    }
}

fn fetch(remote: &Remote, dir: &str) -> Result<Vec<PathBuf>> {
    let art = root().join("target/release-artifacts");
    let staging = root()
        .join("target/tmp")
        .join(format!("{}-fetch", remote.host));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let mut cmd = Command::new("scp");
    cmd.args(["-q", "-r"])
        .args(SSH_OPTS)
        .arg(format!("{}:{dir}/target/release-artifacts", remote.host))
        .arg(format!("{}:{dir}/target/release-build.log", remote.host))
        .arg(&staging);
    let (ok, _) = run_bounded(&mut cmd, Duration::from_mins(60), false)?;
    if !ok {
        println!(
            "warning: scp of the artifacts from {} reported an error",
            remote.host
        );
    }
    std::fs::create_dir_all(art.join("logs"))?;
    let log = staging.join("release-build.log");
    if log.is_file() {
        std::fs::copy(&log, art.join("logs").join(format!("{}.log", remote.host)))?;
    }
    let mut targets = Vec::new();
    let Ok(entries) = std::fs::read_dir(staging.join("release-artifacts")) else {
        bail!("no artifacts fetched from {}", remote.host);
    };
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let to = art.join(entry.file_name());
        if to.exists() {
            std::fs::remove_dir_all(&to)?;
        }
        std::fs::rename(entry.path(), &to)?;
        targets.push(to);
    }
    Ok(targets)
}

fn summarize(dirs: &[PathBuf]) {
    for dir in dirs {
        println!("\n{}", dir.display());
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let size = e.metadata().map_or(0, |m| m.len());
            println!(
                "  {:<60} {:>8} MiB",
                e.file_name().to_string_lossy(),
                mib(size)
            );
        }
        if let Ok(smoke) = std::fs::read_to_string(dir.join("smoke.txt")) {
            for line in smoke.lines() {
                println!("  {line}");
            }
        }
    }
}

pub fn remote_build(args: &[String]) -> Result<()> {
    let opts = parse(args)?;
    if opts.host == "local" {
        let script = root().join("scripts/release/build-host.sh");
        let status = Command::new("bash")
            .current_dir(root())
            .arg(script)
            .args(&opts.script_args)
            .env("JOBS", opts.jobs.to_string())
            .status()?;
        if !status.success() {
            bail!("build-host.sh failed");
        }
        return Ok(());
    }
    let os = match opts.os {
        Some(os) => os,
        None => detect_os(&opts.host)?,
    };
    println!("{}: {os:?}", opts.host);
    let remote = Remote {
        host: opts.host.clone(),
        os,
        bash: std::env::var("OINO_REMOTE_BASH")
            .unwrap_or_else(|_| r"C:\Program Files\Git\bin\bash.exe".into()),
    };
    if opts.sync {
        sync(&remote, &opts.dir)?;
    }
    if opts.build {
        build(&remote, &opts)?;
    }
    let dirs = fetch(&remote, &opts.dir)?;
    summarize(&dirs);
    Ok(())
}
