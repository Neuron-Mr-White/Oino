//! End-to-end: `--exclude-extension` (WP-X2) and the safe-mode banner/reset (X-SAFE) of the
//! `--exclude-extension` end-to-end against the `oino` binary, with the distribution
//! packages staged through `OINO_DIST_EXTENSIONS` (`extensions/dist`, built by
//! `cargo xtask wasm`).
//!
//! Skips (passes without running) when `extensions/dist` is missing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn dist() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/dist")
}

fn faux_package() -> Option<PathBuf> {
    let package = dist().join("oino.fixture.provider-faux");
    let dist_built = dist()
        .join("oino.openai-compat")
        .join("component.wasm")
        .is_file();
    (package.join("component.wasm").is_file() && dist_built).then_some(package)
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("oino-exclude-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        for sub in ["home", "agent/cache", "work"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        // Share the wasmtime compile cache between runs (the first compile is slow in debug).
        let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("oino-wasmtime-cache");
        std::fs::create_dir_all(&cache).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&cache, dir.path().join("agent/cache/wasmtime")).unwrap();
        std::fs::write(
            dir.path().join("agent/auth.json"),
            r#"{"faux":{"type":"api_key","key":"k"}}"#,
        )
        .unwrap();
        Self { dir }
    }

    fn command(&self, package: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_oino"));
        command
            .current_dir(self.dir.path().join("work"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("OINO_AGENT_DIR", self.dir.path().join("agent"))
            // The distribution share dir: the composed source the exclusions act on.
            .env("OINO_DIST_EXTENSIONS", dist())
            .arg("-e")
            .arg(package)
            .args(["--model", "faux/faux-1"])
            .args(args);
        command
    }

    fn run(&self, package: &Path, args: &[&str]) -> Output {
        self.command(package, args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_documents_the_exclusion_flag() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oino"))
        .current_dir(dir.path())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir.path().join("home"))
        .arg("--help")
        .output()
        .unwrap();
    let help = stdout(&output);
    assert!(
        help.contains("--exclude-extension <id>"),
        "the flag line sits next to --no-extensions:\n{help}"
    );
    assert!(help.contains("--no-extensions, -ne"), "{help}");
}

#[test]
fn exclusion_drops_distribution_packages_and_warns_on_misses() {
    let Some(package) = faux_package() else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: run `cargo xtask wasm` to build extensions/dist"
        );
        return;
    };
    let sandbox = Sandbox::new();

    // Spec acceptance: `--exclude-extension oino.openai-compat,oino.anthropic -p "hi"` — neither
    // loads, the log names both, the run still answers (WP-C6).
    let output = sandbox.run(
        &package,
        &[
            "-p",
            "hi",
            "--exclude-extension",
            "oino.openai-compat,oino.anthropic",
            "--exclude-extension",
            "oino.openai-responses",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Hello from faux.\n");
    let err = stderr(&output);
    for id in [
        "oino.openai-compat",
        "oino.anthropic",
        "oino.openai-responses",
    ] {
        assert!(
            err.contains(&format!("Excluded extension \"{id}\" not loaded")),
            "{err}"
        );
    }

    // Spec acceptance: an id matching nothing warns and exits 0.
    let output = sandbox.run(
        &package,
        &["-p", "hi", "--exclude-extension", "does.not-exist"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Hello from faux.\n");
    let err = stderr(&output);
    assert!(
        err.contains("--exclude-extension \"does.not-exist\" matched no extension package"),
        "{err}"
    );
}

#[test]
fn exclusion_beats_explicit_e_packages() {
    let Some(package) = faux_package() else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: run `cargo xtask wasm` to build extensions/dist"
        );
        return;
    };
    let sandbox = Sandbox::new();

    // Excluding the `-e` provider package by id drops it, so no model is available.
    let output = sandbox.run(
        &package,
        &[
            "-p",
            "hi",
            "--exclude-extension",
            package.file_name().unwrap().to_string_lossy().as_ref(),
        ],
    );
    assert_ne!(output.status.code(), Some(0));
    let err = stderr(&output);
    assert!(
        err.contains("Excluded extension \"oino.fixture.provider-faux\" not loaded"),
        "{err}"
    );
}

#[test]
fn no_extensions_keeps_providers() {
    let Some(package) = faux_package() else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: run `cargo xtask wasm` to build extensions/dist"
        );
        return;
    };
    let sandbox = Sandbox::new();

    // `-ne` unchanged: extensions off, providers (the `-e` faux package) on.
    let output = sandbox.run(&package, &["-p", "hi", "-ne"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Hello from faux.\n");
}

#[test]
fn safe_mode_banner_names_suspects_and_resets_on_success() {
    let Some(package) = faux_package() else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: run `cargo xtask wasm` to build extensions/dist"
        );
        return;
    };
    let sandbox = Sandbox::new();
    let state = sandbox.dir.path().join("agent/state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("oino.safe-mode.json"),
        r#"{"v":1,"consecutiveFailures":2,"lastFailures":[{"at":1,"where":"services","extensionId":"acme.notify","error":"init failed: boom"},{"at":2,"where":"session","error":"crash"}]}"#,
    )
    .unwrap();

    // Two recorded failed starts: extensions off, providers on, banner names the suspects.
    let output = sandbox.run(&package, &["-p", "hi"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Hello from faux.\n");
    let err = stderr(&output);
    assert!(
        err.contains(
            "Safe mode: extensions are off after 2 failed starts; suspects: acme.notify (init failed: boom)"
        ),
        "{err}"
    );

    // The successful start reset the counter.
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(state.join("oino.safe-mode.json")).unwrap())
            .unwrap();
    assert_eq!(saved["consecutiveFailures"], 0, "{saved}");
    assert_eq!(saved["lastFailures"].as_array().unwrap().len(), 0);

    // And the next start runs without the banner.
    let output = sandbox.run(&package, &["-p", "hi"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("Safe mode:"),
        "{}",
        stderr(&output)
    );
}
