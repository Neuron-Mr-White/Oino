# v2 spec — Extension ABI: kanboard CLI, host services, cron job files, `--exclude-extension`, long-horizon recovery (`ext-abi`)

Status: **proposed** (implementation-ready). Owner: `ext-abi`. Inputs: [DIRECTION.md](../DIRECTION.md)
§3.4 (WIT changes), §3.3 item 4 (`--exclude-extension`), §7 (long-horizon recovery), §2 (unipi-host),
§8 (distribution). Context: [extension-api.md](../../v1/extension-api.md), [unipi-port.md](../../v1/unipi-port.md)
§3.16/§3.24, [browser-control.md](../../v1/browser-control.md) §8. Reference unipi originals:
`~/.pi/agent/npm/node_modules/@pi-unipi/unipi/packages/kanboard` (`src/bin.ts`, `guard.ts`, `monitor.ts`,
`reminders.ts`, `bus-hooks.ts`, `skills/kanboard`), `packages/long-horizon/src/graph/engine.ts`.

**Coordination boundaries** (referenced, not redefined here):

| Topic | Owner |
|---|---|
| Core durability (§3.1 flush points, resume-from-session, single-writer lock), repo/package split, physical crate relocation out of the root workspace, bundle/load-order machinery | `spec-core` |
| unipi-host internals (process model, tray, app↔host proto surface, how the host runs the kanboard CLI for the Board, cron scheduler embedding, browser service wiring) | `spec-host` |
| Extension UI protocol v2, ratatui vocabulary, slots, reserved panel ids, per-extension toggle in the app | `spec-ui-protocol` |

Where this spec says a crate "moves to the unipi side", the *dependency cut* (what stops depending on
it) and the crate's new role are binding here; the *physical destination path and workspace* are
`spec-core`'s (§9 phase 3). Interim locations in §5–§7 keep the tree green until then.

---

## 1. Current state (findings)

### 1.1 WIT and host

- `wit/oino-extension/kanboard.wit` — `kanboard.exec(request)` runs an `oino-kanboard` argv
  **in-process** in the host (round-5 #7; no binary, no daemon). `wit/oino-extension/cron.wit` —
  `cron.call(op, payload)` (18 ops, permission `cron`). `wit/oino-extension/browser.wit` —
  `browser.execute(command, cancel) -> outcome{result, image}` + `browser.status()`, permission
  `browser`. All three are imported by `world extension` (`wit/oino-extension/worlds.wit:15-17`).
  Package version is `oino:extension@1.0.0` (re-declared in every `.wit` file).
- `crates/oino-wasm-host` implements the three imports in-process:
  - `src/imports/kanboard.rs:29-53` — parses the JSON request, defaults `cwd` to the session cwd and
    the board home to the host agent dir, calls `oino_kanboard::service::exec` (library call).
  - `src/imports/cron.rs` — permission check then `HostServices::cron_call`.
  - `src/imports/browser.rs` — permission check then `HostServices::browser_execute/_status`.
- `crates/oino-wasm-host/src/hub.rs` (`LocalServices`): process-wide `set_browser`/`set_cron`
  (`hub.rs:88-103`) + lazy fallback to the **daemon client** `oino_cron::host::DaemonCronHost`
  (`hub.rs:105-124`); `browser_execute` returns `BrowserResult::unavailable("disabled", …)` when no
  browser host was installed (`hub.rs:664-682`).
- Dependencies that make oino unipi-specific: `crates/oino-wasm-host/Cargo.toml:25-29`
  (`oino-browser-protocol`, `oino-cron`, `oino-kanboard`), `crates/oino-extension-api/Cargo.toml:17`
  (`oino-browser-protocol`, for `src/browser.rs` `BrowserHost` trait + `src/cron.rs` `CronHost`),
  `crates/oino/Cargo.toml:25,29` (`oino-browser`, `oino-cron`).
- `crates/oino/src/browser.rs` — the `oino` binary creates one `oino_browser::BrowserService` per
  process and installs it into the wasm host (`OINO_BROWSER=off|ephemeral|headless|window`).
- `crates/oino/src/cli/cron.rs` — the `oino cron` CLI: `serve`, `install`, `uninstall`, storage ops,
  and `run`/`stop`/`status` routed to the live daemon over the loopback control socket
  (`cli/cron.rs:595-598`). `crates/oino-cron/src/serve.rs` — single-owner flock + loopback TCP
  control socket + `serve.json`; `crates/oino-cron/src/install.rs` — systemd/launchd/schtasks
  install. `crates/oino-cron/src/exec.rs:2-7` — each fire spawns `oino -p "<prompt>" --mode json`
  in its own session file.
- Cron storage today (`crates/oino-cron/src/paths.rs:1-11`): one `jobs.json` under
  `~/.oino/cron/` (written under `jobs.lock`), `runs/<job>/<run>.json|.log` (64 KiB tail cap),
  `settings.json`, `scheduler.lock`, `serve.json`.
- Process permission matching is by **file name**: `Permissions::allows_program`
  (`crates/oino-extension-api/src/manifest.rs:394-410`) strips directories and `.exe`, so an
  absolute path whose basename is allowlisted passes. `ExtShared`
  (`crates/oino-wasm-host/src/state.rs:54-72`) does **not** hold the package root;
  `LoadedPackage.root` exists (`crates/oino-extension-api/src/runtime.rs:73-74`) but is unused by
  the process service.
- Extension loading: `crates/oino/src/loader.rs` — bundled → `~/.oino/extensions` → project
  `.oino/extensions` → `-e` (`LoaderContext`, `loader.rs:69-76`); `-ne` keeps providers only
  (`loader.rs:171-172`). `disabledExtensions` (global settings) already exists end-to-end:
  `oino-settings/src/manager.rs:629-637`, applied in `oino-core/src/services.rs:588-594` via
  `ExtensionLoader::load_enabled(host, trusted, disabled)` and toggleable at runtime
  (`oino-core/src/session/feeds.rs:293-306`). There is **no** CLI exclude flag today (only
  `--no-extensions`, `crates/oino/src/cli/args.rs:347`).

### 1.2 Kanboard

- `crates/oino-kanboard` — lib + bin `oino-kanboard` (`Cargo.toml` `[[bin]] src/main.rs`). CLI:
  `project add/list/show/archive/unarchive/rebind`, `add list show move note attach attachments
  edit link unlink order start finish next reap chain search release duplicate archive-sweep status
  validate` (`src/cli.rs`). No daemon since round-5; `status` is in-process. Output contract:
  success → pretty JSON on stdout, exit 0 (except `validate` findings → 1); failure → one JSON line
  `{"ok":false,"error":…,"kind":"usage|not_found|rule|io"}` on **stderr**, exit 2 (usage) / 1
  (other) (`src/main.rs:14-54`, `src/error.rs:50-58`). `src/service.rs` additionally returns error
  payloads inline (`exec_json`, `err_payload`) and exposes `watch_projects` (notify watcher,
  coalesced 150 ms/project, `service.rs:211-330`) — the refresh signal the daemon SSE used to carry.
- `extensions/unipi/kanboard` — the extension: `cli.rs` currently calls the WIT
  `kanboard.exec` **in-process** (`cli.rs:127-152`; `binary`/`timeout_ms` parameters are ignored);
  the binary-run interpreter `interpret()` (`cli.rs:65-105`, pi-shape `{stdout, stderr, code,
  killed}`) and `argv()` `--json` handling are already there and tested, currently dormant.
  `shell.rs` classifies the agent's bash calls into the binary (`BINARY = "oino-kanboard"`,
  `KNOWN_SUBCOMMANDS`, `shell.rs:8-49` — note: the list still contains removed subcommands
  `queue`, `unqueue`, `claim-next`, `set-run`, `serve`, `settings`, `rotate-token`, `stop`).
  `guard.rs` write budget, `monitor.rs` nudge provider (50/40), `reminders.rs`, bus-hooks
  (compaction context, re-arm on owner resume) mirror unipi. Manifest already grants
  `process.spawn` with `allow: ["oino-kanboard", "tailscale"]` + `detached`, and env
  `OINO_KANBOARD_BIN|CHILD|HOME` (`extensions/unipi/kanboard/extension.json:12-14`).
  The binary path is probed via `status --json` → `payload.binary` and cached
  (`src/lib.rs:91-92, 210-216`), then handed to the agent (`lib.rs:260`).
