//! KB-1: declared contained helpers work across contexts; user commands own writes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(unix)]
use oino_extension_api::{
    CallKind, ComponentSource, EventKind, ExtensionInstance, ExtensionRuntime, HostServices,
    InitInput, LoadedPackage,
};
use oino_wasm_host::testing::{Harness, fixture_manifest, fixture_wasm};
use serde_json::{Value, json};
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    sync::Arc,
};

fn executable(path: &Path) {
    std::fs::write(path, "#!/bin/sh\nprintf helper-ok").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

async fn probe(ext: &dyn ExtensionInstance, h: &Harness, command: &Path, args: &[&str]) -> Value {
    ext.on_event(
        EventKind::TurnEnd,
        json!({"type":"turn_end", "processProbe":{"command":command,"args":args}}).into(),
        h.cx("s1", CallKind::Event),
    )
    .await
    .unwrap()
    .unwrap()
    .to_value()
}

#[tokio::test(flavor = "multi_thread")]
#[allow(
    clippy::too_many_lines,
    reason = "Keep this linear integration scenario and its assertions together."
)]
async fn declared_contained_helpers_and_user_commands_skip_approval() {
    let h = Harness::new().unwrap();
    let pkg = tempfile::tempdir().unwrap();
    std::fs::create_dir(pkg.path().join("bin")).unwrap();
    let helper = pkg.path().join("bin/helper");
    let undeclared = pkg.path().join("bin/undeclared");
    executable(&helper);
    executable(&undeclared);
    let outside = tempfile::tempdir().unwrap();
    executable(&outside.path().join("helper"));
    let mut manifest = fixture_manifest(
        "echo",
        Some(json!({"process":{"spawn":true,"allow":["helper","undeclared"]}})),
    )
    .unwrap();
    manifest.binaries = vec!["helper".into()];
    manifest.helpers.insert(
        "helper".into(),
        vec![vec!["status".into(), "--json".into()]],
    );
    let host: Arc<dyn HostServices> = h.host.clone();
    let ext = h
        .runtime
        .load(
            LoadedPackage {
                manifest,
                component: ComponentSource::Path(fixture_wasm("echo")),
                root: Some(pkg.path().into()),
                bundled: false,
            },
            host,
        )
        .await
        .unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    assert_eq!(
        probe(&*ext, &h, &helper, &["status", "--json"]).await["ok"]["stdout"],
        "helper-ok"
    );
    for (path, args) in [
        (&undeclared, vec!["status", "--json"]),
        (&helper, vec!["delete"]),
        (&outside.path().join("helper"), vec!["status", "--json"]),
    ] {
        assert!(
            probe(&*ext, &h, path, &args).await["err"]
                .as_str()
                .unwrap()
                .contains("needs Autonomy High")
        );
    }
    // A user's explicit command may invoke its own helper, including writes.
    let out = ext
        .run_command(
            "probe",
            &format!(
                "exec-program {}",
                json!({"command":helper,"args":["status","--json"]})
            ),
            tokio_util::sync::CancellationToken::default(),
            h.cx("s1", CallKind::Command),
        )
        .await
        .unwrap()
        .unwrap()
        .to_value();
    assert_eq!(out["ok"]["stdout"], "helper-ok", "{out}");
    for kind in [CallKind::Service, CallKind::Tool] {
        assert_eq!(
            ext.on_event(
                EventKind::TurnEnd,
                json!({"processProbe":{"command":helper,"args":["status","--json"]}}).into(),
                h.cx("s1", kind)
            )
            .await
            .unwrap()
            .unwrap()
            .to_value()["ok"]["stdout"],
            "helper-ok"
        );
        let denied = ext
            .on_event(
                EventKind::TurnEnd,
                json!({"processProbe":{"command":helper,"args":["delete"]}}).into(),
                h.cx("s1", kind),
            )
            .await
            .unwrap()
            .unwrap()
            .to_value();
        assert!(
            denied["err"]
                .as_str()
                .unwrap()
                .contains("needs Autonomy High")
        );
    }
    let write = ext
        .run_command(
            "probe",
            &format!(
                "exec-program {}",
                json!({"command":helper,"args":["delete"]})
            ),
            tokio_util::sync::CancellationToken::default(),
            h.cx("s1", CallKind::Command),
        )
        .await
        .unwrap()
        .unwrap()
        .to_value();
    assert_eq!(write["ok"]["stdout"], "helper-ok");
    let service = ext
        .handle_service(
            "echo.process",
            json!({"command":helper,"args":["status","--json"]}).into(),
            &harness_caller(),
            h.cx("s1", CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(service["ok"]["stdout"], "helper-ok");
    // Explicit commands still cannot execute undeclared or external binaries.
    for path in [&undeclared, &outside.path().join("helper")] {
        let out = ext
            .run_command(
                "probe",
                &format!(
                    "exec-program {}",
                    json!({"command":path,"args":["status","--json"]})
                ),
                tokio_util::sync::CancellationToken::default(),
                h.cx("s1", CallKind::Command),
            )
            .await
            .unwrap()
            .unwrap()
            .to_value();
        assert!(out["err"].as_str().unwrap().contains("needs Autonomy High"));
    }
    // An extension cannot borrow another installed package's helper declaration.
    let mut other_manifest = fixture_manifest(
        "echo",
        Some(json!({"process":{"spawn":true,"allow":["helper"]}})),
    )
    .unwrap();
    other_manifest.id = oino_extension_api::ExtensionId::new("other.extension").unwrap();
    other_manifest.binaries = vec!["helper".into()];
    other_manifest.helpers.insert(
        "helper".into(),
        vec![vec!["status".into(), "--json".into()]],
    );
    let host: Arc<dyn HostServices> = h.host.clone();
    let other = h
        .runtime
        .load(
            LoadedPackage {
                manifest: other_manifest,
                component: ComponentSource::Path(fixture_wasm("echo")),
                root: Some(outside.path().into()),
                bundled: false,
            },
            host,
        )
        .await
        .unwrap();
    other
        .init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    assert!(
        probe(&*other, &h, &helper, &["status", "--json"]).await["err"]
            .as_str()
            .unwrap()
            .contains("needs Autonomy High")
    );
    std::fs::remove_file(&helper).unwrap();
    symlink(outside.path().join("helper"), &helper).unwrap();
    assert!(
        probe(&*ext, &h, &helper, &["status", "--json"]).await["err"]
            .as_str()
            .unwrap()
            .contains("needs Autonomy High")
    );
    let escaped = ext
        .run_command(
            "probe",
            &format!(
                "exec-program {}",
                json!({"command":helper,"args":["delete"]})
            ),
            tokio_util::sync::CancellationToken::default(),
            h.cx("s1", CallKind::Command),
        )
        .await
        .unwrap()
        .unwrap()
        .to_value();
    assert!(
        escaped["err"]
            .as_str()
            .unwrap()
            .contains("needs Autonomy High")
    );
}

fn harness_caller() -> oino_extension_api::ExtensionId {
    oino_extension_api::ExtensionId::new("test.caller").unwrap()
}
