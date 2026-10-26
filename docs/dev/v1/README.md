# Oino v1.alpha specifications

Status: **binding** (work id `spec`, 2026-10-26). Branch `v1-alpha`.
Contract hierarchy: [CHARTER.md](CHARTER.md) > these specs > audits/research notes.
Audits ([audit-pi](audit-pi.md), [audit-unipi](audit-unipi.md), [audit-oino](audit-oino.md))
and research ([research-acp](research-acp.md), [research-gpui](research-gpui.md)) are
*reference*; where a spec and an audit differ, the spec wins. Where a spec is silent on a
pi behaviour, pi 1.4.0 behaviour (as described in audit-pi) is binding.

## 1. Index

| File | Scope | Primary consumers |
|---|---|---|
| [architecture.md](architecture.md) | Workspaces, crates, dependency rules, process model, embedding | everyone |
| [session-v3.md](session-v3.md) | `oino-ai` message types, session v3 entries, migration, projection, `SessionManager` | core-types-session, core-runtime, modes, clients |
| [runtime.md](runtime.md) | Agent loop, `AgentSession`, runtime, events, tools, resources, settings, compaction, retry, models/auth, virtual models | core-runtime |
| [extension-api.md](extension-api.md) | WIT `oino:extension@1.0.0`, host capabilities, permissions, manifest, guest SDK, bundling | wit-v1, providers-v1, unipi-* |
| [ui-protocol.md](ui-protocol.md) | Frontend-agnostic extension UI: dialogs, notify, status, frame, widgets, panels, renderers, editor, shortcuts | core-runtime, modes, tui-client, gpui-*, acp, unipi-* |
| [modes.md](modes.md) | CLI, print, JSON, RPC (pi-compatible + `_oino/*`), `oino serve`, ACP mapping | modes, acp |
| [unipi-port.md](unipi-port.md) | Per-package port plan, ids, capabilities, batches, replaced built-ins | unipi-* |
| [gpui-app.md](gpui-app.md) | Desktop + mobile apps, crates under `apps/`, UX, build matrix | gpui-desktop, gpui-mobile |
| [quality.md](quality.md) | Gates, test strategy, golden tests, fixtures, CI | everyone |

## 2. Decision register

Every open question raised by the audits is resolved here. Detail lives in the linked spec.