- unipi reference (`packages/kanboard/src/bin.ts`): resolution order `UNIPI_KANBOARD_BIN` →
  platform package `@pi-unipi/kanboard-<platform>-<arch>` `bin/unipi-kanboard(.exe)` → dev build
  `<repo>/crates/kanboard/target/{release,debug}/…`; when nothing resolves, every command reports
  `kanboard binary unavailable for <platform>-<arch>` (`unavailableMessage`). Runner defaults:
  timeout 10 s, `maxBuffer` 8 MiB, actor defaults `user`, `--json` auto-appended. The skill says
  "call it by absolute path (it may not be on PATH)".
- unipi-app 1.0.0 Board (now `apps/unipi`) talks to a `unipi-kanboard serve` daemon over loopback
  HTTP + SSE (`apps/unipi/crates/host/src/board/mod.rs:1-16`, spawns the daemon when idle-dead).
  v2 deletes that daemon path (DIRECTION §3.4.1).

### 1.3 Browser extension

- `extensions/oino/browser` (id `oino.browser`) registers the 19 `browser_*` tools only when
  `browser.status().available` (`src/lib.rs:52-120`), re-checking on `session_start`; drives turn-tail
  screenshots and lifecycle itself (`turn_ended`, `session_closed` — `lib.rs:135-160`); manifest
  permission `"browser": true` (`extension.json`).

### 1.4 Long-horizon graph

- `extensions/unipi/long-horizon/src/graph.rs` ports unipi `engine.ts`: pure state machine; work
  statuses `queued|running|completed|failed|stopped|cancelled|superseded`; `blocked` is derived
  (`input_state`, blocked when an input is failed/unknown). Children run through the
  `subagents.graph-run` service (resume via `resume_operator_id`); outcomes arrive on the bus topic
  `unipi:subagents:run-done` and settle via `settle(operator, status, text, error, tool_calls)`
  (`graph.rs:909-953`). Host-crash repair on `bind`: running work with an operator is **resumed
  once** (`restart_resumes`, `MAX_RESTART_RESUMES = 1`), else `failed("interrupted: …")` — the port
  keeps unipi's semantics. State persists to `graph.json` in the `ext-workspace` preopen
  (`src/lib.rs:211-234`), written after every change. **There is no transient-failure retry today**:
  any failed settle is terminal; the supervisor is woken with the failed work in its attention set
  (`checkpoint`, `graph.rs:965-1005`).
- `Work` already carries `attempts` and `restart_resumes` fields (`graph.rs:111-114`).

---

## 2. Decision summary

