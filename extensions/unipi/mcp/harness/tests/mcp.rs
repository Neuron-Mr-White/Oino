//! Isolated real WASM fixture mounts. No owner state or remote MCP traffic.
use oino_extension_api::{
    CallContext, CallKind, ComponentSource, ExtensionId, ExtensionInstance, ExtensionRuntime,
    HostServices, InitInput, LoadedPackage, Mode, RawJson,
};
use oino_extension_api::{manifest::Manifest, testing::RecordingHostServices};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

struct Fixture {
    _dir: tempfile::TempDir,
    agent: PathBuf,
    project: PathBuf,
    runtime: WasmRuntime,
    host: Arc<RecordingHostServices>,
}
fn put(path: &Path, v: Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
}
impl Fixture {
    fn new() -> Self {
        let base =
            PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/oino-work/mcp-wi2/harness");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::tempdir_in(base).unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(&agent, &project));
        options.cache_dir = Some(dir.path().join("compiled"));
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: RecordingHostServices::new(),
            _dir: dir,
            agent,
            project,
        }
    }
    fn cx(&self) -> CallContext {
        CallContext {
            session_id: "mcp-fixture".into(),
            mode: Mode::Rpc,
            has_ui: false,
            cwd: self.project.clone(),
            kind: CallKind::Service,
            ..CallContext::default()
        }
    }
    async fn load(&self) -> Arc<dyn ExtensionInstance> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let manifest = Manifest::parse(
            &std::fs::read_to_string(root.join("extensions/unipi/mcp/extension.json")).unwrap(),
        )
        .unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(root.join("target/wasm32-wasip2/release/unipi_mcp.wasm")),
            root: None,
            bundled: true,
        };
        let ext = self
            .runtime
            .load(package, self.host.clone() as Arc<dyn HostServices>)
            .await
            .unwrap();
        let reg = ext
            .init(InitInput::default(), self.cx().with_kind(CallKind::Init))
            .await
            .unwrap();
        assert!(
            !reg.commands
                .iter()
                .find(|c| c.name == "mcp-reload")
                .unwrap()
                .hidden
        );
        ext
    }
    async fn service(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, p: Value) -> Value {
        ext.handle_service(
            name,
            RawJson::from(p),
            &ExtensionId::new("fixture").unwrap(),
            self.cx(),
        )
        .await
        .unwrap()
        .to_value()
    }
}
fn row<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_mounts_discovery_partial_save_remove_and_parse() {
    let f = Fixture::new();
    put(
        &f.agent.join("mcp.json"),
        json!({"mcpServers": {"global": {"command": "unused", "enabled": false}, "same": {"command": "g", "enabled": false}}}),
    );
    for (file, name, command) in [
        (".mcp.json", "root", "root"),
        (".unipi/config/mcp/mcp-config.json", "legacy", "legacy"),
        (".pi/mcp.json", "pi", "pi"),
        (".oino/mcp.json", "oino", "oino"),
    ] {
        put(
            &f.project.join(file),
            json!({"mcpServers": {name: {"command": command, "enabled": false}, "same": {"command": command, "enabled": false}}}),
        );
    }
    let ext = f.load().await;
    let v = f
        .service(&ext, "mcp.servers", json!({"cwd": f.project}))
        .await;
    assert_eq!(v["servers"].as_array().unwrap().len(), 6);
    assert_eq!(row(&v, "same")["sourcePath"], f.project.join(".oino/mcp.json").to_string_lossy().as_ref());
    assert_eq!(row(&v, "pi")["sourcePath"], f.project.join(".pi/mcp.json").to_string_lossy().as_ref());
    assert_eq!(
        row(&v, "legacy")["sourcePath"],
        f.project.join(".unipi/config/mcp/mcp-config.json").to_string_lossy().as_ref()
    );
    let original = std::fs::read(f.project.join(".pi/mcp.json")).unwrap();
    let r = f
        .service(
            &ext,
            "mcp.set-enabled",
            json!({"scope":"project", "cwd":f.project,"name":"global","enabled":false}),
        )
        .await;
    assert_eq!(r["ok"], true);
    let doc: Value =
        serde_json::from_slice(&std::fs::read(f.project.join(".oino/mcp.json")).unwrap()).unwrap();
    assert_eq!(doc["mcpServers"]["global"], json!({"enabled":false}));
    assert_eq!(row(&r["servers"], "global")["source"], "project-override");
    assert_eq!(
        original,
        std::fs::read(f.project.join(".pi/mcp.json")).unwrap()
    );
    let r = f
        .service(
            &ext,
            "mcp.save",
            json!({"scope":"global","entries":{"added":{"command":"unused","enabled":false}}}),
        )
        .await;
    assert_eq!(r["ok"], true);
    assert!(
        std::fs::read_to_string(f.agent.join("mcp.json"))
            .unwrap()
            .contains("added")
    );
    assert_eq!(
        f.service(&ext, "mcp.remove", json!({"scope":"global","name":"added"}))
            .await["ok"],
        true
    );
    std::fs::write(f.project.join(".oino/mcp.json"), "{\n invalid").unwrap();
    f.service(&ext, "mcp.server-action", json!({"action":"reload"}))
        .await;
    let v = f.service(&ext, "mcp.servers", json!({})).await;
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("parse error at line 2"))
    );
    assert_eq!(row(&v, "same")["sourcePath"], f.project.join(".pi/mcp.json").to_string_lossy().as_ref());
    assert_eq!(
        f.service(&ext, "mcp.save", json!({"scope":"project","entries":{}}))
            .await["ok"],
        false
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_migration_secrets_and_unresolved_no_spawn() {
    let f = Fixture::new();
    let old = f.agent.join("state/unipi.mcp/global");
    put(
        &old.join("mcp.json"),
        json!({"mcpServers":{"migrated":{"command":"unused"}}}),
    );
    put(
        &old.join("config.json"),
        json!({"servers":{"migrated":{"enabled":false}}}),
    );
    let ext = f.load().await;
    let v = f.service(&ext, "mcp.servers", json!({})).await;
    assert_eq!(row(&v, "migrated")["enabled"], false);
    assert!(old.join("mcp.json.migrated").exists());
    assert!(old.join("config.json.migrated").exists());
    let r = f.service(&ext,"mcp.save",json!({"scope":"project","entries":{"secret":{"command":"unused","enabled":false,"env":{"API_KEY":"fixture-sensitive"}}}})).await;
    assert_eq!(r["ok"], true);
    assert_eq!(r["secretsMoved"], 1);
    assert!(!r.to_string().contains("fixture-sensitive"));
    let text = std::fs::read_to_string(f.project.join(".oino/mcp.json")).unwrap();
    assert!(text.contains("PROJECT_") && text.contains("_SECRET_API_KEY}"));
    assert!(!text.contains("fixture-sensitive"));
    let r = f.service(&ext,"mcp.save",json!({"scope":"project","entries":{"missing":{"command":"never-spawn-this","env":{"TOKEN":"${UNSET_FIXTURE_TOKEN}"}}}})).await;
    assert_eq!(r["ok"], true);
    assert!(
        row(&r["servers"], "missing")["error"]
            .as_str()
            .unwrap()
            .contains("UNSET_FIXTURE_TOKEN is not set")
    );
    assert!(f.host.calls_named("process_spawn").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn tiny_local_stdio_connect_call_and_redacted_error() {
    let f = Fixture::new();
    let script = f.project.join("server.py");
    std::fs::write(&script, r#"import sys, json, os
for line in sys.stdin:
    m = json.loads(line)
    if 'id' not in m: continue
    method = m['method']
    if os.getenv('FAIL'):
        print(os.getenv('API_KEY'), file=sys.stderr, flush=True)
        print(json.dumps({'jsonrpc':'2.0','id':m['id'],'error':{'code':-32000,'message':os.getenv('API_KEY')}}), flush=True)
        continue
    if method == 'initialize':
        result = {'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'ping','description':'fixture ping','inputSchema':{'type':'object','properties':{}}}]}
    elif method == 'tools/call':
        result = {'content':[{'type':'text','text':'WI2_OK'}]}
    else: result = {}
    print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':result}), flush=True)
"#).unwrap();
    f.host.allow_process_command("/usr/bin/python3");
    put(&f.agent.join("mcp.json"),json!({"mcpServers":{}}));
    let ext = f.load().await;
    let r = f.service(&ext,"mcp.save",json!({"scope":"project","entries":{"probe":{"command":"/usr/bin/python3","args":[script]}}})).await;
    assert_eq!(r["ok"],true);
    assert_eq!(row(&r["servers"],"probe")["toolCount"],1, "{r}");
    let result = ext.run_tool(oino_extension_api::ToolInvocation {
        name: "probe__ping".into(), tool_call_id: "ping-1".into(), args: RawJson::from(json!({})),
        parent_tool_call_id: None
    }, oino_extension_api::CancellationToken::new(), f.cx().with_kind(CallKind::Tool)).await.unwrap();
    assert!(format!("{result:?}").contains("WI2_OK"));
    let r = f.service(&ext,"mcp.save",json!({"scope":"project","entries":{"bad":{"command":"/usr/bin/python3","args":[script],"env":{"FAIL":"1","API_KEY":"fixture-never-print"}}}})).await;
    assert!(!r.to_string().contains("fixture-never-print"));
    assert!(row(&r["servers"],"bad")["error"].as_str().unwrap().contains("[REDACTED]"), "{r}");
    // Stop only children spawned by this fixture through the extension lifecycle.
    ext.on_event(oino_extension_api::EventKind::SessionShutdown, RawJson::from(json!({})), f.cx().with_kind(CallKind::Event)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_seed_and_5000_rows_fit_panel() {
    let f = Fixture::new();
    let ext = f.load().await;
    let mut cx = f.cx().with_kind(CallKind::Command);
    cx.has_ui = true;
    ext.run_command("mcp-add", "", oino_extension_api::CancellationToken::new(), cx.clone()).await.unwrap();
    let cache = f.agent.join("state/unipi.mcp/global/servers.json");
    let entries: Vec<Value> = (0..5000).map(|i| json!({"id":"same/repo", "name":format!("fixture-{i}"), "description":"catalog fixture", "github":format!("https://github.com/same/repo/tree/{i}"), "categories":[],"language":"python","scope":"local","official":false})).collect();
    put(&cache,json!({"lastUpdated":"fixture", "source":"fixture", "totalServers":5000,"servers":entries}));
    ext.run_command("mcp-add", "", oino_extension_api::CancellationToken::new(), cx).await.unwrap();
    let panels: Vec<Value> = f.host.calls_named("ui_panel").into_iter().map(|c| c.detail).collect();
    // Panel emission itself validates unique sibling keys and encoded tree size.
    assert!(panels.len() >= 2, "{}", format!("{:?}", f.host.calls_named("ui_panel")));
    let encoded = serde_json::to_string(panels.last().unwrap()).unwrap();
    assert!(encoded.len() < 256 * 1024);
    assert!(encoded.contains("fixture-49"));
    assert!(!encoded.contains("fixture-50\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn entry_env_expands_args_and_broken_project_falls_back_with_warning() {
    let f = Fixture::new();
    f.host.allow_process_command("/usr/bin/python3");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/mcp_stdio.py");
    put(&f.agent.join("mcp.json"), json!({"mcpServers":{"refero":{"command":"/usr/bin/python3","args":[script,"global"]}}}));
    put(&f.project.join(".unipi/config/mcp/mcp-config.json"), json!({"mcpServers":{"refero":{"command":"/usr/bin/python3","args":[script,"project","--header","Authorization:${REFERO_AUTH}"],"env":{"REFERO_AUTH":"fixture-only"}}}}));
    let ext = f.load().await;
    let state = f.service(&ext, "mcp.servers", json!({})).await;
    assert_eq!(row(&state,"refero")["status"], "running", "{state}");
    assert!(row(&state,"refero")["warning"].is_null(), "entry env must win, not fallback: {state}");
    put(&f.project.join(".unipi/config/mcp/mcp-config.json"), json!({"mcpServers":{"refero":{"command":"never-execute-this","env":{"TOKEN":"${UNSET_FIXTURE}"}}}}));
    let state = f.service(&ext,"mcp.server-action",json!({"action":"reconnect","name":"refero"})).await;
    let row = row(&state["servers"],"refero");
    assert_eq!(row["status"], "running", "{state}");
    assert_eq!(row["source"], "global");
    assert!(row["warning"].as_str().unwrap().contains("Using global"));
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_project_secret_migrates_and_masked_override_keeps_auth() {
    let f = Fixture::new();
    put(&f.agent.join("mcp.json"), json!({"mcpServers":{}}));
    let ext = f.load().await;
    let saved = f.service(&ext, "mcp.save", json!({"scope":"global","entries":{"legacy":{"command":"unused","enabled":false,"env":{"API_KEY":"${LEGACY_API_KEY}"}}},"secrets":{"LEGACY_API_KEY":"fixture-legacy"}})).await;
    assert_eq!(saved["ok"], true);
    put(&f.project.join(".oino/mcp.json"), json!({"mcpServers":{"legacy":{"command":"unused","enabled":false,"env":{"API_KEY":"${LEGACY_API_KEY}"}}}}));
    f.service(&ext,"mcp.server-action",json!({"action":"reload"})).await;
    let migrated = std::fs::read_to_string(f.project.join(".oino/mcp.json")).unwrap();
    assert!(migrated.contains("PROJECT_") && migrated.contains("_LEGACY_API_KEY}"));
    let global = std::fs::read_to_string(f.agent.join("mcp.json")).unwrap();
    assert!(global.contains("${LEGACY_API_KEY}"));
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/mcp_stdio.py");
    f.host.allow_process_command("/usr/bin/python3");
    put(&f.project.join(".unipi/config/mcp/mcp-config.json"), json!({"mcpServers":{"refero":{"command":"/usr/bin/python3","args":[script,"refero","--header","Authorization:${REFERO_AUTH}"],"enabled":false,"env":{"REFERO_AUTH":"fixture-compat"}}}}));
    f.service(&ext,"mcp.server-action",json!({"action":"reload"})).await;
    let state = f.service(&ext,"mcp.servers",json!({})).await;
    let mut entry = row(&state,"refero")["entry"].clone();
    assert_eq!(entry["env"]["REFERO_AUTH"], "••••");
    entry["enabled"] = json!(true);
    let saved = f.service(&ext,"mcp.save",json!({"scope":"project","entries":{"refero":entry}})).await;
    assert_eq!(saved["ok"], true, "{saved}");
    let canonical = std::fs::read_to_string(f.project.join(".oino/mcp.json")).unwrap();
    // N8 de-duplicates an env key already prefixed with the server name.
    assert!(canonical.contains("_REFERO_AUTH}"));
    assert!(!canonical.contains("_REFERO_REFERO_AUTH}"));
    assert!(!canonical.contains("fixture-compat") && !canonical.contains("••••"));
    assert!(!std::fs::read_to_string(f.agent.join("mcp.json")).unwrap().contains("fixture-compat"));
    // The synthetic server refuses startup if the header is unresolved or differs
    // from the original secret. A running row proves the masked override kept auth.
    assert_eq!(row(&saved["servers"], "refero")["status"], "running", "{saved}");
    assert_eq!(row(&saved["servers"], "refero")["toolCount"], 2);
    let reloaded = f.service(&ext,"mcp.server-action",json!({"action":"reload"})).await;
    assert_eq!(row(&reloaded["servers"], "refero")["status"], "running", "{reloaded}");
    assert_eq!(canonical, std::fs::read_to_string(f.project.join(".oino/mcp.json")).unwrap(), "migration is idempotent");
    ext.on_event(oino_extension_api::EventKind::SessionShutdown, RawJson::from(json!({})), f.cx().with_kind(CallKind::Event)).await.unwrap();
}