| # | Decision | Justification | Spec |
|---|---|---|---|
| D1 | **Legacy freeze.** In phase 1, all current crates move to `legacy/crates/*` and current extension crates/packages to `legacy/extensions/*`; `legacy/` is excluded from every workspace and deleted in phase 5. | Teams build the new layout without fighting legacy compile errors; legacy stays readable for porting (TUI rendering, wasm host, provider kit). | architecture §8 |
| D2 | Rust **1.99.0** pinned in `rust-toolchain.toml` (`targets = ["wasm32-wasip2"]`, components `rustfmt`, `clippy`); **edition 2024** for every crate. | The active toolchain lacks `wasm32-wasip2` (audit-oino §2); one pinned toolchain for host and guests. | quality §2 |
| D3 | **Three cargo workspaces**: root (`crates/*`, `xtask`), `extensions/` (`wasm32-wasip2`), `apps/` (GPUI). Member lists use globs. | Core gates stay fast and GPUI-free; the gpui-pre pin and mobile crate types are isolated; globs avoid shared-file edits. | architecture §1 |
| D4 | Contract crates first: `oino-ai`, `oino-session`, `oino-protocol` (owner core-types-session) and `oino-extension-api` (owner wit-v1), written verbatim from these specs. | Parallel teams compile against stable contracts instead of each other's work in progress. | architecture §2 |
| D5 | At the WIT boundary, **pi data-model values travel as canonical JSON** (`type json = string`) with exact pi shapes; host-capability calls (process, http, fs, timers, bus, agents, registration) are **typed WIT**. Guests decode JSON with the same `oino-ai`/`oino-protocol` serde types as the host. | Pi parity is defined by JSON shapes; duplicating ~200 pi types in WIT would drift. Typed WIT stays where it pays (small, stable, security-relevant). | extension-api §1 |
| D6 | WIT v1 uses **component-model async** (`async func`, `stream<T>`, `future<T>`) on wasmtime `=48.0.2`, wit-bindgen `=0.62.0`. Multiple guest tasks may be in flight per instance (no per-extension call mutex). | Long tools (`ask_user`, `read_subagent` 600 s, `sidekick`) must not block that extension's event handlers. WASI 0.3 async ABI shipped 2026-06. | extension-api §3 |
| D7 | All guest outbound HTTP goes through `oino:extension/http` (no `wasi:http` import). Host enforces per-extension host allowlists, proxy, timeouts, credential injection, and optional TLS-fingerprint impersonation (`wreq`, cargo feature `fingerprint`, on for desktop targets, off for mobile). | One choke point for egress policy, `credential-for`, proxy settings and web-api smart-fetch. | extension-api §5.6 |
| D8 | **Subagents, btw, fusion sidekick, memory side-save, dream and rename sessions run in-process** as nested `AgentSession`s via the host `agents` interface. No child `oino` processes in v1. | Speed, shared provider components and credentials, exact usage accounting, works on every frontend. Dream no longer outlives the process: an interrupted dream run resumes on next start. | runtime §12, unipi-port §3 |
| D9 | The unipi **turn arbiter is a host feature** (guest exports `propose-nudge`, `wait-reason`), applied after normal `agent_before_settle` handlers. | Removes cross-component singletons; pi composition of `agent_before_settle` is preserved because handlers still run first. | runtime §7.6 |
| D10 | Extension state lives at `~/.oino/state/<ext-id>/{global,workspace/<wsid>,session/<sid>}`; `wsid` = first 16 hex of SHA-256 of the canonical project root (git toplevel, else cwd). **No marker file is written** into user projects. | Pi never writes into the project implicitly; cwd-keyed layout matches sessions. `/unipi:move` re-keys via move handlers. | extension-api §5.8 |
| D11 | Extension settings live in `settings.json` under `extensionSettings.<namespace>` (global and project, deep-merged, project wins). Secrets live in `~/.oino/secrets.json` (mode 0600), never in settings. | One settings manager with locking, migrations, trust gating; secrets separated as in pi `auth.json`. | runtime §10, extension-api §5.9 |
| D12 | Env vars use `OINO_*` (`OINO_AGENT_DIR`, `OINO_SESSION_DIR`, `OINO_OFFLINE`, `OINO_CACHE_RETENTION`). Tool processes get `OINO_SESSION_ID/FILE/PROVIDER/MODEL/REASONING_LEVEL` **and identical `PI_*` aliases**. | Product identity plus compatibility with skills written for pi. | runtime §9.5 |
| D13 | Pi strings that name the product are substituted (`pi` → `Oino`, pi docs paths → Oino docs). All other strings stay byte-identical. | Parity with honest branding; substitutions are enumerated in runtime §9 and tested. | runtime §9 |
| D14 | The `oino` binary and Oino Desktop **bundle** all provider and unipi components via `crates/oino-bundle`; resources are extracted once to `~/.oino/bundled/<id>@<version>-<hash8>/`. | Charter: installing Oino is enough. Skills need on-disk paths for the `read` tool. | extension-api §8 |
| D15 | Manifests declare `kind: "provider" \| "extension"`. `-ne/--no-extensions` disables `kind:"extension"` only. | Providers are extensions in Oino but must survive pi's `-ne` semantics. | extension-api §7 |
| D16 | Remote access: `oino serve` speaks Oino RPC (pi RPC + session routing + `_oino/*`) as JSONL over TCP+TLS with token pairing; multiple clients may attach to one live session (first UI answer wins). **Mobile is remote-only in v1.** | Replaces unipi app-bridge; iOS cannot JIT wasmtime; one protocol for every client. | modes §5, gpui-app §4 |
| D17 | GPUI: `gpui-kit =0.8.0` with its `gpui-pre =0.3.9` pin; mobile platform vendored at `apps/vendor/gpui-mobile` (Apache-2.0 option) and patched to `gpui-pre =0.3.9`. | One GPUI version across desktop and mobile is mandatory (research-gpui §1). | gpui-app §2 |
| D18 | No core permission gate (pi parity). Tool approval is a typed UI dialog (`approval`) that extensions (workflow) raise from `tool_call`; ACP maps it to `session/request_permission`. | Pi has no gate; unipi workflow is the policy; ACP gets native permission UX. | ui-protocol §4.3, modes §6.4 |
| D19 | Built-in tools include pi's inactive `grep`, `find`, `ls` (and `powershell` on Windows). | Replaces Oino `search`; needed for `--tools` parity and btw/explore subagents. | runtime §8 |
| D20 | `SessionManager` uses synchronous std I/O (append-only line writes, like pi `appendFileSync`); async callers use `spawn_blocking` for `list`/`list_all`. | Small writes, deterministic ordering, usable from clients without a runtime. | session-v3 §6 |
| D21 | Legacy Oino session files (`~/.oino/sessions/<uuid>.jsonl`, `record`-tagged v1/v2) migrate to v3 on first access into `--<cwd>--/<ts>_<uuid>.jsonl`; originals move to `~/.oino/sessions/.legacy/`. | Charter: migrate on load; keeps a recovery copy. | session-v3 §5 |
| D22 | Unipi scope cuts: drop `/unipi:cron`, `cron`/`browser` skills, browser-state context, app push encryption, Herdr integration, app-bridge, umbrella. jev stays (fail-open). Budget caps (cost, wall time, consecutive tool errors) move into **watchdog** as opt-in session caps with flags. MCP is ported as the unipi `mcp` extension. Updater targets Oino releases. Kanboard board renders as a panel over the daemon JSON API (no web UI). | Backends for the dropped features are the out-of-scope unipi app host; the rest keeps parity. | unipi-port §2 |
| D23 | Frontend rendering of extension UI uses the **Oino UI Spec (OUS) v1** node vocabulary (ui-protocol §6); pi `rpc-extension-ui` records are kept verbatim and extended, never changed. | Charter: frontend-agnostic, modelled on rpc-extension-ui. | ui-protocol |
| D24 | Spec changes after this commit go to `docs/dev/v1/deviations/<work-id>.md` (owned by that work item), never into these files. Integration folds them in at phase 5. | Specs stay stable while teams work concurrently. | §5 |
| D25 | `serde_json` is used with `preserve_order` everywhere; golden comparisons are parsed-JSON equality (key order free) except where pi semantics depend on order (`sections`, `extensionSettings` order irrelevant). | Pi system `sections` order is significant; byte-equality on key order is not required elsewhere. | quality §4 |