| # | Decision |
|---|---|
| D1 | Remove `kanboard.wit`; the kanboard extension calls the **`unipi-kanboard` CLI** (rename of `oino-kanboard`) through `process.exec`. No daemon, no library call in the host. |
| D2 | The CLI binary ships **inside the extension package** at `bin/<name>(.exe)`; the host's process service resolves allowlisted program names against the calling extension's package `bin/` dir before `PATH`. One package per platform (the installer picks its own). |
| D3 | Replace `cron.wit` and `browser.wit` with one generic interface `host-service` (`call(name, op, json)`, `available(name)`, `list()`); permission `hostServices: [names]`. Oino registers none; unipi-host registers `browser` and `cron`. |
| D4 | Cron jobs become **one JSON file per job** under `~/.oino/cron/jobs/`, written by the cron extension (fs alias preopens) and the `unipi-host cron` CLI; unipi-host's embedded scheduler watches and fires them. `oino cron serve`/`install`/`uninstall` (and with them the whole `oino cron` subcommand and the control socket/`serve.json`) are removed from `oino`. |
| D5 | `oino --exclude-extension <id>` (repeatable, comma-lists accepted) filters the composed package set by id before instantiation; additive to `--no-extensions` and to the settings `disabledExtensions` list. |
| D6 | Long-horizon graph work items auto-retry **transient** failures (429/Retry-After, network, provider 5xx, crashed worker) from the node's own session: up to 3 retries, backoff 30 s / 2 min / 10 min, Retry-After waits honored without consuming budget (1 h cumulative safety cap). Then `failed`, downstream `blocked`, supervisor nudge, user notification. |
| D7 | WIT package bumps to `oino:extension@2.0.0`; manifest `apiVersion` `oino.extension/2`; removed interfaces never come back; no dual-version hosting — all components rebuild (precedent: deviation #23). |

---

## 3. WP-X1 — generic `host-service` interface (WIT + host + registry)

**Additive first** (no interface removed yet); this WP is green on its own.

### 3.1 WIT (new file `wit/oino-extension/host-service.wit`, added to `world extension` imports)

```wit
package oino:extension@1.0.0;   // becomes @2.0.0 in WP-X6

/// Host-provided services (v2 §3.4.2): named capabilities an embedding host
/// (unipi-host) registers with the runtime — "browser", "cron", … . Oino
/// itself registers none: under a plain TUI every name is unavailable and
/// extensions degrade (typically: tools report unavailable).
interface host-service {
    use types.{json, host-error, cancel-token};

    /// Names of the services this host provides (sorted). Not permission-gated
    /// (discovery); service names are not secret.
    list: func() -> list<string>;
    /// Whether `name` is provided by this host. Cheap, never traps.
    available: func(name: string) -> bool;
    /// Runs one operation of a host service. `payload` and the reply are the
    /// service's own JSON contract (documented by the service, not by WIT):
    /// user-facing failures travel inside the reply; `host-error` is only for
    /// permission (`denied("hostServices:<name>")`), missing services
    /// (`not-found`) and transport/timeout problems.
    call: async func(name: string, op: string, payload: json,
                     cancel: option<borrow<cancel-token>>) -> result<json, host-error>;
}
```

Deliberately **no stream/event function**: browser screencast frames and tab events for UIs travel
the app↔host protocol (`spec-host`/`spec-ui-protocol`), not the WIT boundary; push events an
extension needs arrive on the existing `bus` (e.g. cron publishes `unipi:cron:run-finished`).
If a future service genuinely needs guest-side streaming, add it in 2.x (additive) or 3.0.

### 3.2 Manifest permission

New key `permissions.hostServices`: list of service names or `"*"` (`"*"` honoured only for bundled
packages, like `network.hosts`/`process.allow`). Default `[]` (denied). Replaces `cron: true` and
`browser: true` when those interfaces are removed (WP-X4/X5). Install-time text for a name: *"Call
the host service '<name>'"*; `"browser"` additionally shows browser-control.md §8.4's wording.

- `host-service.list` / `available`: not gated.
- `host-service.call`: gated per name; unlisted → `host-error::denied("hostServices:<name>")`.
- Unknown/absent service → `host-error::not-found("host service <name>")`; the extension maps that
  to its own "unavailable" presentation.

### 3.3 Rust contracts (`oino-extension-api`)

- New module `crates/oino-extension-api/src/host_service.rs`:

```rust
/// One host-provided service, registered process-wide by an embedding host.
pub trait HostService: Send + Sync {
    /// Runs one op. `payload`/reply are the service's own JSON contract.
    /// User-facing failures travel inside the reply JSON.
    fn call(&self, op: &str, payload: RawJson, cancel: CancellationToken)
        -> BoxFut<'_, Result<RawJson, HostError>>;
}
```

- `HostServices` (runtime.rs): replace `browser_execute`/`browser_status`/`cron_call` (removed with
  their WPs) with, added now:
  `fn host_service(&self, ext: &ExtensionId, cx: &CallContext, name: &str, op: &str, payload: RawJson, cancel: CancellationToken) -> BoxFut<'_, Result<RawJson, HostError>>` (default `unsupported`)
  and `fn host_services(&self) -> Vec<String>` (default empty). This is the seam an embedding host
  that does not use the wasm-host hub would implement.
- `Permissions` (manifest.rs): add `pub host_services: Vec<String>` (+ `allows_host_service(name,
  bundled)` mirroring `allows_host`), default empty.

### 3.4 Wasm host

- `LocalServices` (hub.rs): add `host_services: RwLock<BTreeMap<String, Arc<dyn HostService>>>` with
  `pub fn register_host_service(&self, name: &str, service: Arc<dyn HostService>)` and
  `pub fn host_service(&self, name: &str) -> Option<Arc<dyn HostService>>`. This **replaces**
  `set_browser`/`set_cron`/`browser()`/`cron()` (those are deleted in WP-X4/X5 with their traits).
- New `crates/oino-wasm-host/src/imports/host_service.rs` mirroring the shape of `imports/cron.rs`:
  permission check (`call` only) → hub registry first, then `inner.host_service(...)` → convert.
  `list`/`available` read the registry (+ `inner.host_services()`).
- `bindings.rs` regenerated (bindgen covers the new interface).

### 3.5 Behaviour when no host provides a service

- Plain `oino` (TUI/CLI/RPC/ACP): registry empty. `available("browser") == false`;
  `call("browser", …)` → `not-found`. The ported `unipi.browser` extension checks `available` before
  registering tools and its tools return the existing "browser unavailable" text — same UX as v1's
  `BrowserResult::unavailable("disabled", …)` path (hub.rs:664-682).
- unipi-host registers `"browser"` and `"cron"` at startup (its wiring is `spec-host`'s; the traits
  it implements are defined here).

### 3.6 Files / acceptance

Files: `wit/oino-extension/host-service.wit`, `worlds.wit` (add import), `crates/oino-extension-api/src/{host_service.rs, manifest.rs, runtime.rs, lib.rs}`, `crates/oino-wasm-host/src/{hub.rs, imports/host_service.rs, imports/mod.rs, bindings.rs, state.rs (no change expected)}`, `crates/oino-wasm-host/tests/*` (new host tests: allowed/denied per name, unknown service, registry vs inner fallback).

Acceptance:
```sh
wasm-tools component wit wit/oino-extension
CARGO_BUILD_JOBS=8 cargo check --workspace
CARGO_BUILD_JOBS=8 cargo test -p oino-extension-api -p oino-wasm-host
cargo clippy -p oino-extension-api -p oino-wasm-host --all-targets -- -D warnings
cargo fmt --all --check
```

---

## 4. WP-X2 — `oino --exclude-extension <id>`

### 4.1 Semantics

- `--exclude-extension <id>`: **repeatable**; each occurrence also accepts comma-separated ids
  (consistent with `--exclude-tools`, args.rs:312-314). Accepted in every mode (the flag is parsed
  globally like `-ne`).
- Filtering happens **by extension id after composition, before instantiation**: bundled, installed
  (`~/.oino/extensions`), project (`.oino/extensions`) and explicit `-e` packages are all subject;
  `builtin:` test components are not. Providers can be excluded too (id-based, uniform).
- Composition with the other switches:
  - `--no-extensions` still skips all discovered/bundled `kind: "extension"` packages (providers
    stay) — exclusion then only matters for providers and `-e` packages.
  - Settings `disabledExtensions` (global) remains the persistent, runtime-toggleable list
    (core applies it in `services.rs:588-594`; the app toggle is `spec-ui-protocol`'s). The CLI flag
    is an **ephemeral union** with it: a package excluded either way does not load. The flag never
    writes settings.
  - Exclusion does not bypass load-order semantics (it removes entries; `-e` replacement-in-place
    happens first, then the filter).
- Diagnostics: when an excluded id matches no composed package, emit a **warning diagnostic**
  (helps safe-mode §3.3.3 name the right suspect); never fatal.
- Safe mode (§3.3.3, 2 failed starts → extensions off) is `spec-core`'s crash-protection work; this
  flag is the manual lever it names.

### 4.2 Files / acceptance

Files: `crates/oino/src/cli/args.rs` (`pub exclude_extensions: Option<Vec<String>>` + parsing +
tests), `crates/oino/src/cli/help.rs` (flag line next to `--no-extensions`), `crates/oino/src/loader.rs`
(`LoaderContext.exclude_extensions: Vec<String>`; filter + no-match warning in `CliLoader::new`),
`crates/oino/src/startup.rs:371,465` (thread through), `crates/oino/src/cli/mod.rs` (usage text).

Acceptance:
```sh
CARGO_BUILD_JOBS=8 cargo test -p oino cli::args loader
oino --exclude-extension unipi.kanboard,oino.anthropic -p "hi" --mode print   # neither loads; log names them
oino --exclude-extension does.not-exist -p "hi" --mode print                  # warning diagnostic, exit 0
cargo fmt --all --check && cargo clippy -p oino --all-targets -- -D warnings
```
(Verification of "not loaded": the startup diagnostics list loaded extensions; assert absence.)

---

## 5. WP-X3 — kanboard via the `unipi-kanboard` CLI

### 5.1 Naming and placement

- Binary and crate rename **`oino-kanboard` → `unipi-kanboard`** (D1). Rationale: DIRECTION §3.4.1/§8
  name `unipi-kanboard`; the CLI is UniPi distribution content, and `oino` must build without it.
  Env prefix follows the product: `UNIPI_KANBOARD_BIN|HOME|ACTOR|SESSION|PROJECT|PID|CHILD|CHAIN_GATE|MAX_SESSIONS`
  (mechanical rename of the `OINO_KANBOARD_*` set; `OINO_AGENT_DIR` stays — it is oino-wide).
  Board home default is unchanged (`~/.oino/state/unipi.kanboard/`).
- Physical move is `spec-core`'s. **Interim**: `crates/oino-kanboard` → `apps/unipi/crates/unipi-kanboard`
  (the root workspace's member glob `crates/*` keeps it building — zero workspace edits). Final
  home (`extensions/unipi/kanboard/cli` or a unipi workspace) is decided by `spec-core`; this spec
  only requires: **not** a dependency of `oino`, `oino-wasm-host`, `oino-extension-api`,
  `oino-bundle`.

### 5.2 Shipping the binary inside the extension package (D2)

Package layout (per-platform; the UniPi installer bundles its platform's package — §8):

```text
unipi.kanboard/
  extension.json        # + "binaries": ["unipi-kanboard"]
  component.wasm
  skills/kanboard/…
  bin/unipi-kanboard(.exe)     # this platform's build
```

- Manifest: new optional top-level key `"binaries": ["<name>", …]` — CLI names the package ships in
  `bin/`. Validation (`oino ext validate`, xtask, loader): each declared name must exist in `bin/`
  **at package build time**; a missing binary at load time is a warning diagnostic (the extension
  still loads; its tools report unavailable). Checksums: `bin/*` entries are added to the package
  checksum manifest like any other file.
- Build (`cargo xtask wasm`): for extension packages declaring `binaries`, xtask additionally runs
  `cargo build --release -p <owning-crate> --target <host-target>` and copies the binary into
  `extensions/dist/unipi.kanboard/bin/`. The dist is therefore target-specific; CI builds one dist
  per target as it already does for the host binaries.
- **Host-side resolution** (the guest stays dumb — it cannot see its package path): the process
  service resolves a non-path `command` for `process.exec`/`child.spawn` in this order:
  1. each entry of `OINO_EXT_BIN_DIR` (path-list env, host-level, dev-only override — a stale
     package binary must not shadow a fresh dev build, unipi `bin.ts`'s rationale);
  2. `<package-root>/bin/<name>[.exe]` when the calling extension's manifest declares `name` in
     `binaries` (`ExtShared` gains `package_root: Option<PathBuf>` from `LoadedPackage.root`);
  3. `PATH` (the installer also puts the CLIs on PATH, §8).
  Absolute/relative paths pass through unchanged (permission matching is already by file name,
  manifest.rs:394-410). This mirrors unipi `bin.ts`'s env-override → package → dev-build order in a
  host-appropriate shape.
- **Guest-side override** (tests/tinkerers, unipi parity): the extension keeps reading
  `UNIPI_KANBOARD_BIN` (granted via `permissions.env`) and passes it as the absolute `command`.
- The absolute path for the agent's skill/bash lines: the extension probes
  `unipi-kanboard status --json` and caches `payload.binary` exactly as today (lib.rs:91-92,210-216).

### 5.3 Extension changes (`extensions/unipi/kanboard`)

- `cli.rs`: delete the WIT call; `run()` uses `process::exec` with the resolved binary
  (env override → `BINARY` name; the host resolves the rest), `DEFAULT_TIMEOUT_MS` (10 s) now live,
  `cwd` = session cwd, env on top of the host's, `UNIPI_KANBOARD_ACTOR` defaulting to `user`.
  `interpret()` (already present and tested, cli.rs:65-105) becomes the live decoder again:
  stdout/stderr/`{ok:false,error,kind}` → `CliError{message, code, kind}`. Spawn failure →
  `CliError::io` → the tools/commands surface `unavailable_message(platform, arch)` (SDK
  `host::platform()`/`arch()`), matching unipi's `unavailableMessage`.
- `shell.rs`: `BINARY = "unipi-kanboard"`; prune the stale subcommand names from
  `KNOWN_SUBCOMMANDS` (`queue`, `unqueue`, `claim-next`, `set-run`, `serve`, `settings`,
  `rotate-token`, `stop`) — they are the binary's usage errors now, not write candidates (guard
  neither charges nor blocks unknown names, so this is a correctness cleanup only).
- Guard/monitor/reminders/bus-hooks/panel/settings: **behaviour unchanged**; only env-var names and
  the `unipi-kanboard` program name change. The write guard keeps classifying the agent's bash by
  argv (`shell.rs`), the monitor keeps its 50/40 nudges, compaction context and owner-resume re-arm
  stay as ported.
- Manifest: `process.allow: ["unipi-kanboard", "tailscale"]`, `env:
  ["UNIPI_KANBOARD_BIN","UNIPI_KANBOARD_CHILD","UNIPI_KANBOARD_HOME"]`, add
  `"binaries": ["unipi-kanboard"]`; drop nothing else (the kanboard WIT permission never existed —
  `kanboard.exec` was ungated; note in §8 compat).
- Skill (`skills/kanboard/SKILL.md`): unchanged content-wise (it already speaks of `<binary>` by
  absolute path and `unipi-kanboard` in unipi; the port's wording switches to the new name).

### 5.4 Host changes

- Delete `wit/oino-extension/kanboard.wit`, its world import, `crates/oino-wasm-host/src/imports/kanboard.rs`
  and the `oino-kanboard` dependency from `crates/oino-wasm-host/Cargo.toml` (this WP; the version
  bump itself is WP-X6).
- Delete the SDK's `oino_extension::kanboard` module (its only consumer is this extension).
- `oino_kanboard::service` (`exec`/`parse_request`) is no longer called from the host; it stays in
  the crate as the CLI's in-process core (used by tests and `unipi-host`'s Board if `spec-host`
  wants a library path — allowed, but the default Board path is the CLI, §5.5).

### 5.5 How the app Board calls the CLI (contract; wiring is `spec-host`'s)

- The Board is a reserved first-party screen; the app asks **unipi-host** (app↔host proto,
  `spec-host`/`spec-ui-protocol` define the frames). The host runs the CLI as **actor `user`**
  (the app acts as the user; only the agent's own calls say `--actor agent`): one process per call,
  `--json`, `--project <slug>` pinned per view. `watch_projects` (service.rs:211-330) is the
  host-side refresh signal (push change notifications over the proto; replaces the deleted SSE).
- **JSON output contract** (what the Board and any caller may rely on; stable within v2):
  - Success: exit `0`, exactly one (pretty) JSON value on stdout. Shapes per subcommand as produced
    by `apps/unipi/crates/unipi-kanboard/src/commands.rs`: `list` → `{tasks: [Task]}` (task objects carry
    `id,title,status,priority,labels,body?,ready,waitingFor,depsStatus,staleness`); `show` → task +
    activity; `project list` → `{projects: […]}`; `status` → `{home, binary, projects, …}`;
    `validate` → `{problems: […]}` (exit 1 when non-empty).
  - Failure: exit `2` (usage) or `1` (rule/not_found/io), one JSON **line** on stderr:
    `{"ok": false, "error": "<user-facing message>", "kind": "usage|not_found|rule|io"}`.
  - No other stdout output; nothing is stream-delimited. Callers decode like `interpret()`
    (stderr JSON wins on failure).
- Under a plain `oino` TUI there is no Board; nothing else changes.

### 5.6 Files / acceptance

Files: `crates/oino-kanboard/**` → `apps/unipi/crates/unipi-kanboard/**` (rename crate+bin, env-var rename,
docs), `wit/oino-extension/{kanboard.wit (delete), worlds.wit}`, `crates/oino-wasm-host/{Cargo.toml, src/imports/{kanboard.rs (delete), mod.rs, io.rs (bin resolution + package_root), state.rs, bindings.rs}}`, `crates/oino-extension-api/src/manifest.rs` (`binaries` key + validation), `extensions/unipi/kanboard/{extension.json, src/{cli.rs, shell.rs, lib.rs (env names)}}`, `extensions/sdk/oino-extension/src/{kanboard.rs (delete), lib.rs}`, `xtask/src/wasm.rs` (binaries build/copy/checksum), `extensions/unipi/kanboard/harness/tests/kanboard.rs` (spawn path, `UNIPI_KANBOARD_BIN` override), `crates/oino-bundle` (no code change expected; dist table picks up `bin/` via resources—verify).

Acceptance:
```sh
CARGO_BUILD_JOBS=8 cargo test -p oino-wasm-host && (cd apps/unipi && CARGO_BUILD_JOBS=8 cargo test -p unipi-kanboard)
CARGO_BUILD_JOBS=8 cargo test -p unipi-kanboard --test harness 2>/dev/null || cargo test -p unipi-extension --lib   # harness lives in the extension dir; run its suite
cargo xtask wasm && git diff --exit-code extensions/dist/checksums.json   # after rebuild
CARGO_BUILD_JOBS=8 cargo check --workspace
unipi-kanboard --actor agent project add --root . --name Demo --json     # exit 0, JSON on stdout
unipi-kanboard frobnicate --json; echo $?                                # exit 2, {"ok":false,…} on stderr
scripts/docs-check.sh
```

---

## 6. WP-X4 — cron: job files + unipi-host firing; `oino cron` removed (D4)

### 6.1 Crate

- `crates/oino-cron` → **`unipi-cron`** (interim `apps/unipi/crates/unipi-cron`; final placement `spec-core`).
  Role: the scheduler library **unipi-host embeds** (single owner via the existing
  `scheduler.lock` flock) and the `unipi-host cron` CLI backs onto it. It stops being a dependency
  of `oino` and `oino-wasm-host`.
- Extract the pure schedule parser (`src/schedule.rs`, no I/O) into a wasm-compatible crate
  **`unipi-cron-schedule`** (builds for `wasm32-wasip2`, like `oino-browser-protocol` does today):
  the extension validates/previews schedules in-guest; scheduler and CLI share the same definition.

### 6.2 Job storage — files, not `jobs.json`

- New layout under `~/.oino/cron/`:

```text
jobs/<id>.json      # one job per file, atomic write (tmp+rename) under jobs.lock
runs/<job>/<run>.json|.log   # unchanged (scheduler-written)
settings.json       # scheduler settings (unipi-host owned)
scheduler.lock      # single-owner flock (unipi-host)
jobs.lock           # serialises job-file writes (extension, CLI, host)
```

- `serve.json` and the loopback control socket are **deleted** (no external daemon clients remain:
  `oino cron` is gone, the hub's daemon-client fallback is gone, the app talks to unipi-host over
  its own proto). `install.rs` (systemd/launchd/schtasks) is deleted with `oino cron install`.
- Job file format (schema `"v": 2`, fields = today's `model::Job`, camelCase, `id` = file stem):

```json
{
  "v": 2, "id": "digest", "name": "Daily digest",
  "prompt": "…", "schedule": "0 30 9 * * 1-5",     // 6-field cron | "at:<RFC3339>" | plain-language
  "tz": "Europe/Berlin", "cwd": "/path",
  "model": "anthropic/claude-…", "thinking": null,
  "enabled": true, "overlap": "skip", "missed": "run-latest",
  "timeoutMinutes": 30, "costCapUsd": 1.0, "notify": {…},
  "createdAt": 0, "updatedAt": 0
}
```

  Unknown ids (file name ≠ `id` field, malformed JSON, wrong `v`) are ignored with a diagnostic;
  deletion = remove the file; enable/disable = rewrite with `enabled` flipped. Every writer
  (extension, CLI, host) uses atomic write under `jobs.lock`; the scheduler **watches** `jobs/`
  (notify, like `watch_projects`) and rescans on change, so edits land without a restart.
- Migration: whoever opens the store first (`unipi-host` startup or `unipi-host cron` CLI) splits a
  legacy `jobs.json` into `jobs/*.json` under `jobs.lock` and renames it to `jobs.json.imported`
  (idempotent; same code path, tested with a fixture).

### 6.3 Firing

- unipi-host runs the embedded scheduler as the single owner (lock); each fire spawns a headless
  run in its own session file exactly as today (`exec.rs` `oino -p "<prompt>" --mode json …`), and
  publishes `unipi:cron:run-finished` on the bus (sticky) as the wake/notification hook — mechanism
  unchanged, host moves. Runs/notify/missed-run collapse/concurrency gate behaviour is unchanged
  (unipi-port §3.24).
- Under a plain `oino` TUI: nothing fires (no scheduler); job files can still be created/edited via
  the extension — the extension's run/status tools report "no scheduler running (unipi-host is not
  running)".

### 6.4 The cron extension (`extensions/unipi/cron`)

- Job CRUD (`add`, `edit`, `remove`, `enable`, `disable`) and reads (`list`, `show`, `runs`, `log`)
  go straight to the **files** — works identically with or without a host. Manifest gains fs
  aliases: `fs.paths: [{path: "~/.oino/cron", access: "read", alias: "cron-root"},
  {path: "~/.oino/cron/jobs", access: "write", alias: "cron-jobs"}]` and drops `"cron": true`,
  adding `hostServices: ["cron"]`. Schedule parse/preview uses `unipi-cron-schedule` in-guest.
- Live-scheduler ops (`run`, `stop`, `pause`, `resume`, `status`, `settings-get`, `settings-set`,
  `test-notify`) call `host-service.call("cron", op, payload)`; `available("cron") == false` → the
  "no scheduler running" message. `/cron` help text and the bundled skill updated accordingly.
- Tools keep their names and approval behaviour (`cron_list` read-only; `cron_add/update/remove/run`
  write tools, unipi-port §3.24). Settings write-through (`settings.rs:116`) switches to the
  host-service op.

### 6.5 CLI surface

- **Removed from `oino`**: the whole `oino cron` subcommand (`crates/oino/src/cli/cron.rs` deleted,
  help.rs:31 line removed, `oino-cron` dep dropped, modes wiring cleaned).
- **Remains (moved)**: `unipi-host cron <cmd>` (`spec-host` owns its arg parsing; commands):
  `status list show add edit remove enable disable run stop runs log pause resume settings
  test-notify` — every op `--json`; storage ops work against the files; `run`/`stop`/`status`
  in-process against the embedded scheduler. (`serve`/`install`/`uninstall` cease to exist.)

### 6.6 Host/WIT changes

- Delete `wit/oino-extension/cron.wit` + world import + `crates/oino-wasm-host/src/imports/cron.rs`;
  delete `oino-extension-api/src/cron.rs` (`CronHost`) and the hub's `set_cron`/`cron()` +
  daemon-client fallback (hub.rs:118-157); drop `oino-cron` from `crates/oino-wasm-host/Cargo.toml`
  and `crates/oino/Cargo.toml`; delete the SDK's `oino_extension::cron` module.
- unipi-host registers the `"cron"` `HostService` (ops = the live-scheduler list above; payloads =
  today's `control.rs` shapes) — trait from WP-X1, wiring `spec-host`.

### 6.7 Files / acceptance

Files: `crates/oino-cron/**` → `apps/unipi/crates/unipi-cron/**` (+ new `apps/unipi/crates/unipi-cron-schedule/`), `crates/oino/src/{cli/cron.rs (delete), cli/mod.rs, cli/help.rs, startup.rs, Cargo.toml, lib.rs}`, `wit/oino-extension/{cron.wit (delete), worlds.wit}`, `crates/oino-wasm-host/{Cargo.toml, src/hub.rs, src/imports/{cron.rs (delete), mod.rs}, bindings.rs}`, `crates/oino-extension-api/src/{cron.rs (delete), lib.rs, runtime.rs (drop cron_call)}`, `extensions/sdk/oino-extension/src/{cron.rs (delete), lib.rs}`, `extensions/unipi/cron/{extension.json, src/{lib.rs, args.rs, tools.rs, settings.rs}, skills/cron/SKILL.md}`, tests for the job-file store + migration + watcher in `apps/unipi/crates/unipi-cron`.

Acceptance:
```sh
CARGO_BUILD_JOBS=8 cargo test -p oino-wasm-host -p oino && (cd apps/unipi && CARGO_BUILD_JOBS=8 cargo test -p unipi-cron -p unipi-cron-schedule)
oino --help | grep -v "oino cron"          # no cron line
CARGO_BUILD_JOBS=8 cargo check --workspace
cargo xtask wasm
```

---

## 7. WP-X5 — browser as a host service; extension move

### 7.1 Crates

- `crates/oino-browser` → **`unipi-browser`** and `crates/oino-browser-protocol` →
  **`unipi-browser-protocol`** (interim `apps/unipi/crates/unipi-*`; final placement `spec-core`). Both stop
  being dependencies of `oino` (`crates/oino/src/browser.rs` deleted with its `OINO_BROWSER` env
  story), `oino-wasm-host` and `oino-extension-api`. The engine becomes unipi-host's browser
  service implementation (DIRECTION §6; cloak/cookie/gate ports are `spec-host`'s).
- Wire types (`BrowserCommand`, `BrowserResult`, `BrowserStatus`, …) stay in
  `unipi-browser-protocol`; it keeps building for `wasm32-wasip2` so the extension decodes replies.
  `BrowserResult.image` bytes travel as base64 inside the JSON reply (the v1 WIT carried them
  natively; screenshots are ≤ a few hundred KB — acceptable, and images also carry `path`).

### 7.2 Extension

- Move `extensions/oino/browser` → `extensions/unipi/browser`; id **`unipi.browser`** (aligns with
  every other unipi extension and DIRECTION's "Oino carries no unipi-specific ABI"; final id/table
  confirmation with `spec-core`'s package split — flagged in §10).
- Calls become `host_service.call("browser", op, command-json, cancel)`; `status` is
  `call("browser", "status", {})` (or `available("browser")` for the register-tools gate, replacing
  `browser.status()` at `lib.rs:52-120` — same re-check on `session_start`). Approval handshake,
  turn-tail screenshots, `turn_ended`/`session_closed` lifecycle, prompt section, `/browser`:
  unchanged behaviour, new transport. When `available("browser") == false` (plain TUI): tools are
  not registered / report unavailable, prompt section absent — exactly the v1 unavailable path.
- Manifest: `"hostServices": ["browser"]` replaces `"browser": true`.

### 7.3 Host/WIT

- Delete `wit/oino-extension/browser.wit` + world import + `imports/browser.rs`; delete
  `oino-extension-api/src/browser.rs` (`BrowserHost`) and the hub's `set_browser`/`browser()`
  (hub.rs:88-131, 664-700); drop `oino-browser-protocol` from `oino-extension-api` and
  `oino-wasm-host` `Cargo.toml`s and `oino-browser` from `crates/oino/Cargo.toml`. Delete the SDK's
  `oino_extension::browser` module (replaced by a thin `host_service` wrapper the extension owns).
- unipi-host registers the `"browser"` `HostService`: op = `BrowserCommand`'s `op` tag (the 19 tool
  commands plus `status`, `turn_shot`, `reveal`, `release`, `turn_ended`, `session_closed`), reply =
  `BrowserResult` JSON; scope (session root + cwd) resolved host-side from the call context as
  today (hub.rs:147-172). Wiring is `spec-host`'s; the op/reply contract is stated here so both
  sides build against it.

### 7.4 Files / acceptance

Files: crate renames/moves + `Cargo.toml`s; `crates/oino/src/{browser.rs (delete), lib.rs, startup.rs (install call), Cargo.toml}`; `wit/oino-extension/{browser.wit (delete), worlds.wit}`; `crates/oino-wasm-host/{Cargo.toml, src/hub.rs, src/imports/{browser.rs (delete), mod.rs}, bindings.rs}`; `crates/oino-extension-api/src/{browser.rs (delete), lib.rs, runtime.rs (drop browser_execute/browser_status), Cargo.toml}`; `extensions/{oino/browser → unipi/browser}` (id, manifest, src/lib.rs, src/result.rs transport); `extensions/sdk/oino-extension/src/{browser.rs (delete), host_service.rs additions}`.

Acceptance:
```sh
CARGO_BUILD_JOBS=8 cargo check --workspace
CARGO_BUILD_JOBS=8 cargo test -p oino-extension-api -p oino-wasm-host
cargo xtask wasm
wasm-tools component wit wit/oino-extension
```

---

## 8. WP-X6 — WIT v2 cutover: version bump and compatibility statement (D7)

- Package `oino:extension@1.0.0` → **`oino:extension@2.0.0`** (every `.wit` file's package line).
- Manifest `apiVersion`: `"oino.extension/2"` (validation rejects `/1` packages with a message
  naming the rebuild); `host-info.info.api-version` reports `"2.0.0"`.
- **Breaking in 2.0.0** (vs 1.0.0): interfaces `kanboard`, `browser`, `cron` removed; interface
  `host-service` added (world imports updated); manifest permission keys `cron`, `browser` removed,
  `hostServices` added; manifest top-level `binaries` added (optional).
- **Compatibility statement**: oino 2.x hosts only `oino:extension@2.x` components — no dual-version
  hosting; a 1.x package fails validation at load with a clear diagnostic (the monorepo rebuilds
  everything; precedent for signature-driven rebuilds is deviation #23). Within 2.x the 1.x rule
  holds: additive record fields and new functions/interfaces are minor bumps; any removal or
  signature change is a major bump. Components that import a *subset* of the world keep loading
  (adding world imports is additive — relied on by WP-X1).
- WASI deps stay at 0.2.x (currently 0.2.12); no other interface changes ride along.
- Docs: this spec supersedes extension-api.md §3/§7 for the deltas above; update
  `docs/dev/v1/extension-api.md` only via a short pointer at top → this file (history stays).

Acceptance:
```sh
wasm-tools component wit wit/oino-extension          # validates @2.0.0
cargo xtask wasm && git diff --exit-code extensions/dist/checksums.json
CARGO_BUILD_JOBS=8 cargo check --workspace && cargo test --workspace -j 8
oino -p "hi" --mode print                            # loads the rebuilt bundle
scripts/docs-check.sh
```

---

## 9. WP-X7 — long-horizon transient-failure recovery (D6)

All inside `extensions/unipi/long-horizon`; no core change (it *consumes* `spec-core`'s §3.1
durability: resume-from-session, single-writer reopen, interrupted-turn repair).

### 9.1 Failure classification (pure, in `graph.rs`)

```rust
enum FailureClass { Transient { retry_after_ms: Option<u64> }, Permanent }
fn classify_failure(crashed: bool, error: &str) -> FailureClass
```

| Signal (checked in order) | Class |
|---|---|
| Worker crashed: child session marked crashed / process gone without a settle (subagents run-done outcome) | Transient, no retry-after |
| `429` \| `rate limit` \| `quota` \| `overloaded` (case-insensitive, word-ish match) | Transient; `retry_after_ms` extracted from `retry-after: <n>s`/`retry-after-ms` in the error text when present |
| HTTP/provider `5xx`: `500|502|503|504`, `internal error`, `api error`, `server error` | Transient |
| Network: `connection error`, `timed out`, `timeout`, `dns`, `econnreset`, `tls`, `network` | Transient |
| Anything else (provider 4xx, auth, context overflow, budget, unknown) | Permanent |

Regexes on the failure text only (providers already fold numeric Retry-After into error text;
providers deviation #9). No cross-crate change. Table is data-driven and unit-tested.

### 9.2 Retry state and flow

- `Work` gains persisted fields (additive to `graph.json`, default-absent): `retry_count: u32`,
  `next_retry_at: Option<i64>` (epoch ms), `failure_class: Option<String>`. `attempts` (existing)
  still counts starts; `restart_resumes` (existing, budget 1) stays **separate** — host-crash
  resumes do not consume retry budget and vice versa.
- On a failed settle (`settle(operator, "failed", …)`): classify before committing the terminal
  status.
  - **Transient** and `retry_count < 3`: the work returns to `queued` with
    `resume_operator_id = operator_id` (keep `operator_id` for the resume mapping),
    `retry_count += 1`, `next_retry_at = now + [30 s, 2 min, 10 min][retry_count-1]` — or
    `now + retry_after_ms` when the failure carried one (see the Retry-After rule below).
    Failure text is kept on the work for the supervisor view.
  - **Retry-After does not consume budget**: when the failure carried `retry_after_ms`, the wait
    uses it and `retry_count` is **not** incremented. Safety cap: cumulative honored Retry-After
    waiting per work item is bounded at **1 h**; beyond that the next Retry-After failure is treated
    as a budget-consuming retry (prevents an infinite Retry-After loop from keeping a graph alive
    forever).
  - **Permanent**, or transient with budget exhausted: `status = failed` (existing path), failure
    text prefixed with the class (`transient (429), retries exhausted` / `permanent`).
- **Retry execution**: a timer (`timers.set("lh-retry-<work_id>", delay)`) fires `oino_timer` →
  `start_work` resumes the **node's own session** through the existing `subagents.graph-run`
  resume path (`resume_operator_id`; the child cold-resumes from its log + kv — §3.1/§3.2). The
  resume instruction is the existing restart text plus: name the transient failure, continue from
  where it stopped, finish with the handoff. If the child session cannot be resumed at all
  (unrecoverable file), the retry spawns a **new** operator with the original instruction —
  once only; a second unrecoverable loss is a permanent failure.
- **Durability of the schedule**: `next_retry_at` persists in `graph.json` with every transition
  (the file already writes after each change, lib.rs:211-234). On `bind` (session start / reload):
  overdue retries fire immediately; future ones re-arm timers. In-memory timers dying with the
  process is fine — the schedule is the file (extension state rule §3.1.6).
- Concurrency: a retry never spawns while the operator session runs elsewhere; resuming triggers
  §3.1.5's single-writer takeover (the old runner is closed, the session continues here).

### 9.3 After final failure

- Downstream stays `blocked` (existing `input_state` derivation — no change).
- Supervisor wake: the failed work enters the attention set (existing `checkpoint`); the wake text
  adds: work id, class, last error, and "replace, skip or re-plan" with the `replaces=` mechanism
  (existing) — this is DIRECTION §7's "operator agent is nudged".
- User notification, once per terminal failure: `ui.fire(notify)` + `ui.os-notify` when a frontend
  is attached, and a sticky bus publish `unipi:long-horizon:work-failed` `{graphId, workId, error}`
  (lets `unipi.notify`/the app surface it later; no hard dependency).
- Scope: graph/swarm work items only. `goal` and `ralph` loops keep their existing stop/resume
  semantics (§7 says "graph failures"; loop modes are user-driven and out of scope — noted for the
  owner in §11).

### 9.4 Files / acceptance

Files: `extensions/unipi/long-horizon/src/{graph.rs (classification, retry fields/transitions, bind re-arm, checkpoint text), lib.rs (timer wiring, settle hook, notify+bus), tools.rs (no schema changes)}`, tests in-crate (`classify_failure` table tests; retry/retry-after/cap/exhaustion state-machine tests over a fake runner; bind re-arm test).

Acceptance:
```sh
CARGO_BUILD_JOBS=8 cargo test -p unipi-long-horizon   # in-tree crate name per extensions workspace
CARGO_BUILD_JOBS=8 cargo check --workspace
cargo xtask wasm
```
(Manual/e2e: a faux provider returning 429 with `retry-after: 1s` on a graph child → observe
retry_count growth without budget consumption, then recovery; a permanent 400 → immediate failed +
blocked downstream + nudge. The faux provider fixture supports error scripts today.)

---

## 10. Risks

1. **Browser JSON image size** (WP-X5): base64 screenshots through the generic JSON reply are ~33 %
   larger and copy once more. Mitigation: images already carry `path`; if it ever matters, add a
   2.x additive `call-bytes` — do not pre-add.
2. **Per-platform dist** (WP-X3): `cargo xtask wasm` must run per target; a Linux-built dist in a
   Windows installer fails validation (`bin/` missing). CI matrix already builds per target for the
   host binaries — extend, don't invent. Dev checkouts rely on `OINO_EXT_BIN_DIR`/PATH; forgetting
   it yields the (intended) "binary unavailable" message.
3. **Cron watcher latency**: notify-based reload adds ~100 ms and can miss edits while unipi-host is
   down (plain-TUI edits): covered by full rescan on lock acquisition and at startup. Fan-out
   filesystems (NFS) degrade to startup+manual `unipi-host cron status` rescan — same class of
   limitation as the kanboard watcher today.
4. **Exclusion vs `-e` replacement order** (WP-X2): filtering after in-place replacement means
   `--exclude-extension X -e <dir-of-X>` loads nothing for X. Documented as intended (exclusion
   wins); a test pins it.
5. **Retry classification by regex** (WP-X7): error texts drift across providers. Mitigation: the
   table is data-driven with broad patterns; misclassification errs either way — false-transient
   costs one wasted retry, false-permanent surfaces to the operator immediately (recoverable by
   `add_work replaces=`).
6. **`unipi.browser` id rename** (WP-X5) invalidates installed-package upgrade paths and any
   settings keyed by id (`extensionSettings`, `disabledExtensions`). Bundled packages replace in
   place by id — old `oino.browser` entries simply stop matching; acceptable at v2 (pre-release),
   but `spec-core` should confirm the final id table before the split.
7. **Board UX without SSE** (WP-X3): refresh now depends on unipi-host's watch → proto push
   (`spec-host`). If that lands late, the Board can poll `list --json`; the CLI contract here is
   stable either way.
8. **Sequencing**: WP-X3/X4/X5 each remove one WIT interface while the package is still `@1.0.0`
   (the bump is X6). Everything in the monorepo rebuilds together, so no released artifact ever
   sees the intermediate state; noted for reviewers, not users.

## 11. Owner questions

1. **Retry-After cap** (§9.2): I set a 1 h cumulative cap on honored Retry-After waits per work item
   (after which waits consume retry budget) to bound livelock. DIRECTION says only "Retry-After
   waits don't count" — confirm the cap (or name a different bound).
2. **`unipi.browser` id** (§7.2): confirm the rename from `oino.browser` (recommended for
   consistency; touches settings keys and the bundled id table `spec-core` owns).
