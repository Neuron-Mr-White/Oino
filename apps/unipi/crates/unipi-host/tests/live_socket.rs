#![cfg(unix)]

use std::io::{Read, Write};
use std::{os::unix::net::UnixStream, time::Duration};

#[test]
fn partial_request_does_not_block_a_second_probe() {
    let dir = tempfile::tempdir().unwrap();
    let paths = unipi_host::data_dir::HostPaths::from_dir(dir.path());
    let _thread = unipi_host::live::start(paths).unwrap();
    unipi_host::liveness::refresh(test_info(dir.path()));
    let mut partial = UnixStream::connect(dir.path().join("live.sock")).unwrap();
    partial.write_all(&[0, 0]).unwrap();
    let start = std::time::Instant::now();
    let mut stream = UnixStream::connect(dir.path().join("live.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let request = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":{"op":"host_info"}});
    stream
        .write_all(&unipi_proto::frame::encode(
            unipi_proto::FrameKind::Control,
            &serde_json::to_vec(&request).unwrap(),
        ))
        .unwrap();
    let mut header = [0; 5];
    stream.read_exact(&mut header).unwrap();
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(partial);
}

fn test_info(dir: &std::path::Path) -> unipi_proto::HostInfoData {
    unipi_proto::HostInfoData {
        version: "test".into(),
        proto: unipi_proto::PROTO_VERSION.into(),
        mode: "manual".into(),
        pid: std::process::id(),
        started_at: 1,
        host_id: "id".into(),
        host_name: "test".into(),
        data_dir: dir.display().to_string(),
        tools: unipi_proto::HostTools::default(),
        devices: 0,
        busy: 0,
        capabilities: vec![unipi_proto::TOPICS_CAPABILITY.into()],
        loop_lag_ms: Some(0),
    }
}

#[test]
fn live_socket_answers_while_the_main_runtime_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let paths = unipi_host::data_dir::HostPaths::from_dir(dir.path());
    let _thread = unipi_host::live::start(paths.clone()).unwrap();
    unipi_host::liveness::refresh(unipi_proto::HostInfoData {
        version: "test".into(),
        proto: unipi_proto::PROTO_VERSION.into(),
        mode: "manual".into(),
        pid: std::process::id(),
        started_at: 1,
        host_id: "id".into(),
        host_name: "test".into(),
        data_dir: dir.path().display().to_string(),
        tools: unipi_proto::HostTools::default(),
        devices: 0,
        busy: 0,
        capabilities: vec![unipi_proto::TOPICS_CAPABILITY.into()],
        loop_lag_ms: Some(0),
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    for _ in 0..2 {
        tokio::spawn(async {
            std::thread::sleep(Duration::from_secs(3));
        });
    }
    std::thread::sleep(Duration::from_millis(100));
    let start = std::time::Instant::now();
    let mut stream = UnixStream::connect(dir.path().join("live.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let request = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":{"op":"host_info"}});
    stream
        .write_all(&unipi_proto::frame::encode(
            unipi_proto::FrameKind::Control,
            &serde_json::to_vec(&request).unwrap(),
        ))
        .unwrap();
    let mut header = [0; 5];
    stream.read_exact(&mut header).unwrap();
    assert_eq!(header[0], 0);
    let len = u32::from_be_bytes(header[1..5].try_into().unwrap()) as usize;
    let mut body = vec![0; len];
    stream.read_exact(&mut body).unwrap();
    assert!(start.elapsed() < Duration::from_millis(200));
}
