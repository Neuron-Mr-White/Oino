//! Host services and the host's extension loader (spec-host §4.2; ext-abi §3).
//!
//! The wasm host serves `host-service.call(name, op, json)` from the process-wide
//! [`LocalServices`] registry before anything else (WP-X1), so an embedding host installs its
//! services there once, at startup. Two of the four names in spec-host §4.2 are the host's
//! own and live here:
//!
//! - `"host"` — `info` (host facts) and `shell-env` (read-only environment values).
//! - `"kanboard"` — the `unipi-kanboard` CLI behind the Board screen (ext-abi §5.5): one
//!   process per call, `--actor user`, `--json`; stdout is the CLI's single JSON value,
//!   failures are the CLI's stderr JSON line.
//!
//! `"cron"` and `"browser"` are registered by their owners through
//! [`HostServiceDeps::cron`] / [`HostServiceDeps::browser`] (the X4 cron merge and
//! `spec-browser`'s `BrowserService`); until one is supplied the service is simply absent, so
//! the extension falls back to its documented "no scheduler running"/"unavailable" path.
//!
//! [`HostLoader`] is the loader the hub hands to every project's services: the same
//! discovery as the `oino` binary ([`oino_core::extensions::sources`]: the distribution share
//! dir, `~/.oino/extensions`, trust-gated `<cwd>/.oino/extensions`), `--exclude-extension` and
//! `--no-extensions`/safe mode, on the process-shared wasm engine.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::{ready, BoxFuture};
use oino_core::extensions::api::ExtensionInstance;
use oino_core::extensions::host::HostApi;
use oino_core::extensions::sources::{self, SourcesContext};
use oino_core::services::ExtensionLoader;
use oino_extension_api::host_service::HostService;
use oino_extension_api::{CancellationToken, HostError, RawJson};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::{HostConfig, LocalServices, WasmRuntime, WasmRuntimeOptions};
use serde_json::{json, Value};

/// Service name of the host facts service (ext-abi §3's `"cron"`/`"browser"` siblings).
pub const HOST_SERVICE: &str = "host";
/// Service name of the Board's kanboard CLI wrapper (ext-abi §5.5).
pub const KANBOARD_SERVICE: &str = "kanboard";
/// Service name the cron extension drives (X4; registered by the cron merge).
pub const CRON_SERVICE: &str = "cron";
/// Service name the browser extension drives (`spec-browser` WP-B7).
pub const BROWSER_SERVICE: &str = "browser";

/// How long one `unipi-kanboard` call may take before the host gives up on it.
pub const KANBOARD_TIMEOUT: Duration = Duration::from_secs(30);

/// What [`register_all`] needs to build the host's services.
pub struct HostServiceDeps {
    /// `~/.oino/host` (the data dir `info` reports).
    pub data_dir: PathBuf,
    /// `~/.oino`.
    pub agent_dir: PathBuf,
    /// Host name (OS hostname, the pairing identity's display name).
    pub host_name: String,
    /// Run mode (`app` / `manual`, see [`crate::instance::Mode`]).
    pub mode: String,
    /// The `unipi-kanboard` binary; [`kanboard_bin`] resolves the package/PATH default.
    pub kanboard_bin: Option<PathBuf>,
    /// The embedded scheduler service (X4 cron merge, WP-H8).
    pub cron: Option<Arc<dyn HostService>>,
    /// The browser service (`unipi_browser::BrowserService`, WP-B7).
    pub browser: Option<Arc<dyn HostService>>,
}

/// Registers the host's services into the process-wide wasm host hub and returns the names
/// it registered (for the startup log and the tests).
pub fn register_all(local: &LocalServices, deps: &HostServiceDeps) -> Vec<String> {
    local.register_host_service(
        HOST_SERVICE,
        Arc::new(HostFacts {
            data_dir: deps.data_dir.clone(),
            agent_dir: deps.agent_dir.clone(),
            host_name: deps.host_name.clone(),
            mode: deps.mode.clone(),
        }),
    );
    local.register_host_service(
        KANBOARD_SERVICE,
        Arc::new(KanboardCli {
            bin: deps.kanboard_bin.clone(),
        }),
    );
    let mut names = vec![HOST_SERVICE.to_owned(), KANBOARD_SERVICE.to_owned()];
    if let Some(cron) = &deps.cron {
        local.register_host_service(CRON_SERVICE, Arc::clone(cron));
        names.push(CRON_SERVICE.to_owned());
    }
    if let Some(browser) = &deps.browser {
        local.register_host_service(BROWSER_SERVICE, Arc::clone(browser));
        names.push(BROWSER_SERVICE.to_owned());
    }
    names
}

