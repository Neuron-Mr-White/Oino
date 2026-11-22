//! Manifest process grants must not bypass the host's execution policy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use oino_extension_api::{CallKind, CancellationToken, InitInput};
use oino_wasm_host::testing::Harness;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn process_imports_deny_before_execution() {
    let harness = Harness::new().unwrap();
    let ext = harness
        .load(
            "echo",
            Some(json!({"process":{"spawn":true,"allow":["sh","cat"]}})),
        )
        .await
        .unwrap();
    ext.init(InitInput::default(), harness.cx("s1", CallKind::Init))
        .await
        .unwrap();
    // RecordingHostServices denies processes unless explicitly allowed.
    for probe in [
        "exec {\"script\":\"printf bypass-ok > marker\"}",
        "spawn {\"input\":\"probe\"}",
    ] {
        let result = ext
            .run_command(
                "probe",
                probe,
                CancellationToken::new(),
                harness.cx("s1", CallKind::Command),
            )
            .await
            .unwrap()
            .unwrap()
            .to_value();
        assert!(result.get("err").is_some(), "{result}");
        assert!(
            result.to_string().contains("needs Autonomy High"),
            "{result}"
        );
        assert!(!harness.project.path().join("marker").exists());
    }
}
