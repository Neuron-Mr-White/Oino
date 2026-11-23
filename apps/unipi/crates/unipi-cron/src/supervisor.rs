//! Linux per-run owner. It outlives the desktop/host and reaps daemonized tools.
use std::time::Duration;

use crate::{
    exec,
    model::{Job, Run, RunStatus},
    paths::CronPaths,
    store,
};

/// PID reuse protection, persisted with the run (the PID is also its process group).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    /// Linux boot UUID.
    pub boot: String,
    /// Kernel start ticks.
    pub start: String,
    /// This process handles Stop and saves its own result.
    #[serde(default)]
    pub supervisor: bool,
}

/// Read the kernel identity, excluding zombies.
#[must_use]
pub fn identity(pid: u32) -> Option<Identity> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    if matches!(fields.first().copied(), Some("Z" | "X")) {
        return None;
    }
    Some(Identity {
        boot: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()?
            .trim()
            .to_owned(),
        start: fields.get(19)?.to_string(),
        supervisor: false,
    })
}

/// Whether the saved process still has the same boot and start identity.
#[must_use]
pub fn live(run: &Run) -> bool {
    run.pid
        .zip(run.process_identity.as_ref())
        .is_some_and(|(pid, saved)| {
            identity(pid).is_some_and(|now| now.boot == saved.boot && now.start == saved.start)
        })
}