fn reply(value: Value) -> RawJson {
    RawJson::from_json_value(&value)
}

/// `"host"`: facts and read-only environment values.
struct HostFacts {
    data_dir: PathBuf,
    agent_dir: PathBuf,
    host_name: String,
    mode: String,
}

impl HostService for HostFacts {
    fn call(
        &self,
        op: &str,
        payload: RawJson,
        _cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<RawJson, HostError>> {
        let op = op.to_owned();
        Box::pin(async move {
            let value = match op.as_str() {
                "info" => json!({
                    "ok": true,
                    "version": env!("CARGO_PKG_VERSION"),
                    "proto": unipi_proto::version::PROTO_VERSION,
                    "pid": std::process::id(),
                    "mode": self.mode,
                    "hostName": self.host_name,
                    "dataDir": self.data_dir.display().to_string(),
                    "agentDir": self.agent_dir.display().to_string(),
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                }),
                "shell-env" => shell_env(&payload.to_value()),
                other => json!({
                    "ok": false,
                    "kind": "usage",
                    "error": format!("unknown host op {other:?}"),
                }),
            };
            Ok(reply(value))
        })
    }
}

/// `shell-env`: `{"names": ["PATH", …]}` → `{"vars": {"PATH": "…"}}`; names that are unset
/// stay out of the reply. Read-only — the host never sets or unsets anything.
fn shell_env(payload: &Value) -> Value {
    let Some(names) = payload.get("names").and_then(Value::as_array) else {
        return json!({
            "ok": false,
            "kind": "usage",
            "error": "shell-env needs {\"names\": [\"VAR\", …]}",
        });
    };
    let mut vars = serde_json::Map::new();
    for name in names.iter().filter_map(Value::as_str) {
        // Variable *names* only: no `=`/spaces and no NUL, so this cannot smuggle a command.
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || name.starts_with(|c: char| c.is_ascii_digit())
        {
            return json!({
                "ok": false,
                "kind": "usage",
                "error": format!("not an environment variable name: {name:?}"),
            });
        }
        if let Some(value) = std::env::var_os(name) {
            vars.insert(
                name.to_owned(),
                Value::String(value.to_string_lossy().into_owned()),
            );
        }
    }
    json!({ "ok": true, "vars": vars })
}

/// `"kanboard"`: the `unipi-kanboard` CLI as the app's Board runs it (ext-abi §5.5).
struct KanboardCli {
    bin: Option<PathBuf>,
}

impl HostService for KanboardCli {
    fn call(
        &self,
        op: &str,
        payload: RawJson,
        _cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<RawJson, HostError>> {
        let bin = self.bin.clone();
        let op = op.to_owned();
        let payload = payload.to_value();
        Box::pin(async move { Ok(reply(run_kanboard(bin.as_deref(), &op, &payload).await)) })
    }
}

/// Runs one Board command: `<bin> --actor user <op…> --json [args…]`.
async fn run_kanboard(bin: Option<&Path>, op: &str, payload: &Value) -> Value {
    let Some(bin) = bin else {
        return json!({
            "ok": false,
            "kind": "unavailable",
            "error": "unipi-kanboard is not installed on this host",
        });
    };
    let words: Vec<&str> = op.split_whitespace().collect();
    if words.is_empty()
        || !words.iter().all(|word| {
            word.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
    {
        return json!({
            "ok": false,
            "kind": "usage",
            "error": format!("not a kanboard command path: {op:?}"),
        });
    }
    // The Board acts as the user (only the agent's own calls say `--actor agent`); the
    // extension package ships the binary, the installer puts it in the dist.
    let mut args: Vec<String> = vec!["--actor".to_owned(), "user".to_owned()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    args.push("--json".to_owned());
    if let Some(extra) = payload.get("args") {
        let Some(extra) = extra.as_array() else {
            return json!({
                "ok": false,
                "kind": "usage",
                "error": "args must be an array of strings",
            });
        };
        for arg in extra {
            let Some(arg) = arg.as_str() else {
                return json!({
                    "ok": false,
                    "kind": "usage",
                    "error": "args must be an array of strings",
                });
            };
            args.push(arg.to_owned());
        }
    }
    let child = tokio::process::Command::new(bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(child) => child,
        Err(error) => {
            return json!({
                "ok": false,
                "kind": "io",
                "error": format!("running {}: {error}", bin.display()),
            });
        }
    };
    let output = match tokio::time::timeout(KANBOARD_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return json!({
                "ok": false,
                "kind": "io",
                "error": format!("running {}: {error}", bin.display()),
            });
        }
        Err(_) => {
            return json!({
                "ok": false,
                "kind": "timeout",
                "error": format!(
                    "{} did not finish within {} s",
                    bin.display(),
                    KANBOARD_TIMEOUT.as_secs()
                ),
            });
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        return match serde_json::from_str::<Value>(stdout.trim()) {
            Ok(value) => json!({ "ok": true, "value": value }),
            Err(error) => json!({
                "ok": false,
                "kind": "io",
                "error": format!("{} did not print JSON: {error}", bin.display()),
            }),
        };
    }
    let exit_code = output.status.code();
    // The CLI's failure contract: one JSON line on stderr (`{"ok":false,"error":…,"kind":…}`).
    if let Ok(mut value) = serde_json::from_str::<Value>(stderr.trim()) {
        if value.get("ok").is_none() {
            value["ok"] = Value::Bool(false);
        }
        if let Some(exit_code) = exit_code {
            value["exitCode"] = json!(exit_code);
        }
        return value;
    }
    json!({
        "ok": false,
        "kind": "io",
        "error": stderr.trim().to_owned(),
        "exitCode": exit_code,
    })
}

/// Resolves the `unipi-kanboard` binary: `UNIPI_KANBOARD_BIN`, the distribution package's
/// `bin/`, then `PATH` (the extension ships it, the installer may also put it on PATH).
#[must_use]
pub fn kanboard_bin() -> Option<PathBuf> {
    if let Some(bin) = std::env::var_os("UNIPI_KANBOARD_BIN").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(bin));
    }
    if let Some(share) = sources::dist_share_dir() {
        let name = if cfg!(windows) {
            "unipi-kanboard.exe"
        } else {
            "unipi-kanboard"
        };
        let candidate = share.join("unipi.kanboard").join("bin").join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    on_path("unipi-kanboard").then(|| PathBuf::from("unipi-kanboard"))
}

/// Is `name` an executable on our PATH? (Also resolves `name` itself when it is a path entry.)
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file() || (cfg!(windows) && dir.join(format!("{name}.exe")).is_file())
    })
}

/// The agent dir the host's sessions and extensions live in (`~/.oino`, next to the host's
/// `~/.oino/host`): `OINO_AGENT_DIR`, else the platform agent dir, else the data dir's parent.
///
/// A sandboxed host (tests, a second data dir) sets `OINO_AGENT_DIR`; the host never writes
/// into `~/.unipi` (§5.1).
#[must_use]
pub fn agent_dir(data_dir: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("OINO_AGENT_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(dir);
    }
    oino_settings::paths::agent_dir().unwrap_or_else(|_| {
        data_dir
            .parent()
            .map_or_else(|| data_dir.to_path_buf(), Path::to_path_buf)
    })
}
/// What went wrong while resolving the host's extensions (startup diagnostics).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoaderDiagnostic {
    /// Fatal at startup (an unloadable explicit package).
    pub error: bool,
    /// Text.
    pub message: String,
}

