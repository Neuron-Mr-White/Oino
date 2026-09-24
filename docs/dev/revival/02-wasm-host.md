# P2: WASM Host

Goal: a real, sandboxed extension runtime built on Wasmtime components, able to host
tools, commands, hooks and streaming model providers. Replaces the fixture-only
`JsonWasmRuntime` (`crates/oino-extension-runtime/src/lib.rs`) and the `wasm-json-v1` ABI.

## Current state

- Manifests (`oino.extension.json`), `RuntimeKind::{Wasm, BuiltIn, NativeSidecar}`,
  `ExtensionPermissions` and `CapabilityBroker` exist and are tested.
- No engine. Every shipped built-in is `runtime.kind: built_in`: its behavior is native
  Rust in `oino-app`/`oino-tui`, and its manifest only declares contributions.
- `examples/extensions/rust-wasm-fixture/plugin.wasm` is not executed by anything.

## Dependencies

- `wasmtime`, `wasmtime-wasi`, `wasmtime-wasi-http`: newest release published at least 7
  days before the pin date (49.0.0 was current on 2026-09-24; use 48.x if 49 is newer
  than 7 days at implementation). Needs rustc >= 1.96 (we have 1.98.1).
- Guest side: `wit-bindgen` (0.62.x), target `wasm32-wasip2`.
- Features: `component-model`, `async`, `cranelift`, `cache`, `pooling-allocator`.
  Do not depend on `component-model-async` (WASI 0.3 streams) in v0.1; the pull-based
  resource design below works on WASI 0.2 and can adopt native streams later.

## Crate layout

- New `crates/oino-wasm-host`: engine, linker, store, capability-gated host imports,
  compiled-module cache, instance lifecycle. Depends on `oino-extension-core`,
  `oino-types`, `oino-agent-loop`.
- `oino-extension-runtime`: keeps the capability broker, hook dispatch and the
  `RuntimeHost` trait; the JSON-v1 lifecycle types and `FixtureWasmModule` are deleted.
- New guest workspace `extensions/Cargo.toml` (separate from the host workspace, built
  with `--target wasm32-wasip2`), containing `oino-extension-guest` (bindings plus
  ergonomic macros) and, from P3 onward, provider and extension crates.
- `oino-extension-sdk` stays host-side (pack, validate, install tooling) and gains
  `oino-extension-sdk build`, which builds a guest crate and places `plugin.wasm` into the
  package.

## WIT package `oino:extension@0.1.0`

Lives in `wit/` at repo root; both host (`wasmtime::component::bindgen!`) and guest
(`wit_bindgen::generate!`) read it.

```wit
package oino:extension@0.1.0;

interface types {
  // Mirrors of oino-types (P1): message, content-block, tool-declaration, usage,
  // thinking-level, stop-reason, model. Complex/opaque fields are `json` (string).
  type json = string;
  record model { provider: string, name: string }
  // ... message, content-block, assistant-event variants mirror oino_types exactly.
}

interface host {           // imports, each gated by a manifest permission
  use types.{json};
  log: func(level: u8, msg: string);
  secret: func(name: string) -> result<string, string>;           // permissions.secrets
  kv-get: func(key: string) -> option<list<u8>>;                  // session_persistence
  kv-set: func(key: string, value: list<u8>) -> result<_, string>;
  session-append: func(kind: string, payload: json) -> result<_, string>;
  notify: func(level: u8, text: string);                          // ui
  confirm: func(title: string, body: string) -> bool;             // ui (async on host)
  exec: func(cmd: string, args: list<string>, cwd: option<string>) -> result<exec-output, string>; // shell_process
  record exec-output { status: s32, stdout: list<u8>, stderr: list<u8> }
}

interface guest {          // exports
  use types.{json};
  record registration {
    tools: list<types.tool-declaration>,
    commands: list<command-decl>,
    hooks: list<string>,          // event kinds this extension handles
    providers: list<string>,      // provider ids served via `provider` interface
  }
  record command-decl { name: string, description: string }
  init: func(config: json) -> result<registration, string>;
  handle-event: func(kind: string, payload: json) -> result<json, string>;
  run-tool: func(name: string, call-id: string, args: json) -> result<tool-output, string>;
  run-command: func(name: string, args: string) -> result<json, string>;
  record tool-output { content: list<types.content-block>, is-error: bool, details: option<json>, added-tool-names: list<string>, terminate: bool }
}

interface provider {
  use types.{model, json};
  list-models: func() -> result<json, string>;          // catalog, see P3
  resource response {
    // Pull-based: host calls repeatedly until `none`. Each call returns a batch.
    next: func() -> result<option<list<types.assistant-event>>, string>;
    cancel: func();
  }
  stream: func(request: json) -> result<response, string>;
}

world extension { import host; import wasi:http/outgoing-handler@0.2.x; export guest; }
world provider-extension { include extension; export provider; }
```

