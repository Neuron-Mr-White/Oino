//! `host-service` (v2 ext-abi §3): discovery ungated, `call` permission-gated per service
//! name (`denied("hostServices:<name>")`), unknown services `not-found`, hub registry
//! before the inner [`HostServices`] host.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;

use oino_extension_api::host_service::HostService;
use oino_extension_api::registry::{CommandSpec, FlagSpec, ShortcutSpec, ToolSpec};
use oino_extension_api::{
    BoxFut, CallContext, CallKind, CancellationToken, EventKind, ExtensionId, ExtensionInstance,
    HostError, HostServices, InitInput, RawJson,
};
use oino_wasm_host::LocalServices;
use oino_wasm_host::testing::Harness;
use serde_json::{Value, json};

/// A host service that answers `add` with `{"via": <marker>, "sum": a+b}`.
struct Marker(&'static str);

impl HostService for Marker {
    fn call(
        &self,
        op: &str,
        payload: RawJson,
        _cancel: CancellationToken,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        let (op, payload) = (op.to_owned(), payload.to_value());
        Box::pin(async move {
            match op.as_str() {
                "add" => Ok(RawJson::from_json_value(&json!({
                    "via": self.0,
                    "sum": payload["a"].as_i64().unwrap_or(0) + payload["b"].as_i64().unwrap_or(0),
                }))),
                other => Err(HostError::Failed(format!("unknown op {other}"))),
            }
        })
    }
}

/// A host service that answers with the caller's session (`call_in`, browser.md §7).
struct WhoCalls;

impl HostService for WhoCalls {
    fn call(
        &self,
        _op: &str,
        _payload: RawJson,
        _cancel: CancellationToken,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        Box::pin(async { Ok(RawJson::from_json_value(&json!({"session": null}))) })
    }

    fn call_in(
        &self,
        cx: &CallContext,
        _op: &str,
        _payload: RawJson,
        _cancel: CancellationToken,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        let session = cx.session_id.clone();
        Box::pin(async move { Ok(RawJson::from_json_value(&json!({"session": session}))) })
    }
}

/// A core stand-in that itself provides the host services `dup` and `inner` (the fallback
/// below the hub registry).
struct InnerHost;

impl HostServices for InnerHost {
    fn subscribe(&self, _ext: &ExtensionId, _events: &[EventKind]) {}
    fn register_tool(&self, _ext: &ExtensionId, _spec: ToolSpec) -> Result<(), HostError> {
        Ok(())
    }
    fn register_command(&self, _ext: &ExtensionId, spec: CommandSpec) -> Result<String, HostError> {
        Ok(spec.name)
    }
    fn register_shortcut(&self, _ext: &ExtensionId, _spec: ShortcutSpec) -> Result<(), HostError> {
        Ok(())
    }
    fn register_flag(&self, _ext: &ExtensionId, _spec: FlagSpec) -> Result<(), HostError> {
        Ok(())
    }
    fn register_service(&self, _ext: &ExtensionId, _name: &str) -> Result<(), HostError> {
        Ok(())
    }
    fn host_services(&self) -> Vec<String> {
        vec!["dup".into(), "inner".into()]
    }
    fn host_service(
        &self,
        _ext: &ExtensionId,
        _cx: &CallContext,
        name: &str,
        op: &str,
        _payload: RawJson,
        _cancel: CancellationToken,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        let (name, op) = (name.to_owned(), op.to_owned());
        Box::pin(async move {
            Ok(RawJson::from_json_value(
                &json!({"via": "inner", "name": name, "op": op}),
            ))
        })
    }
}

/// Loads the echo fixture with `permissions` and inits it.
async fn echo(h: &Harness, permissions: Value) -> Arc<dyn ExtensionInstance> {
    let ext = h.load("echo", Some(permissions)).await.unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    ext
}

/// Runs `probe <name> <arg>` on the echo fixture.
async fn probe(ext: &Arc<dyn ExtensionInstance>, h: &Harness, name: &str, arg: Value) -> Value {
    ext.run_command(
        "probe",
        &format!("{name} {arg}"),
        CancellationToken::new(),
        h.cx("s1", CallKind::Command),
    )
    .await
    .unwrap()
    .unwrap()
    .to_value()
}

#[test]
fn registry_lists_sorted_and_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let hub = LocalServices::new(dir.path());
    hub.register_host_service("b", Arc::new(Marker("b")));
    hub.register_host_service("a", Arc::new(Marker("a")));
    assert_eq!(hub.host_service_names(), ["a", "b"]);
    assert!(hub.host_service("a").is_some());
    assert!(hub.host_service("c").is_none());
    hub.register_host_service("a", Arc::new(Marker("a2")));
    assert_eq!(hub.host_service_names(), ["a", "b"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn call_is_gated_per_name_but_discovery_is_not() {
    let h = Harness::new().unwrap();
    h.runtime
        .local_services()
        .unwrap()
        .register_host_service("calc", Arc::new(Marker("hub")));

    // No `hostServices` grant: discovery still works, calls are denied.
    let ext = echo(&h, json!({})).await;
    assert_eq!(
        probe(&ext, &h, "host-service-available", json!({"name": "calc"})).await,
        json!({"ok": true})
    );
    assert_eq!(
        probe(&ext, &h, "host-service-list", json!(null)).await,
        json!({"ok": ["calc"]})
    );
    let denied = probe(
        &ext,
        &h,
        "host-service",
        json!({"name": "calc", "op": "add", "payload": {"a": 2, "b": 3}}),
    )
    .await;
    assert_eq!(denied["err"], "permission denied: hostServices:calc");

    // Named grant: the call reaches the registered service.
    let ext = echo(&h, json!({"hostServices": ["calc"]})).await;
    assert_eq!(
        probe(
            &ext,
            &h,
            "host-service",
            json!({"name": "calc", "op": "add", "payload": {"a": 2, "b": 3}}),
        )
        .await,
        json!({"ok": {"via": "hub", "sum": 5}})
    );
    // A user-facing failure of the service travels inside the reply (`Failed` → err text).
    let bad = probe(
        &ext,
        &h,
        "host-service",
        json!({"name": "calc", "op": "frob", "payload": {}}),
    )
    .await;
    assert!(bad["err"].as_str().unwrap().contains("unknown op frob"));

    // `"*"` is honoured only for bundled packages (the fixture is not).
    let ext = echo(&h, json!({"hostServices": ["*"]})).await;
    let denied = probe(
        &ext,
        &h,
        "host-service",
        json!({"name": "calc", "op": "add", "payload": {}}),
    )
    .await;
    assert_eq!(denied["err"], "permission denied: hostServices:calc");
}

#[tokio::test(flavor = "multi_thread")]
async fn registry_services_see_the_caller_context() {
    let h = Harness::new().unwrap();
    h.runtime
        .local_services()
        .unwrap()
        .register_host_service("who", Arc::new(WhoCalls));
    let ext = echo(&h, json!({"hostServices": ["who"]})).await;
    assert_eq!(
        probe(
            &ext,
            &h,
            "host-service",
            json!({"name": "who", "op": "x", "payload": {}}),
        )
        .await,
        json!({"ok": {"session": "s1"}})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_service_is_not_found() {
    let h = Harness::new().unwrap();
    // Granted but absent (plain oino registers nothing).
    let ext = echo(&h, json!({"hostServices": ["ghost"]})).await;
    assert_eq!(
        probe(&ext, &h, "host-service-available", json!({"name": "ghost"})).await,
        json!({"ok": false})
    );
    assert_eq!(
        probe(&ext, &h, "host-service-list", json!(null)).await,
        json!({"ok": []})
    );
    let missing = probe(
        &ext,
        &h,
        "host-service",
        json!({"name": "ghost", "op": "add", "payload": {}}),
    )
    .await;
    assert_eq!(missing["err"], "not found: host service ghost");
}

#[tokio::test(flavor = "multi_thread")]
async fn hub_registry_is_served_before_the_inner_host() {
    let h = Harness::new().unwrap();
    // Both the hub registry and the inner host provide `dup`: the registry wins.
    h.runtime
        .local_services()
        .unwrap()
        .register_host_service("dup", Arc::new(Marker("hub")));
    let ext = h
        .load_with(
            "echo",
            Some(json!({"hostServices": ["dup", "inner"]})),
            Arc::new(InnerHost),
        )
        .await
        .unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();

    assert_eq!(
        probe(&ext, &h, "host-service-list", json!(null)).await,
        json!({"ok": ["dup", "inner"]})
    );
    // Registry-first …
    assert_eq!(
        probe(
            &ext,
            &h,
            "host-service",
            json!({"name": "dup", "op": "add", "payload": {"a": 1, "b": 1}}),
        )
        .await,
        json!({"ok": {"via": "hub", "sum": 2}})
    );
    // … then the inner host's own services.
    assert_eq!(
        probe(
            &ext,
            &h,
            "host-service",
            json!({"name": "inner", "op": "ping", "payload": {}}),
        )
        .await,
        json!({"ok": {"via": "inner", "name": "inner", "op": "ping"}})
    );
}