## 3. Phase plan

Phases gate on the *contract* each later item needs, not on whole items being finished.
Items inside a phase run in parallel. "Ready" conditions are checked by the graph owner.

| Phase | Work items (parallel) | Ready when |
|---|---|---|
| **P0 spec** | `spec` (this) | committed |
| **P1 contracts** | `integration` (legacy freeze D1, workspaces, toolchain, xtask skeleton, CI skeleton); `core-types-session` (`oino-ai`, `oino-session`, `oino-protocol`); `wit-v1` step 1 (`wit/oino-extension/*.wit` + `oino-extension-api` crate exactly as specced) | contract crates compile, have docs and tests; `wit/` validates with `wasm-tools component wit` |
| **P2 engine** | `core-runtime`; `wit-v1` step 2 (wasm host, guest SDK, fixtures); `modes` (RPC codec, `oino-client`, CLI, print/json, against a fake session until core lands); `tui-client` (against `oino-client` fake backend); `providers-v1` (against WIT + SDK) | `oino -p "hi"` works end-to-end with a provider component from `extensions/dist` (`-e`); JSON/RPC golden suites green with the faux provider |
| **P3 extensions & clients** | `unipi-core` (B0 kit + B1); `unipi-ux`; `unipi-integrations`; `unipi-agents`; `unipi-context`; `unipi-kanboard`; `acp`; `gpui-desktop`; `gpui-mobile` | per-item acceptance in its spec |
| **P4 integration** | `integration` (bundle all components, golden e2e vs pi, installers, docs rewrite, release matrix); stragglers from P3 | quality §6 release checklist green |
| **P5 cleanup** | `integration` (delete `legacy/`, fold deviations into specs) | — |

Batch order *within* P3 for unipi follows [unipi-port.md §4](unipi-port.md): B1 leaf packages
first; B2 (agents) needs host `agents`/`process`; B3 needs B2 subagents for long-horizon; B4
presentation last. unipi-ux and unipi-integrations start immediately in P3.

## 4. Work ownership map

A work item may create, edit and delete files **only** under its paths. Shared files and
their rules are in §4.2. Anything else is a "Request for other work" in the item's report.

### 4.1 Paths per work item