/// Called before host initialization, in a dedicated single-threaded process.
///
/// # Errors
/// A plain supervisor setup, process, or store failure.
#[cfg(target_os = "linux")]
pub fn entry(agent: &std::path::Path, job_id: &str, run_id: &str) -> Result<(), String> {
    run_entry(agent, job_id, run_id).map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
#[allow(
    clippy::too_many_lines,
    reason = "one linear supervisor lifecycle: adopt, drain pipes and record the result"
)]
fn run_entry(
    agent: &std::path::Path,
    job_id: &str,
    run_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    nix::sys::prctl::set_child_subreaper(true)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // Install before spawning: a Stop immediately after adoption must not terminate us.
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())?;
        let paths = CronPaths::new(agent);
        let job = store::load_jobs(&paths)?
            .into_iter()
            .find(|j| j.id == job_id)
            .ok_or("job no longer exists")?;
        let mut run = store::load_run(&paths, job_id, run_id)?.ok_or("run no longer exists")?;
        if run.status != RunStatus::Running {
            return Ok(());
        }
        let pid = std::process::id();
        run.pid = Some(pid);
        run.process_identity = identity(pid).map(|mut identity| {
            identity.supervisor = true;
            identity
        });
        store::save_run(&paths, &run)?;
        let stop = tokio_util::sync::CancellationToken::new();
        let token = stop.clone();
        let signal = tokio::spawn(async move {
            interrupt.recv().await;
            token.cancel();
        });
        let spec = exec::RunSpec {
            prompt: job.prompt,
            cwd: job.cwd,
            model: job.model,
            thinking: job.thinking,
            time_limit_s: job
                .time_limit_s
                .saturating_sub(
                    now_ms().saturating_sub(run.started_at.unwrap_or_else(now_ms)) / 1000,
                )
                .max(1),
            cost_cap_usd: job.cost_cap_usd,
        };
        let outcome = exec::run(&paths, job_id, run_id, &spec, stop, |_| {}).await;
        signal.abort();
        // Orphans (including double-fork + setsid) reparent here, not to init.
        // Repeat after killing parents to collect/reap their newly orphaned children.
        for _ in 0..100 {
            let children = exec::descendant_processes(pid);
            for (child, start) in &children {
                if exec::process_identity(*child).is_some_and(|(_, current)| current == *start) {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw((*child).cast_signed()),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
            }
            loop {
                match nix::sys::wait::waitpid(
                    nix::unistd::Pid::from_raw(-1),
                    Some(nix::sys::wait::WaitPidFlag::WNOHANG),
                ) {
                    Ok(nix::sys::wait::WaitStatus::StillAlive) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            if children.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let started = run.started_at.unwrap_or_else(now_ms);
        exec::finish_run(&mut run, outcome, started, now_ms());
        store::save_run(&paths, &run)?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0).cast_unsigned()
}

/// Follow a supervisor across host restarts. Only signal a verified identity.
#[allow(
    clippy::too_many_lines,
    reason = "one linear poll loop: read the record, signal once, enforce the grace period"
)]
pub async fn watch(
    paths: &CronPaths,
    job: &Job,
    mut run: Run,
    stop: tokio_util::sync::CancellationToken,
) -> Run {
    // The on-disk completion watcher never holds an engine or hub lock.
    let mut signalled = false;
    let mut ending: Option<(std::time::Instant, RunStatus)> = None;
    loop {
        if let Ok(Some(saved)) = store::load_run(paths, &job.id, &run.id) {
            run = saved;
            if run.status.is_finished() {
                return run;
            }
        }
        if !live(&run) {
            // The supervisor saves before exit; re-read after observing its death.
            if let Ok(Some(saved)) = store::load_run(paths, &job.id, &run.id)
                && saved.status.is_finished()
            {
                return saved;
            }
            run.status = ending.map_or(RunStatus::Failed, |(_, status)| status);
            run.ended_at = Some(now_ms());
            run.reason = Some(match run.status {
                RunStatus::Stopped => "Stopped by you".into(),
                RunStatus::Timeout => format!("exceeded the {}s time limit", job.time_limit_s),
                _ => "The scheduled run's process ended without saving a result".into(),
            });
            let _ = store::save_run(paths, &run);
            return run;
        }
        let expired = now_ms().saturating_sub(run.started_at.unwrap_or(run.fired_at))
            >= job.time_limit_s.max(1).saturating_mul(1000);
        if (stop.is_cancelled()
            || (expired
                && !run
                    .process_identity
                    .as_ref()
                    .is_some_and(|id| id.supervisor)))
            && !signalled
            && let Some(pid) = run.pid
        {
            let supervised = run
                .process_identity
                .as_ref()
                .is_some_and(|id| id.supervisor);
            #[cfg(unix)]
            let result = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid.cast_signed()),
                if supervised {
                    nix::sys::signal::Signal::SIGUSR1
                } else {
                    nix::sys::signal::Signal::SIGINT
                },
            );
            #[cfg(not(unix))]
            let result: Result<(), ()> = {
                exec::interrupt_pid(pid);
                Ok(())
            };
            signalled = result.is_ok();
            if signalled {
                ending = Some((
                    std::time::Instant::now(),
                    if stop.is_cancelled() {
                        RunStatus::Stopped
                    } else {
                        RunStatus::Timeout
                    },
                ));
            }
        }
        if let Some((at, status)) = ending {
            let grace = if run
                .process_identity
                .as_ref()
                .is_some_and(|id| id.supervisor)
            {
                10
            } else {
                5
            };
            if at.elapsed() >= Duration::from_secs(grace) {
                if live(&run)
                    && let Some(pid) = run.pid
                {
                    #[cfg(target_os = "linux")]
                    let descendants = exec::descendant_processes(pid);
                    exec::kill_pid(pid);
                    #[cfg(target_os = "linux")]
                    for (child, start) in descendants {
                        if exec::process_identity(child)
                            .is_some_and(|(_, current)| current == start)
                        {
                            let _ = nix::sys::signal::kill(
                                nix::unistd::Pid::from_raw(child.cast_signed()),
                                nix::sys::signal::Signal::SIGKILL,
                            );
                        }
                    }
                }
                run.status = status;
                run.ended_at = Some(now_ms());
                run.reason = Some(if status == RunStatus::Stopped {
                    "Stopped by you".into()
                } else {
                    format!("exceeded the {}s time limit", job.time_limit_s)
                });
                let _ = store::save_run(paths, &run);
                return run;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Launch at the packaged host path. The supervisor owns pipes and result recording.
#[cfg(target_os = "linux")]
pub async fn launch(
    paths: &CronPaths,
    job: &Job,
    run: &Run,
    stop: tokio_util::sync::CancellationToken,
) -> Option<Run> {
    let exe = std::env::current_exe().ok()?;
    if !matches!(exe.file_name()?.to_str()?, "unipi-host" | "unipi-oino-host") {
        return None;
    }
    let Ok(mut child) = tokio::process::Command::new(exe)
        .arg("cron-supervisor")
        .arg(&paths.agent_dir)
        .arg(&job.id)
        .arg(&run.id)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
    else {
        return None;
    };
    // Wait for the supervisor's durable ready record before following it.
    for _ in 0..100 {
        if let Ok(Some(saved)) = store::load_run(paths, &job.id, &run.id)
            && (saved.status.is_finished() || live(&saved))
        {
            let result = watch(paths, job, saved, stop).await;
            let _ = child.wait().await;
            return Some(result);
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let _ = child.kill().await;
    let mut failed = run.clone();
    failed.status = RunStatus::Failed;
    failed.ended_at = Some(now_ms());
    failed.reason = Some("Couldn't start the scheduled run supervisor".into());
    Some(failed)
}