/// The host's extension loader: the `oino` binary's discovery over the process-shared engine.
pub struct HostLoader {
    wasm: Option<CoreLoader>,
    diagnostics: Vec<LoaderDiagnostic>,
}

impl std::fmt::Debug for HostLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostLoader")
            .field("packages", &self.wasm.is_some())
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

impl HostLoader {
    /// Resolves the packages for `cwd` (the same discovery as the `oino` binary) and builds
    /// the runtime on the process-shared engine. `no_extensions` is `-ne`/safe mode
    /// (`kind: "extension"` packages stay out, providers load); `excluded` is
    /// `--exclude-extension`.
    #[must_use]
    pub fn build(agent_dir: &Path, cwd: &Path, no_extensions: bool, excluded: &[String]) -> Self {
        Self::build_filtered(agent_dir, cwd, no_extensions, excluded, false)
    }

    /// Restricted router: load only the normal memory extension, never project extensions.
    pub fn memory_only(agent_dir: &Path) -> Self {
        Self::build_filtered(agent_dir, agent_dir, false, &[], true)
    }

    fn build_filtered(agent_dir: &Path, cwd: &Path, no_extensions: bool, excluded: &[String], memory_only: bool) -> Self {
        // The UniPi reserved-panel table is host registration, not SDK data (§8).
        crate::ui_panels::register();
        let context = SourcesContext {
            agent_dir: agent_dir.to_path_buf(),
            cwd: cwd.to_path_buf(),
            dist_dirs: dist_dirs(),
            no_extensions,
            excluded: excluded.to_vec(),
        };
        let resolved = sources::resolve(&[], &context);
        let mut diagnostics: Vec<LoaderDiagnostic> = resolved
            .diagnostics
            .iter()
            .map(|diagnostic| LoaderDiagnostic {
                error: diagnostic.severity == sources::SourceSeverity::Error,
                message: diagnostic.message.clone(),
            })
            .collect();
        let packages: Vec<SourcedPackage> = resolved
            .packages
            .into_iter()
            .filter(|source| !memory_only || (!source.project && source.package.manifest.id.as_str() == "unipi.memory"))
            .map(|source| SourcedPackage {
                package: source.package,
                project: source.project,
            })
            .collect();
        let wasm = if packages.is_empty() {
            None
        } else {
            let config = HostConfig::new(agent_dir.to_path_buf(), cwd.to_path_buf());
            match WasmRuntime::new(WasmRuntimeOptions::new(config)) {
                Ok(runtime) => Some(CoreLoader::new(Arc::new(runtime), packages)),
                Err(error) => {
                    diagnostics.push(LoaderDiagnostic {
                        error: true,
                        message: format!("Failed to start the extension host: {error}"),
                    });
                    None
                }
            }
        };
        Self { wasm, diagnostics }
    }

