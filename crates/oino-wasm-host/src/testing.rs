//! Test harness (feature `testing`): builds fixture components from `extensions/fixtures`
//! and loads them into a [`WasmRuntime`] against
//! [`oino_extension_api::testing::RecordingHostServices`].

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, ComponentSource, ExtensionInstance, ExtensionRuntime, HostServices,
    LoadedPackage, Mode,
};

use crate::{HostConfig, WasmRuntime, WasmRuntimeOptions};

/// Repository root (two levels above this crate).
#[must_use]
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("../.."))
}

static BUILD: OnceLock<Result<(), String>> = OnceLock::new();
static BUILD_LOCK: Mutex<()> = Mutex::new(());

/// Builds every fixture (release, `wasm32-wasip2`) once per process.
///
/// # Errors
///
/// Cargo failed.
pub fn build_fixtures() -> Result<(), String> {
    let _guard = BUILD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    BUILD
        .get_or_init(|| {
            let root = repo_root();
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            // Provider components build separately so feature unification does not give every
            // fixture the `provider-extension` world.
            let groups: [&[&str]; 2] = [
                &["fixture-echo", "fixture-ui-probe", "fixture-async-probe"],
                &["fixture-provider-faux"],
            ];
            for group in groups {
                let mut cmd = Command::new(&cargo);
                cmd.current_dir(root.join("extensions")).args([
                    "build",
                    "--release",
                    "--target",
                    "wasm32-wasip2",
                ]);
                for p in group {
                    cmd.args(["-p", p]);
                }
                for var in [
                    "CARGO_TARGET_DIR",
                    "RUSTFLAGS",
                    "CARGO_ENCODED_RUSTFLAGS",
                    "CARGO_BUILD_TARGET",
                    "CARGO_MANIFEST_DIR",
                ] {
                    cmd.env_remove(var);
                }
                let out = cmd.output().map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(String::from_utf8_lossy(&out.stderr).into_owned());
                }
            }
            Ok(())
        })
        .clone()
}

/// Path of a built fixture component (`echo`, `ui-probe`, `async-probe`, `provider-faux`).
#[must_use]
pub fn fixture_wasm(name: &str) -> PathBuf {
    repo_root()
        .join("extensions/target/wasm32-wasip2/release")
        .join(format!("fixture_{}.wasm", name.replace('-', "_")))
}

/// Manifest of a fixture, optionally with `permissions` replaced.
///
/// # Errors
///
/// Invalid manifest.
pub fn fixture_manifest(
    name: &str,
    permissions: Option<serde_json::Value>,
) -> Result<Manifest, String> {
    let path = repo_root()
        .join("extensions/fixtures")
        .join(name)
        .join("extension.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if let Some(p) = permissions {
        value["permissions"] = p;
    }
    Manifest::parse(&value.to_string()).map_err(|e| e.to_string())
}

/// A loaded fixture plus its environment.
pub struct Harness {
    /// The runtime.
    pub runtime: WasmRuntime,
    /// Recorded host calls.
    pub host: Arc<RecordingHostServices>,
    /// Agent dir (temp).
    pub agent_dir: tempfile::TempDir,
    /// Project dir (temp; the call cwd).
    pub project: tempfile::TempDir,
}

impl Harness {
    /// A runtime over a fresh temp agent dir and project.
    ///
    /// # Errors
    ///
    /// Fixture build or engine setup failed.
    pub fn new() -> Result<Self, String> {
        build_fixtures()?;
        let agent_dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let project = tempfile::tempdir().map_err(|e| e.to_string())?;
        let mut options =
            WasmRuntimeOptions::new(HostConfig::new(agent_dir.path(), project.path()));
        options.cache_dir = Some(repo_root().join("target/oino-wasm-test-cache"));
        let runtime = WasmRuntime::new(options).map_err(|e| e.to_string())?;
        Ok(Self {
            runtime,
            host: RecordingHostServices::new(),
            agent_dir,
            project,
        })
    }

    /// Call context for session `sid`.
    #[must_use]
    pub fn cx(&self, sid: &str, kind: CallKind) -> CallContext {
        CallContext {
            session_id: sid.into(),
            mode: Mode::Rpc,
            has_ui: true,
            cwd: self.project.path().to_path_buf(),
            kind,
            ..CallContext::default()
        }
    }

    /// Loads fixture `name` (optionally with replaced permissions) against `host`.
    ///
    /// # Errors
    ///
    /// Load failure.
    pub async fn load_with(
        &self,
        name: &str,
        permissions: Option<serde_json::Value>,
        host: Arc<dyn HostServices>,
    ) -> Result<Arc<dyn ExtensionInstance>, String> {
        let manifest = fixture_manifest(name, permissions)?;
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(fixture_wasm(name)),
            root: None,
            bundled: false,
        };
        self.runtime
            .load(package, host)
            .await
            .map_err(|e| e.to_string())
    }

    /// Loads fixture `name` against the recorder.
    ///
    /// # Errors
    ///
    /// Load failure.
    pub async fn load(
        &self,
        name: &str,
        permissions: Option<serde_json::Value>,
    ) -> Result<Arc<dyn ExtensionInstance>, String> {
        let host: Arc<dyn HostServices> = Arc::clone(&self.host) as Arc<dyn HostServices>;
        self.load_with(name, permissions, host).await
    }
}
