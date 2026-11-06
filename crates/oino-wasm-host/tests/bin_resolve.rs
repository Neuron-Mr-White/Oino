//! Process-service program resolution (ext-abi §5.2): `OINO_EXT_BIN_DIR` → the
//! calling extension's package `bin/` (declared in `binaries`) → `PATH`.
//! Absolute and relative paths pass through unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unsafe_code,
    reason = "tests assert by unwrapping and set process env before any host thread reads it"
)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oino_extension_api::{
    CallKind, CancellationToken, ComponentSource, ExtensionInstance, ExtensionRuntime,
    HostServices, InitInput, LoadedPackage,
};
use oino_wasm_host::testing::{Harness, fixture_wasm};
use serde_json::{Value, json};

/// One `bin/<name>` shell script that prints `marker`.
fn script(dir: &Path, name: &str, marker: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\necho {marker}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Loads the echo fixture with `binaries`, `process.allow` and package `root` overridden.
async fn loaded(
    h: &Harness,
    binaries: &[&str],
    allow: &[&str],
    root: Option<PathBuf>,
) -> Arc<dyn ExtensionInstance> {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/fixtures/echo/extension.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["permissions"]["process"] = json!({"spawn": true, "allow": allow});
    manifest["binaries"] = json!(binaries);
    let package = LoadedPackage {
        manifest: oino_extension_api::manifest::Manifest::parse(&manifest.to_string()).unwrap(),
        component: ComponentSource::Path(fixture_wasm("echo")),
        root,
        bundled: false,
    };
    // Decision E: manifest capability is independent of explicit host consent.
    for command in allow {
        h.host.allow_process_command(command);
    }
    let host: Arc<dyn HostServices> = Arc::clone(&h.host) as Arc<dyn HostServices>;
    let ext = h.runtime.load(package, host).await.unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    ext
}

/// Runs the fixture's `exec-program` probe: `process::exec(command)`.
async fn exec_program(ext: &dyn ExtensionInstance, h: &Harness, command: &str) -> Value {
    ext.run_command(
        "probe",
        &format!("exec-program {{\"command\":{command:?}}}"),
        CancellationToken::new(),
        h.cx("s1", CallKind::Command),
    )
    .await
    .unwrap()
    .unwrap()
    .to_value()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn package_bin_resolves_declared_binaries_before_path() {
    let h = Harness::new().unwrap();
    let pkg = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(pkg.path().join("bin")).unwrap();
    script(&pkg.path().join("bin"), "kb-probe-bin", "package-bin");
    let ext = loaded(
        &h,
        &["kb-probe-bin"],
        &["kb-probe-bin"],
        Some(pkg.path().into()),
    )
    .await;
    h.host
        .allow_process_command(&pkg.path().join("bin/kb-probe-bin").to_string_lossy());
    let out = exec_program(&*ext, &h, "kb-probe-bin").await;
    assert_eq!(out["ok"]["stdout"].as_str().unwrap().trim(), "package-bin");
    assert_eq!(out["ok"]["code"], 0);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn undeclared_names_skip_the_package_bin() {
    let h = Harness::new().unwrap();
    let pkg = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(pkg.path().join("bin")).unwrap();
    script(&pkg.path().join("bin"), "kb-skip-bin", "package-bin");
    // Same package content, but `binaries` does not declare the name: the file
    // must not be picked up (a package cannot smuggle arbitrary programs in).
    let ext = loaded(&h, &[], &["kb-skip-bin"], Some(pkg.path().into())).await;
    let out = exec_program(&*ext, &h, "kb-skip-bin").await;
    let err = out["err"].as_str().unwrap_or_default();
    assert!(err.contains("spawn"), "unexpected probe reply: {out}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ext_bin_dir_overrides_the_package_and_searches_in_order() {
    let h = Harness::new().unwrap();
    let pkg = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(pkg.path().join("bin")).unwrap();
    script(&pkg.path().join("bin"), "kb-override-bin", "package-bin");
    let dev = tempfile::tempdir().unwrap();
    script(dev.path(), "kb-override-bin", "dev-build");
    // SAFETY: unique names; sibling tests never resolve these binaries.
    unsafe {
        std::env::set_var("OINO_EXT_BIN_DIR", dev.path());
    }
    let ext = loaded(
        &h,
        &["kb-override-bin"],
        &["kb-override-bin", "kb-order-bin"],
        Some(pkg.path().into()),
    )
    .await;
    h.host
        .allow_process_command(&dev.path().join("kb-override-bin").to_string_lossy());
    let out = exec_program(&*ext, &h, "kb-override-bin").await;
    assert_eq!(
        out["ok"]["stdout"].as_str().unwrap_or_default().trim(),
        "dev-build"
    );

    // A path list: entries are searched in order, first hit wins.
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    script(second.path(), "kb-order-bin", "second");
    let list = format!("{}:{}", first.path().display(), second.path().display());
    // SAFETY: see above.
    unsafe {
        std::env::set_var("OINO_EXT_BIN_DIR", list);
    }
    h.host
        .allow_process_command(&second.path().join("kb-order-bin").to_string_lossy());
    let out = exec_program(&*ext, &h, "kb-order-bin").await;
    // SAFETY: restore before any assertion can early-return.
    unsafe {
        std::env::remove_var("OINO_EXT_BIN_DIR");
    }
    assert_eq!(
        out["ok"]["stdout"].as_str().unwrap_or_default().trim(),
        "second",
        "probe reply: {out}"
    );
}

/// `OINO_EXT_BIN_DIR` also accepts a path list (see
/// [`ext_bin_dir_overrides_the_package_and_searches_in_order`]).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn absolute_paths_pass_through_unchanged() {
    let h = Harness::new().unwrap();
    let pkg = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(pkg.path().join("bin")).unwrap();
    let path = script(&pkg.path().join("bin"), "kb-abs-bin", "absolute");
    // No package root and no declaration: only the path itself can answer.
    let ext = loaded(&h, &[], &["kb-abs-bin"], None).await;
    h.host.allow_process_command(&path.to_string_lossy());
    let out = exec_program(&*ext, &h, &path.to_string_lossy()).await;
    assert_eq!(out["ok"]["stdout"].as_str().unwrap().trim(), "absolute");
}