| Work item | Owns | Produces (contract for others) |
|---|---|---|
| `core-types-session` | `crates/oino-ai/`, `crates/oino-session/`, `crates/oino-protocol/` | message/model/stream types; session v3 + `SessionManager`; client events, RPC records, UI protocol and extension event payload types |
| `core-runtime` | `crates/oino-agent/`, `crates/oino-tools/`, `crates/oino-resources/`, `crates/oino-settings/`, `crates/oino-models/`, `crates/oino-core/` | `AgentSession`, `AgentSessionRuntime`, `Services`, `HostServices` impl, extension runner, compaction, export |
| `wit-v1` | `wit/`, `crates/oino-extension-api/`, `crates/oino-wasm-host/`, `extensions/sdk/oino-extension/`, `extensions/sdk/oino-extension-macros/`, `extensions/fixtures/` | WIT package, runtime traits, wasmtime host, guest SDK, fixture components |
| `modes` | `crates/oino-rpc/`, `crates/oino-client/`, `crates/oino/` (bin: CLI, print, JSON, mode dispatch, `serve`, `update`, package subcommands) | RPC server/codec/transports, typed RPC client, `SessionBackend` |
| `tui-client` | `crates/oino-tui/`, `crates/oino-view/` | interactive terminal mode; frontend-agnostic view models (transcript rows, UI state reducer) |
| `providers-v1` | `extensions/sdk/oino-provider-kit/`, `extensions/providers/` (`openai-compat`, `openai-responses`, `anthropic`, `router`) | provider components + catalogs |
| `unipi-core` | `extensions/sdk/unipi-kit/`, `extensions/unipi/core/`, `extensions/unipi/utility/` | shared guest kit, hints, work-tray model, utility |
| `unipi-ux` | `extensions/unipi/{ask-user,command-enchantment,input-shortcuts,footer,info-screen}/` | — |
| `unipi-agents` | `extensions/unipi/{background-tasks,watchdog,subagents,fusion,btw,long-horizon}/` | `read_subagent`, graph-run service, bg service topics |
| `unipi-context` | `extensions/unipi/{compactor,memory,dream,skill-registry,workflow}/` | — |
| `unipi-integrations` | `extensions/unipi/{web-api,notify,mcp,updater}/` | — |
| `unipi-kanboard` | `crates/oino-kanboard/`, `extensions/unipi/kanboard/` | `oino-kanboard` lib + bin |
| `acp` | `crates/oino-acp/` | `oino --mode acp` adapter |
| `gpui-desktop` | `apps/Cargo.toml`, `apps/Cargo.lock`, `apps/oino-ui/` (except `src/mobile/`), `apps/oino-desktop/` | shared GPUI views, desktop app + installers |
| `gpui-mobile` | `apps/oino-mobile/` (incl. `ios/`, `android/`), `apps/vendor/gpui-mobile/`, `apps/oino-ui/src/mobile/` | mobile app |
| `integration` | `Cargo.toml` (root, except §4.2 rule), `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`, `deny.toml`, `xtask/`, `crates/oino-bundle/`, `extensions/Cargo.toml`, `extensions/dist/` (generated), `scripts/`, `.github/`, `tests/e2e/`, `docs/` except `docs/dev/v1/`, `legacy/`, `README.md`, `AGENT.md`, `examples/` | workspaces, bundle, CI, golden e2e harness, pi oracle, docs, releases |
| `spec` | `docs/dev/v1/*.md` (this set; frozen after commit) | — |

Every work item additionally owns `docs/dev/v1/deviations/<work-id>.md` and
`scripts/pi-oracle/<work-id>/` (fixture generators it needs).

### 4.2 Shared-file rules

| File | Rule |
|---|---|
| root `Cargo.toml` `[workspace.dependencies]` | Any item may **append** a dependency line (keep alphabetical order, exact version policy in quality §2). No other edits. |
| `Cargo.lock`, `extensions/Cargo.lock` | Any item may stage it together with the manifests that changed it. |
| `extensions/Cargo.toml` | Members are globs (`sdk/*`, `providers/*`, `unipi/*`, `fixtures/*`); no edits needed. `[workspace.dependencies]` follows the root rule. |
| `apps/Cargo.toml` | Owned by gpui-desktop; gpui-mobile appends its members/deps (append-only). |
| `docs/dev/v1/*.md` | Frozen. Use `deviations/<work-id>.md`. |
| `xtask/` | Owned by integration. The `desktop` and `mobile` subcommands only delegate to `apps/oino-desktop/scripts/build.sh` and `apps/oino-mobile/scripts/build.sh` (owned by the gpui items), passing arguments through. |

### 4.3 Dependency of work items on contracts

```text
core-types-session ──► core-runtime ──► modes ──► acp
        │                  ▲  │           │
        │      wit-v1 ─────┘  │           ├──► tui-client
        │        │            │           └──► gpui-desktop ──► gpui-mobile
        │        ├──► providers-v1
        │        └──► unipi-core ──► unipi-{ux,integrations,agents,context,kanboard}
        └──────────────────────────► everyone (types)
integration: wraps all (P1 skeleton, P4 bundle/e2e, P5 cleanup)
```

## 5. Rules of engagement (in addition to the charter)

1. Implement the spec; if reality forces a change, record it in your
   `deviations/<work-id>.md` with: section, change, reason, impact on other items.
2. A contract crate change after P1 (any public item in `oino-ai`, `oino-session`,
   `oino-protocol`, `oino-extension-api`, `wit/`) is additive only (new optional fields, new
   enum variants marked `#[non_exhaustive]`). Breaking contract changes require a deviation
   entry **and** a note to the graph owner before commit.
3. Commit prefix = work id (`[core-runtime] …`). Stage explicit paths.
4. Each item's final report lists: crates/dirs touched, gates run (quality §1), open
   deviations, requests for other work.
5. **Fold (1.0.0-alpha.2, integration-2).** Settled deviation rows now live in an appendix
   "Settled deviations" at the end of the spec they amend (architecture, session-v3, runtime,
   extension-api, ui-protocol, modes, unipi-port, gpui-app). Each `deviations/<work-id>.md`
   keeps only open items and requests; new deviations still go there first.
