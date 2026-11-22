//! Dark worker entry point. No control-process routing is installed until WI-4.
mod serve;

use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[arg(long)]
    pub cwd: PathBuf,
    #[arg(long)]
    pub agent_dir: PathBuf,
    #[arg(long)]
    pub parent_pid: u32,
    /// Inherited Unix socketpair endpoints, one per lane (never filesystem paths).
    #[arg(long)]
    pub ctl_fd: i32,
    #[arg(long)]
    pub data_fd: i32,
    #[arg(long, default_value_t = 15)]
    pub exit_grace_sec: u64,
}

/// Build the capped runtime before any engine or wasm executor exists.
/// # Errors
/// Returns an error for invalid worker configuration or worker startup and transport failures.
pub fn run(args: Args) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::env::var("OINO_WORKERS").as_deref() == Ok("1"),
        "worker entry requires OINO_WORKERS=1"
    );
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .max_blocking_threads(64)
        .enable_all()
        .build()?
        .block_on(serve::run(args))
}
