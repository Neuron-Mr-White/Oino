#[cfg(unix)]
use crate::data_dir::HostPaths;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
/// The liveness listener thread. Dropping it detaches the thread (tests);
/// [`Live::stop`] ends it and waits so no background thread outlives the host.
pub struct Live {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl Live {
    /// Stops the listener thread and waits for it to leave its accept poll.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(unix)]
/// # Errors
/// Returns an error if the liveness listener or its dedicated thread cannot start.
/// # Panics
/// The spawned listener thread panics if nonblocking setup or runtime initialization fails.
pub fn start(paths: HostPaths) -> std::io::Result<Live> {
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    let path = paths.root.join("live.sock");
    drop(paths);
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!(path = %path.display(), "live socket ready");
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("unipi-liveness".into())
        .spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("nonblocking listener");
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("liveness runtime");
            runtime.block_on(async move {
                let listener = tokio::net::UnixListener::from_std(listener).expect("listener");
                let mut monitor = tokio::time::interval(std::time::Duration::from_secs(1));
                let connections = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
                let monitor_connections = connections.clone();
                tokio::spawn(async move {
                    let mut stalled_at = None;
                    let mut dumped = false;
                    loop {
                        monitor.tick().await;
                        let lag_ms = crate::liveness::loop_lag_ms();
                        if lag_ms > 2_000 {
                            if stalled_at.is_none() {
                                stalled_at = Some(std::time::Instant::now());
                                tracing::warn!(lag_ms, "main runtime stalled");
                            }
                            if !dumped
                                && stalled_at.is_some_and(|at| {
                                    at.elapsed() > std::time::Duration::from_secs(5)
                                })
                            {
                                std::thread::spawn(dump_threads);
                                dumped = true;
                            }
                        } else if let Some(at) = stalled_at.take() {
                            tracing::info!(
                                stalled_ms =
                                    u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX),
                                "main runtime recovered"
                            );
                            dumped = false;
                        }
                    }
                });
                loop {
                    if thread_stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(Ok((stream, _))) = tokio::time::timeout(
                        std::time::Duration::from_millis(200),
                        listener.accept(),
                    )
                    .await
                    else {
                        continue;
                    };
                    if !crate::local::same_user(&stream) {
                        continue;
                    }
                    let Ok(permit) = monitor_connections.clone().try_acquire_owned() else {
                        continue;
                    };
                    tokio::spawn(async move {
                        let _permit = permit;
                        let exchange = live_exchange(stream);
                        let _ =
                            tokio::time::timeout(std::time::Duration::from_secs(2), exchange).await;
                    });
                }
            });
        })?;
    Ok(Live {
        stop,
        thread: Some(thread),
    })
}

#[cfg(unix)]
async fn live_exchange(stream: tokio::net::UnixStream) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = stream;
    let mut header = [0u8; 5];
    if stream.read_exact(&mut header).await.is_err() || header[0] != 0 {
        return;
    }
    let len = u32::from_be_bytes(header[1..5].try_into().unwrap()) as usize;
    if len > unipi_proto::MAX_FRAME {
        return;
    }
    let mut bytes = vec![0; len];
    if stream.read_exact(&mut bytes).await.is_err() {
        return;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    if value["req"]["op"] != "host_info" {
        return;
    }
    let Some(info) = crate::liveness::live_info() else {
        return;
    };
    let reply = unipi_proto::Control::Ok {
        data: serde_json::to_value(info).ok(),
    };
    let Ok(json) = serde_json::to_vec(&reply) else {
        return;
    };
    let frame = unipi_proto::frame::encode(unipi_proto::FrameKind::Control, &json);
    let _ = stream.write_all(&frame).await;
}

#[cfg(unix)]
fn dump_threads() {
    let mut rows = Vec::new();
    let mut message_len = 0usize;
    let mut workers_r = 0;
    let mut workers_s = 0;
    if let Ok(tasks) = std::fs::read_dir("/proc/self/task") {
        for task in tasks.flatten().take(128) {
            let dir = task.path();
            let comm = read_limited(&dir.join("comm")).trim().to_owned();
            let stat = read_limited(&dir.join("stat"));
            let fields = stat
                .rsplit_once(") ")
                .map(|(_, tail)| tail.split_whitespace().collect::<Vec<_>>());
            let state = fields
                .as_ref()
                .and_then(|v| v.first())
                .copied()
                .unwrap_or("?");
            let wchan = read_limited(&dir.join("wchan")).trim().to_owned();
            if comm.starts_with("tokio-rt-worker") {
                match state {
                    "R" => workers_r += 1,
                    "S" => workers_s += 1,
                    _ => {}
                }
            }
            let row = format!(
                "{}:{}:{}:{}",
                task.file_name().to_string_lossy(),
                comm,
                state,
                wchan
            );
            if message_len + row.len() > 8 * 1024 {
                break;
            }
            message_len += row.len();
            rows.push(row);
        }
    }
    tracing::warn!(threads = %rows.join(","), workers_r, workers_s, "main runtime thread dump");
}

#[cfg(unix)]
fn read_limited(path: &std::path::Path) -> String {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut bytes = Vec::with_capacity(512);
    file.take(512).read_to_end(&mut bytes).ok();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(unix)]
pub fn cleanup(paths: &HostPaths) {
    let _ = std::fs::remove_file(paths.root.join("live.sock"));
}
