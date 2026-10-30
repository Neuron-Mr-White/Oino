use unipi_host::{
    data_dir::HostPaths,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, write_control, Handler},
};
use unipi_proto::{Control, Request};
async fn pair(host: Handler, code: String) -> Control {
    let (client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        host.handle(read, write, iroh::SecretKey::generate().public())
            .await
            .unwrap();
    });
    let (mut read, mut write) = tokio::io::split(client);
    let hello = Control::Hello {
        proto: unipi_proto::PROTO_VERSION.to_owned().try_into().unwrap(),
        req: Request::Pair {
            code: code.try_into().unwrap(),
            name: None,
        },
    };
    write_control(&mut write, &hello).await.unwrap();
    let (_, bytes) = read_frame(&mut read).await.unwrap();
    task.await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
fn handler(paths: HostPaths) -> Handler {
    Handler {
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        paths,
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    }
}
#[tokio::test]
async fn concurrent_code_is_single_use() {
    let dir = tempfile::tempdir().unwrap();
    let host = handler(HostPaths::from_dir(dir.path()));
    let code = PairingStore::new(host.paths.pairing())
        .start(now_unix())
        .unwrap()
        .code;
    let (a, b) = tokio::join!(pair(host.clone(), code.clone()), pair(host, code));
    assert_eq!(
        [a, b]
            .iter()
            .filter(|reply| matches!(reply, Control::Ok { .. }))
            .count(),
        1
    );
}
#[tokio::test]
async fn failed_device_save_restores_code() {
    let dir = tempfile::tempdir().unwrap();
    let host = handler(HostPaths::from_dir(dir.path()));
    let store = PairingStore::new(host.paths.pairing());
    let code = store.start(now_unix()).unwrap().code;
    std::fs::create_dir(host.paths.devices()).unwrap();
    assert!(matches!(
        pair(host.clone(), code.clone()).await,
        Control::Err { .. }
    ));
    assert!(!store.current().unwrap().unwrap().used);
    std::fs::remove_dir(host.paths.devices()).unwrap();
    assert!(matches!(pair(host, code).await, Control::Ok { .. }));
}
