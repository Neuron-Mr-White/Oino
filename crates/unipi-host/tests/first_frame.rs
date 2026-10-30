use tokio::io::AsyncWriteExt;
use unipi_host::{
    data_dir::HostPaths,
    protocol::{read_frame, Handler},
};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, Control,
};
fn handler(dir: &std::path::Path) -> Handler {
    let paths = HostPaths::from_dir(dir);
    Handler {
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        paths,
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    }
}
async fn rejected(bytes: Vec<u8>, expected: AppErrorCode) {
    let dir = tempfile::tempdir().unwrap();
    let host = handler(dir.path());
    let (client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        host.handle(read, write, iroh::SecretKey::generate().public())
            .await
            .unwrap();
    });
    let (mut read, mut write) = tokio::io::split(client);
    write.write_all(&bytes).await.unwrap();
    let (_, reply) = read_frame(&mut read).await.unwrap();
    match serde_json::from_slice::<Control>(&reply).unwrap() {
        Control::Err { error } => assert_eq!(error.code, expected),
        _ => panic!("expected error"),
    };
    task.await.unwrap();
}
#[tokio::test]
async fn wrong_kind() {
    rejected(encode(FrameKind::Input, b"{}"), AppErrorCode::E_SCHEMA).await;
}
#[tokio::test]
async fn missing_proto() {
    rejected(
        encode(
            FrameKind::Control,
            br#"{"type":"hello","req":{"op":"list","mux":"herdr"}}"#,
        ),
        AppErrorCode::E_PROTO_VERSION,
    )
    .await;
}
#[tokio::test]
async fn garbage() {
    rejected(
        encode(FrameKind::Control, b"garbage"),
        AppErrorCode::E_SCHEMA,
    )
    .await;
}
#[tokio::test(start_paused = true)]
async fn slow_first_frame() {
    let dir = tempfile::tempdir().unwrap();
    let host = handler(dir.path());
    let (mut client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        host.handle(read, write, iroh::SecretKey::generate().public())
            .await
            .unwrap();
    });
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    let (_, reply) = read_frame(&mut client).await.unwrap();
    assert!(
        matches!(serde_json::from_slice::<Control>(&reply).unwrap(),Control::Err{error} if error.code==AppErrorCode::E_SCHEMA)
    );
    task.await.unwrap();
}
