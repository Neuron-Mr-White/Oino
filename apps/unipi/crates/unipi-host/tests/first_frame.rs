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
        cron: unipi_host::cron_serve::CronHost::new(&paths),
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

/// Captures this thread's tracing output (the tests run on a current-thread runtime).
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
    fn subscriber(&self) -> impl tracing::Subscriber {
        let out = self.clone();
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || out.clone())
            .finish()
    }
}

/// fix5-host: the app's connect-and-close liveness probe (`socket_alive`, every 2 s)
/// is not a rejected operation — no `operation rejected code=E_SCHEMA` WARN, no reply.
#[tokio::test]
async fn a_connect_only_probe_is_not_a_rejected_operation() {
    let log = Captured::default();
    let _guard = tracing::subscriber::set_default(log.subscriber());
    let dir = tempfile::tempdir().unwrap();
    let host = handler(dir.path());
    let (client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(server);
    let (mut client_read, mut client_write) = tokio::io::split(client);
    client_write.shutdown().await.unwrap();
    drop(client_write);
    host.handle(read, write, unipi_host::protocol::Peer::Local)
        .await
        .unwrap();
    let mut rest = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut client_read, &mut rest)
        .await
        .unwrap();
    assert!(rest.is_empty(), "a probe gets no error frame");
    let text = log.text();
    assert!(!text.contains("operation rejected"), "{text}");
    assert!(!text.contains("WARN"), "{text}");
}

/// fix5-host: a real rejection names the op and the reason.
#[tokio::test]
async fn a_rejection_names_the_op_and_the_reason() {
    let log = Captured::default();
    let _guard = tracing::subscriber::set_default(log.subscriber());
    let dir = tempfile::tempdir().unwrap();
    let host = handler(dir.path());
    let (client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(server);
    let (mut client_read, mut client_write) = tokio::io::split(client);
    client_write
        .write_all(&encode(
            FrameKind::Control,
            br#"{"type":"hello","proto":"2.0.0","req":{"op":"no_such_op"}}"#,
        ))
        .await
        .unwrap();
    host.handle(read, write, unipi_host::protocol::Peer::Local)
        .await
        .unwrap();
    let (_, reply) = read_frame(&mut client_read).await.unwrap();
    assert!(matches!(
        serde_json::from_slice::<Control>(&reply).unwrap(),
        Control::Err { error } if error.code == AppErrorCode::E_SCHEMA
    ));
    let text = log.text();
    assert!(
        text.contains("operation rejected")
            && text.contains("op=\"no_such_op\"")
            && text.contains("reason="),
        "{text}"
    );
}
