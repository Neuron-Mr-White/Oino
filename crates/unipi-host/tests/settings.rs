//! v2 (host.md §9.9): `host_settings_get` / `host_settings_set` over a stream.
//!
//! Local ("this computer") may read and write the host settings; a paired
//! phone may read the (masked) object but never write it; unknown dotted keys
//! and wrong-typed values are `E_SCHEMA` and leave `config.json` untouched.
use tokio::io::AsyncWriteExt;
use unipi_host::{
    data_dir::HostPaths,
    devices::DeviceStore,
    protocol::{read_frame, write_control, Handler},
};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, Control, Request,
};

fn handler(paths: HostPaths) -> Handler {
    Handler {
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        paths,
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    }
}

async fn request(host: Handler, remote: iroh::EndpointId, req: Request) -> Control {
    let (client, server) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        host.handle(read, write, remote).await.unwrap();
    });
    let (mut read, mut write) = tokio::io::split(client);
    let hello = Control::Hello {
        proto: unipi_proto::PROTO_VERSION.to_owned().try_into().unwrap(),
        req,
    };
    write_control(&mut write, &hello).await.unwrap();
    let (_, bytes) = read_frame(&mut read).await.unwrap();
    task.await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn local_request(host: Handler, req: Request) -> Control {
    let (client, server) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        host.handle(read, write, unipi_host::protocol::Peer::Local)
            .await
            .unwrap();
    });
    let (mut read, mut write) = tokio::io::split(client);
    let hello = Control::Hello {
        proto: unipi_proto::PROTO_VERSION.to_owned().try_into().unwrap(),
        req,
    };
    write_control(&mut write, &hello).await.unwrap();
    let (_, bytes) = read_frame(&mut read).await.unwrap();
    task.await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Sends a hello built from raw JSON (to exercise schema errors the typed
/// request cannot even represent).
async fn local_raw(host: Handler, hello: serde_json::Value) -> Control {
    let (client, server) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        host.handle(read, write, unipi_host::protocol::Peer::Local)
            .await
            .unwrap();
    });
    let (mut read, mut write) = tokio::io::split(client);
    write
        .write_all(&encode(
            FrameKind::Control,
            &serde_json::to_vec(&hello).unwrap(),
        ))
        .await
        .unwrap();
    let (_, bytes) = read_frame(&mut read).await.unwrap();
    task.await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn data(control: Control) -> serde_json::Value {
    match control {
        Control::Ok { data } => data.unwrap_or(serde_json::Value::Null),
        other => panic!("expected ok, got {other:?}"),
    }
}

fn error_code(control: Control) -> AppErrorCode {
    match control {
        Control::Err { error } => error.code,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test]
async fn local_round_trip_reads_and_writes_the_host_settings() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    paths.ensure_dir().unwrap();

    // Fresh host: an empty object, not an error.
    assert_eq!(
        data(local_request(handler(paths.clone()), Request::HostSettingsGet).await),
        serde_json::json!({})
    );

    // Write one dotted key; the reply is the whole object.
    let reply = data(
        local_request(
            handler(paths.clone()),
            Request::HostSettingsSet(unipi_proto::HostSettingsSetRequest {
                key: "update.mode".to_string().try_into().unwrap(),
                value: serde_json::json!("auto"),
            }),
        )
        .await,
    );
    assert_eq!(reply["update"]["mode"], "auto");

    // Every later request reads the file fresh (no restart).
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
    assert_eq!(written["update"]["mode"], "auto");
    assert_eq!(
        data(local_request(handler(paths.clone()), Request::HostSettingsGet).await)["update"]
            ["mode"],
        "auto"
    );
}

#[tokio::test]
async fn unknown_keys_and_bad_values_are_schema_errors_and_change_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    paths.ensure_dir().unwrap();
    std::fs::write(paths.config(), r#"{"term":{"max":16}}"#).unwrap();

    // Unknown dotted key: rejected while parsing the hello (E_SCHEMA).
    let reply = local_raw(
        handler(paths.clone()),
        serde_json::json!({
            "type": "hello",
            "proto": unipi_proto::PROTO_VERSION,
            "req": {"op": "host_settings_set", "key": "term.nope", "value": 1}
        }),
    )
    .await;
    assert_eq!(error_code(reply), AppErrorCode::E_SCHEMA);

    // A known key with a wrong-typed value is validated before writing.
    let reply = local_request(
        handler(paths.clone()),
        Request::HostSettingsSet(unipi_proto::HostSettingsSetRequest {
            key: "relay".to_string().try_into().unwrap(),
            value: serde_json::json!("http://insecure.example"),
        }),
    )
    .await;
    assert_eq!(error_code(reply), AppErrorCode::E_SCHEMA);

    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
    assert_eq!(raw, serde_json::json!({"term":{"max":16}}));
}

#[tokio::test]
async fn phones_may_read_but_never_write_host_settings() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    paths.ensure_dir().unwrap();
    std::fs::write(
        paths.config(),
        r#"{"forwarding":{"enabled":true,"ports":"any"}}"#,
    )
    .unwrap();
    let devices = DeviceStore::new(paths.devices());
    let phone = iroh::SecretKey::generate().public();
    devices.add(&phone, "Pixel".into()).unwrap();

    let read = request(handler(paths.clone()), phone, Request::HostSettingsGet).await;
    let reply = data(read);
    assert_eq!(reply["forwarding"]["enabled"], true);

    let write = request(
        handler(paths.clone()),
        phone,
        Request::HostSettingsSet(unipi_proto::HostSettingsSetRequest {
            key: "forwarding.enabled".to_string().try_into().unwrap(),
            value: serde_json::json!(false),
        }),
    )
    .await;
    assert_eq!(error_code(write), AppErrorCode::E_UNAUTHORIZED);
    // The owner's switch is untouched.
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
    assert_eq!(raw["forwarding"]["enabled"], true);

    // An unpaired device cannot even read them.
    let stranger = iroh::SecretKey::generate().public();
    let read = request(handler(paths.clone()), stranger, Request::HostSettingsGet).await;
    assert_eq!(error_code(read), AppErrorCode::E_UNAUTHORIZED);
}