    /// Discovery diagnostics (warnings and fatal resolution errors).
    #[must_use]
    pub fn diagnostics(&self) -> &[LoaderDiagnostic] {
        &self.diagnostics
    }

    /// The process-wide wasm host hub the extensions of this host talk to — where
    /// [`register_all`] installs the host services.
    #[must_use]
    pub fn local_services(&self) -> Option<&Arc<LocalServices>> {
        self.wasm
            .as_ref()
            .and_then(|wasm| wasm.runtime().local_services())
    }
}

impl ExtensionLoader for HostLoader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        self.load_enabled(host, project_trusted, &[])
    }

    fn load_enabled<'a>(
        &'a self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
        disabled: &'a [String],
    ) -> BoxFuture<'a, Vec<Arc<dyn ExtensionInstance>>> {
        match &self.wasm {
            Some(wasm) => wasm.load_enabled(host, project_trusted, disabled),
            None => Box::pin(ready(Vec::new())),
        }
    }
}

/// The installed app's bundled extensions, for the layouts the installers
/// produce beyond oino's own exe-relative candidates (`share/oino/extensions`,
/// `extensions`, `bin/extensions`, which the Linux `.deb`/AppImage use):
/// the macOS bundle's `Contents/Resources/extensions` and a Tauri resource dir
/// under `<prefix>/lib/<product>/extensions`. `OINO_DIST_EXTENSIONS` still wins
/// (oino-core reads it first).
#[must_use]
pub fn dist_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(value) = std::env::var_os("OINO_DIST_EXTENSIONS").filter(|v| !v.is_empty()) {
        dirs.push(std::path::PathBuf::from(value));
    }
    let Some(exe) = std::env::current_exe().ok() else {
        return dirs;
    };
    dirs.extend(oino_core::extensions::sources::share_candidates(&exe));
    if let Some(prefix) = exe.parent().and_then(std::path::Path::parent) {
        dirs.push(prefix.join("Resources").join("extensions"));
        if let Ok(entries) = std::fs::read_dir(prefix.join("lib")) {
            let mut found: Vec<_> = entries
                .filter_map(Result::ok)
                .map(|e| e.path().join("extensions"))
                .filter(|d| d.join("unipi.core").is_dir())
                .collect();
            found.sort();
            dirs.extend(found);
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs.dedup();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(dir: &Path) -> HostServiceDeps {
        HostServiceDeps {
            data_dir: dir.join("host"),
            agent_dir: dir.to_path_buf(),
            host_name: "test-host".into(),
            mode: "manual".into(),
            kanboard_bin: None,
            cron: None,
            browser: None,
        }
    }

    async fn call(service: &dyn HostService, op: &str, payload: Value) -> Value {
        let reply = service
            .call(
                op,
                RawJson::from_json_value(&payload),
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| {
                RawJson::from_json_value(&json!({"hostError": error.to_string()}))
            });
        reply.to_value()
    }

    #[tokio::test]
    async fn registers_the_host_and_kanboard_services() {
        let dir = tempfile::tempdir().unwrap();
        let local = LocalServices::new(dir.path());
        let names = register_all(&local, &deps(dir.path()));
        assert_eq!(names, vec!["host".to_owned(), "kanboard".to_owned()]);
        assert_eq!(
            local.host_service_names(),
            vec!["host".to_owned(), "kanboard".to_owned()]
        );
        assert!(local.host_service(HOST_SERVICE).is_some());
        assert!(
            local.host_service(CRON_SERVICE).is_none(),
            "no scheduler yet"
        );
    }

    #[tokio::test]
    async fn host_info_reports_facts_and_shell_env_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let facts = HostFacts {
            data_dir: dir.path().join("host"),
            agent_dir: dir.path().to_path_buf(),
            host_name: "box".into(),
            mode: "app".into(),
        };
        let info = call(&facts, "info", json!({})).await;
        assert_eq!(info["ok"], json!(true));
        assert_eq!(info["hostName"], json!("box"));
        assert_eq!(info["mode"], json!("app"));
        assert_eq!(info["proto"], json!(unipi_proto::version::PROTO_VERSION));
        assert_eq!(info["agentDir"], json!(dir.path().display().to_string()));

        let vars = call(
            &facts,
            "shell-env",
            json!({"names": ["PATH", "OINO_NOT_SET_SURELY"]}),
        )
        .await;
        assert_eq!(vars["ok"], json!(true));
        assert!(vars["vars"].get("PATH").is_some());
        assert!(vars["vars"].get("OINO_NOT_SET_SURELY").is_none());

        let bad = call(&facts, "shell-env", json!({"names": ["PATH=1"]})).await;
        assert_eq!(bad["ok"], json!(false));
        assert_eq!(bad["kind"], json!("usage"));
        let unknown = call(&facts, "frobnicate", json!({})).await;
        assert_eq!(unknown["ok"], json!(false));
        assert!(unknown["error"].as_str().unwrap().contains("frobnicate"));
    }

    #[tokio::test]
    async fn kanboard_without_a_binary_says_it_is_unavailable() {
        let reply = run_kanboard(None, "status", &json!({})).await;
        assert_eq!(reply["ok"], json!(false));
        assert_eq!(reply["kind"], json!("unavailable"));
        let usage = run_kanboard(Some(Path::new("unipi-kanboard")), "Bad Op!", &json!({})).await;
        assert_eq!(usage["kind"], json!("usage"));
        let bad_args = run_kanboard(
            Some(Path::new("unipi-kanboard")),
            "list",
            &json!({"args": [1]}),
        )
        .await;
        assert_eq!(bad_args["kind"], json!("usage"));
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kanboard_runs_one_cli_process_as_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let args_file = dir.path().join("args.txt");
        let bin = script(
            dir.path(),
            "unipi-kanboard",
            &format!(
                "printf '%s\\n' \"$@\" > {}\necho '{{\"projects\": []}}'",
                args_file.display()
            ),
        );
        let reply = run_kanboard(
            Some(&bin),
            "project list",
            &json!({"args": ["--root", "/tmp/project"]}),
        )
        .await;
        assert_eq!(reply["ok"], json!(true));
        assert_eq!(reply["value"], json!({"projects": []}));
        let args = std::fs::read_to_string(&args_file).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            [
                "--actor",
                "user",
                "project",
                "list",
                "--json",
                "--root",
                "/tmp/project"
            ]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kanboard_failures_come_back_as_the_cli_json_line() {
        let dir = tempfile::tempdir().unwrap();
        let bin = script(
            dir.path(),
            "unipi-kanboard",
            "echo '{\"ok\":false,\"error\":\"no such project\",\"kind\":\"not_found\"}' >&2\nexit 1",
        );
        let reply = run_kanboard(Some(&bin), "show", &json!({"args": ["t1"]})).await;
        assert_eq!(reply["ok"], json!(false));
        assert_eq!(reply["kind"], json!("not_found"));
        assert_eq!(reply["error"], json!("no such project"));
        assert_eq!(reply["exitCode"], json!(1));

        let text = script(dir.path(), "plain", "echo boom >&2\nexit 2");
        let reply = run_kanboard(Some(&text), "status", &json!({})).await;
        assert_eq!(reply["kind"], json!("io"));
        assert_eq!(reply["error"], json!("boom"));
    }

    #[test]
    fn the_loader_resolves_an_empty_agent_dir_without_an_engine() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        std::fs::create_dir_all(agent.join("extensions")).unwrap();
        let loader = HostLoader::build(&agent, dir.path(), false, &[]);
        assert!(loader.local_services().is_none(), "no packages, no runtime");
        assert!(loader.diagnostics().is_empty());
        // Resolution never panics whatever this machine has.
        let _ = kanboard_bin();
    }
}
