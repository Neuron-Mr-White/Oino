//! Package manager subcommands (`oino install/list/update/remove/config`) and the
//! self-update check (`oino update --check`) against a temp agent dir.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};
use support::Sandbox;

fn oino(sandbox: &Sandbox, args: &[&str]) -> Output {
    Command::new(support::oino_bin())
        .current_dir(sandbox.cwd())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", sandbox.home())
        .env("OINO_AGENT_DIR", sandbox.agent())
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A minimal package (manifest + component file) at `dir`.
fn package(dir: &Path, version: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("extension.json"),
        json!({
            "apiVersion": "oino.extension/1",
            "id": "test.pkg",
            "name": "Test package",
            "version": version,
            "description": "test",
            "permissions": {"ui": true}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("component.wasm"), b"\0asm").unwrap();
}

#[test]
fn install_list_update_remove_and_config() {
    let sandbox = Sandbox::new();
    let source = sandbox.dir.path().join("src-pkg");
    package(&source, "1.0.0");
    let source_text = source.display().to_string();

    let refused = oino(&sandbox, &["install", &source_text]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        stderr(&refused).contains("Pass --yes"),
        "{}",
        stderr(&refused)
    );

    let installed = oino(&sandbox, &["install", &source_text, "--yes"]);
    assert!(installed.status.success(), "{}", stderr(&installed));
    let out = stdout(&installed);
    assert!(out.contains("Test package 1.0.0 (test.pkg)"), "{out}");
    assert!(out.contains("ui: true"), "{out}");
    let target = sandbox.agent().join("extensions/test.pkg");
    assert!(target.join("extension.json").is_file());
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(target.join(".source.json")).unwrap())
            .unwrap();
    assert_eq!(record["source"], source_text);
    assert_eq!(record["version"], "1.0.0");

    let list = stdout(&oino(&sandbox, &["list"]));
    let line = list.lines().find(|l| l.starts_with("test.pkg")).unwrap();
    assert!(line.contains("1.0.0") && line.contains("global"), "{line}");

    package(&source, "1.1.0");
    let updated = oino(&sandbox, &["update", "test.pkg"]);
    assert!(updated.status.success(), "{}", stderr(&updated));
    assert!(stdout(&updated).contains("Updated test.pkg: 1.0.0 → 1.1.0"));
    let missing = oino(&sandbox, &["update", "nope.pkg"]);
    assert!(stderr(&missing).contains("Not installed: nope.pkg"));

    let config = stdout(&oino(&sandbox, &["config"]));
    assert_eq!(
        config.trim(),
        sandbox.agent().join("settings.json").display().to_string()
    );

    let removed = oino(&sandbox, &["remove", "test.pkg"]);
    assert!(removed.status.success(), "{}", stderr(&removed));
    assert!(!target.exists());
    let again = oino(&sandbox, &["uninstall", "test.pkg"]);
    assert!(stderr(&again).contains("Not installed: test.pkg"));

    let invalid = sandbox.dir.path().join("bad");
    std::fs::create_dir_all(&invalid).unwrap();
    let bad = oino(&sandbox, &["install", &invalid.display().to_string(), "-y"]);
    assert!(
        stderr(&bad).contains("no extension.json"),
        "{}",
        stderr(&bad)
    );
}

#[test]
fn update_check_reads_the_release_manifest() {
    let sandbox = Sandbox::new();
    let manifest = sandbox.dir.path().join("release-manifest.json");
    std::fs::write(
        &manifest,
        json!({"version": "0.0.1", "artifacts": []}).to_string(),
    )
    .unwrap();
    let output = Command::new(support::oino_bin())
        .env(
            "OINO_RELEASE_MANIFEST",
            format!("file://{}", manifest.display()),
        )
        .args(["update", "--check"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("Oino is up to date"));
    std::fs::write(
        &manifest,
        json!({"version": "99.0.0", "artifacts": []}).to_string(),
    )
    .unwrap();
    let output = Command::new(support::oino_bin())
        .env(
            "OINO_RELEASE_MANIFEST",
            format!("file://{}", manifest.display()),
        )
        .args(["update", "--check"])
        .output()
        .unwrap();
    assert!(stdout(&output).contains("→ 99.0.0"), "{}", stdout(&output));
    let output = Command::new(support::oino_bin())
        .env(
            "OINO_RELEASE_MANIFEST",
            format!("file://{}", manifest.display()),
        )
        .args(["update"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("no release binary for"),
        "{}",
        stderr(&output)
    );
}
