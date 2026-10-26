# Embedding Oino (SDK)

Oino is a set of Rust crates; frontends embed the runtime in-process, as the terminal UI and
Oino Desktop do.

| Crate | Role |
|---|---|
| `oino-ai` | message, model and stream types (pi-ai shapes) |
| `oino-session` | session format v3, `SessionManager` |
| `oino-agent` | agent loop, tools contract |
| `oino-tools` | built-in tools |
| `oino-settings`, `oino-resources`, `oino-models` | settings, resources (skills, prompts, context files), model registry and auth |
| `oino-core` | `Services`, `AgentSessionRuntime`, `AgentSession`, extension runner, compaction, export |
| `oino-wasm-host` | wasmtime host; `core_bridge::CoreLoader` loads packages into `oino-core` |
| `oino-bundle` | the bundled components and docs |
| `oino-client` | `SessionBackend` trait with in-process and fake backends |
| `oino-rpc` | RPC codec, server and typed client (`RpcBackend`) |
| `oino-acp` | ACP agent over `oino-core` |
| `oino-view`, `oino-tui` | frontend-agnostic view models and the ratatui client |

Minimal embedding:

```rust
let loader = Arc::new(oino_wasm_host::core_bridge::CoreLoader::new(runtime, packages));
let services = oino_core::services::Services::create(ServicesOptions {
    cwd, agent_dir, docs_root, mode: ExtensionMode::Rpc, has_ui: true,
    flags: ResourceFlags::default(), extension_flag_values: IndexMap::new(),
    loader, approve: None, models: None, settings: None,
}).await?;
let runtime = AgentSessionRuntime::create(services, SessionOptions { /* … */ }).await?;
let backend = oino_client::InProcessBackend::new(Arc::new(runtime));
```

See `crates/oino/src/startup.rs` (CLI) and `apps/oino-desktop/src/embedded.rs` (desktop) for
complete examples, and `docs/dev/v1/architecture.md` for the crate layering rules. Each crate's
rustdoc (`cargo doc --workspace --no-deps --open`) starts with an API map.

Out of process, drive `oino --mode rpc` ([rpc.md](rpc.md)) or `oino --mode acp`
([acp.md](acp.md)).