Notes:

- `request: json` for `provider.stream` is the serialized P1 `StreamRequest` (transcript,
  model, thinking level, session id, cache retention). JSON here keeps the wire stable
  while `oino-types` evolves; typed records can replace it in 0.2.
- Tool progress uses an import `host.tool-progress(call-id, message, details)`, callable
  only during `run-tool`.

## Host behavior

- **Engine:** one shared `Engine` with epoch interruption enabled, plus a background
  ticker (10 ms). Compiled components are cached on disk under
  `~/.oino/cache/wasm/<sha256>.cwasm` and invalidated by component hash plus Wasmtime
  version.
- **Instances:** one `Store` + instance per extension, created lazily on first use and
  kept for the session. Calls into one instance are serialized (`tokio::sync::Mutex`);
  different extensions run concurrently. Each provider `response` runs its pull loop on
  a dedicated task.
- **Limits:** memory cap per store via `StoreLimits` (default 256 MiB, overridable in
  settings). Wall-clock deadline per call via epoch deadline (hook 2 s, command 30 s,
  tool from manifest or 10 min, provider `next` 120 s idle). A trap or deadline marks
  the extension unhealthy in the registry and surfaces a diagnostic; the host re-creates
  the instance on next use at most 3 times per session.
- **Capability gating:**
  - Imports are linked for every extension, but each import checks
    `CapabilityBroker` against the manifest before acting.
  - `wasi:http` requests go through a custom `WasiHttpView::send_request` that enforces
    `permissions.network.hosts` (exact host match, `raw_network: false` means deny all).
  - Filesystem uses WASI preopens built from `permissions.filesystem` (read-only vs
    read-write); no ambient cwd.
  - Environment is empty except declared names.
  - Secrets come only through `host.secret`, never through env.
- **Streaming path:** host `StreamProvider` implementation (`WasmStreamProvider`) calls
  `provider.stream`, then loops `next()` and forwards each `assistant-event` to the loop's
  `StreamEventSink`. Abort calls `response.cancel()` and drops the resource.
  Budget: host-side overhead per batch p95 < 1 ms (measured in a bench, see acceptance).
- **Hook dispatch:** `oino-extension-runtime` hook ordering, modes
  (observe/mutable/cancellable/blocking) and timeouts stay as they are; the host only
  adds the WASM transport.
- **Trust:** installing a WASM package shows its permissions for approval (existing
  manager flow). `built_in` remains valid for host-owned UI contracts
  (ask-user modal, footer-status).

## Migration

1. Implement `oino-wasm-host` plus the WIT package.
2. Rebuild `examples/extensions/rust-wasm-fixture` as a real guest (a tool, a command, a
   hook), committed prebuilt as `plugin.wasm` and rebuilt by a script.
3. Replace `FixtureWasmModule` tests with tests that load the real fixture component.
4. Update `docs/dev/extension-dev.md` with the guest workflow
   (`rustup target add wasm32-wasip2`, `oino-extension-sdk build`).

## Acceptance

- Fixture component: tool call round-trip, command, hook mutation, `tool-progress`,
  kv persistence across instance restart.
- Denials: HTTP to an undeclared host, reading an undeclared path, `exec` without
  `shell_process`, and a secret not declared all fail with a typed capability error.
- Limits: an infinite-loop guest is interrupted by its deadline; a memory-hog guest
  traps; the app stays responsive and marks the extension unhealthy.
- Streaming bench: a faux provider component emitting 10k events in batches keeps p95
  host overhead per batch under 1 ms (`cargo bench`, or a `#[ignore]` test that prints
  numbers).
- Warm start: the second launch loads from the `.cwasm` cache (assert no compile call).
