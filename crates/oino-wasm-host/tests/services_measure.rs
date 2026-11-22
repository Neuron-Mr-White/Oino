#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::default_trait_access,
    reason = "ignored diagnostic measurement harness"
)]
//! Opt-in bundled-extension services measurement (isolated temporary state).
#![cfg(feature = "core")]

use std::sync::Arc;
use std::time::Instant;

use oino_core::extensions::api::ExtensionMode;
use oino_core::services::{ResourceFlags, Services, ServicesOptions};
use oino_extension_api::{ComponentSource, LoadedPackage, Manifest};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .map(|s| s.split_whitespace().next().unwrap().parse().unwrap())
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual bundled-component timing and RSS measurement"]
async fn bundled_services_creation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut packages = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(root.join("extensions/dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("extension.json").is_file())
        .collect();
    entries.sort();
    for path in entries {
        // Parse without binary checksum validation: the known kanboard binary checksum
        // mismatch must not omit that component from this measurement.
        let manifest =
            Manifest::parse(&std::fs::read_to_string(path.join("extension.json")).unwrap())
                .unwrap();
        packages.push(SourcedPackage {
            package: LoadedPackage {
                component: ComponentSource::Path(manifest.component_path(&path)),
                manifest,
                root: Some(path),
                bundled: true,
            },
            project: false,
        });
    }
    let temp = tempfile::tempdir().unwrap();
    let agent = temp.path().join("agent");
    let cwd = temp.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let mut options = WasmRuntimeOptions::new(HostConfig::new(&agent, &cwd));
    options.cache_dir = Some(root.join("target/oino-wasm-test-cache"));
    println!(
        "profile phase=before-runtime rss_kib={} smaps={}",
        rss_kib(),
        std::fs::read_to_string("/proc/self/smaps_rollup").unwrap()
    );
    let runtime = Arc::new(WasmRuntime::new(options).unwrap());
    println!(
        "profile phase=after-runtime rss_kib={} smaps={}",
        rss_kib(),
        std::fs::read_to_string("/proc/self/smaps_rollup").unwrap()
    );
    let loader = Arc::new(CoreLoader::new(runtime, packages));
    for iteration in 0..4 {
        let before = rss_kib();
        let start = Instant::now();
        let services = Services::create(ServicesOptions {
            cwd: cwd.clone(),
            agent_dir: agent.clone(),
            docs_root: Default::default(),
            mode: ExtensionMode::Rpc,
            has_ui: false,
            flags: ResourceFlags {
                offline: true,
                no_skills: true,
                no_prompt_templates: true,
                no_themes: true,
                no_context_files: true,
                ..Default::default()
            },
            extension_flag_values: Default::default(),
            loader: loader.clone(),
            approve: Some(true),
            models: None,
            settings: None,
        })
        .await
        .unwrap();
        println!(
            "services iteration={iteration} ms={} rss_before_kib={before} rss_after_kib={} extensions={} diagnostics={:?}",
            start.elapsed().as_millis(),
            rss_kib(),
            services.extensions.len(),
            services.diagnostics
        );
        println!(
            "profile phase=services-{iteration} smaps={}",
            std::fs::read_to_string("/proc/self/smaps_rollup").unwrap()
        );
        for extension in services.extensions.extensions() {
            extension.instance.shutdown("measurement").await;
        }
        drop(services);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        println!(
            "profile phase=shutdown-{iteration} rss_kib={} smaps={}",
            rss_kib(),
            std::fs::read_to_string("/proc/self/smaps_rollup").unwrap()
        );
    }
    assert!(
        loader.errors.lock().unwrap().is_empty(),
        "{:?}",
        loader.errors.lock().unwrap()
    );
}
