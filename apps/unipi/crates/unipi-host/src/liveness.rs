use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    RwLock,
};
use std::time::{SystemTime, UNIX_EPOCH};

static LAST_HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);
static BUSY: AtomicU32 = AtomicU32::new(0);
static HOST_INFO: RwLock<Option<unipi_proto::HostInfoData>> = RwLock::new(None);

pub fn start() {
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
        loop {
            interval.tick().await;
            LAST_HEARTBEAT_MS.store(now_ms(), Ordering::Relaxed);
        }
    });
}

pub fn loop_lag_ms() -> u32 {
    let last = LAST_HEARTBEAT_MS.load(Ordering::Relaxed);
    u32::try_from(now_ms().saturating_sub(last)).unwrap_or(u32::MAX)
}

pub fn refresh(info: unipi_proto::HostInfoData) {
    BUSY.store(info.busy, Ordering::Relaxed);
    if let Ok(mut cached) = HOST_INFO.write() {
        *cached = Some(info);
    }
}

pub fn snapshot() -> Option<unipi_proto::HostInfoData> {
    HOST_INFO.read().ok()?.clone()
}

pub fn live_info() -> Option<unipi_proto::HostInfoData> {
    let mut info = snapshot()?;
    info.pid = std::process::id();
    info.busy = BUSY.load(Ordering::Relaxed);
    info.loop_lag_ms = Some(loop_lag_ms());
    Some(info)
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
