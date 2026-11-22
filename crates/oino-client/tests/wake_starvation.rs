//! Holding the test lock deliberately must not starve unrelated Tokio tasks.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::await_holding_lock,
    reason = "test deliberately holds a guard across await to reproduce starvation"
)]
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hub_state_lock_does_not_starve_tokio_tasks() {
    let lock = std::sync::Arc::new(std::sync::Mutex::new(()));
    let held = lock.lock().unwrap();
    let worker_lock = lock.clone();
    let blocked = tokio::task::spawn_blocking(move || drop(worker_lock.lock().unwrap()));
    tokio::time::sleep(Duration::from_millis(20)).await;
    let start = tokio::time::Instant::now();
    tokio::time::timeout(Duration::from_millis(200), tokio::task::yield_now())
        .await
        .expect("runtime task was starved");
    assert!(start.elapsed() < Duration::from_millis(200));
    drop(held);
    blocked.await.unwrap();
}
