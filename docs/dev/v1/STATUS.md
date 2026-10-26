# Oino v1.0.0-alpha.1 — status

Date: 2026-10-26 · branch `v1-alpha` · work id `integration`.
Parity targets: `@earendil-works/pi-coding-agent` **1.4.0** and unipi **3.0.0**.
Legend: ✅ done · 🟡 partial · ❌ missing. Details per item live in
[deviations/](deviations/) (one file per work item, plus [integration.md](deviations/integration.md)).

## 1. Gates (this commit)

| Workspace | fmt | clippy `-D warnings -W clippy::pedantic` | tests | other |
|---|---|---|---|---|
| root (`crates/*`, `xtask`) | ✅ | ✅ | ✅ (`cargo test --workspace`, incl. bundled e2e `crates/oino/tests/bundled.rs`) | ✅ rustdoc `-D warnings`; `#![forbid(unsafe_code)]` everywhere except `oino-wasm-host` (`deny` + one `allow` on the `bindgen!` module, wit-v1 #12) |
| `extensions/` (wasm32-wasip2) | ✅ | ✅ (wasm32 target) | ✅ native tests; ✅ the six `harness/run.sh` wasm-host suites (run by their owners; CI runs them) | ✅ `cargo xtask wasm` + `cargo xtask checksums` (31 components) |
| `apps/` (GPUI) | ✅ | ✅ | ✅ | — |

End-to-end smoke (release `oino`, temp agent dir):

- faux provider package (`-e extensions/dist/oino.fixture.provider-faux`) with the full bundle:
  print ✅, JSON ✅ (unipi tools and sections in the system prompt), RPC ✅ (`prompt`,
  `get_commands` lists 43 unipi commands), ACP ✅ (`initialize`, `session/new`, `session/prompt`
  streamed, `end_turn`), TUI ✅ (pty: typed prompt, reply, unipi memory reminder, clean exit).
- real provider: Oino Router key from `~/.oino/auth.json` (stored as provider `oino`):
  `oino/deepseek/deepseek-flash` (chat completions) ✅ incl. a `bash` tool call;
  `oino/agy/claude-sonnet-5-5-low` (Anthropic Messages via the router) ✅.
- panels through the core bridge: RPC client with `_oino/hello` → `/unipi:info` emits
  `openPanel` / `updatePanel` ✅. `models.complete` / `summarize-default` compile and are
  wired but were not exercised end to end (no test drives jev/recap yet).
- `scripts/install-smoke.sh` (source build into a temp prefix + faux prompt) ✅.
- Oino Desktop: `apps` tests incl. `embedded_session_streams_through_an_installed_provider`
  ✅ (bundle order fix and docs root apply to the desktop too).

## 2. Parity matrix — pi 1.4.0

| Area | Status | Notes |
|---|---|---|
| Agent loop (`pi-agent-core`) | ✅ | golden vs pi; TypeBox `format` keyword not validated (core-runtime #4) |
| `AgentSession` (prompt, steer, follow-up, abort, queue modes, retry, overflow recovery) | ✅ | prompt returns at run start (core-runtime #5) |
| Session format v3, `SessionManager`, tree, labels, context edits, legacy migration | ✅ | golden suites `session-context`, `legacy-migration` |
| Compaction, branch summaries, split turns, file tracking | ✅ | golden `compaction` |
| Built-in tools `read write edit bash grep find ls` (+`powershell`) | ✅ | Windows paths untested; image re-encoding differs from Photon |
| Resources: AGENTS.md/CLAUDE.md, SYSTEM.md, APPEND_SYSTEM.md, skills, prompt templates, themes discovery, trust | ✅ | custom theme files are discovered but not rendered |
| System prompt (sections, docs section, D13 substitutions) | ✅ | docs root = bundled `oino.docs` |
| Settings manager (layers, migrations, locking) | ✅ | |
| Models: registry, `models.json`, auth.json, resolver, scopes, virtual models | ✅ | dynamic catalogs refreshed at startup (integration #5) |
| Providers (pi-ai wires) | 🟡 | Anthropic Messages, OpenAI Responses, chat completions, Oino Router ✅; Copilot, Codex sign-in, Azure, Bedrock, Gemini/Vertex native, Cloudflare, WebSockets ❌ (providers-v1) |
| OAuth `/login` `/logout` | ❌ | Anthropic OAuth exists in the component; core/frontends do not drive it |
| CLI flags and help | ✅ | `--model` ignored in ACP mode (integration #9) |
| Print / JSON modes | ✅ | golden `print`, `json-mode` |
| RPC mode (pi commands, extension UI sub-protocol) | ✅ | golden `rpc`; `ctx.shutdown()` reaches core (`AgentSessionRuntime::shutdown_requested`) but no mode exits on it yet (modes #11) |
| Interactive TUI | 🟡 | full chrome, dialogs, panels, pickers ✅; inline (non-alt-screen) mode, inline images, extension settings hub, extension argument completion, editor events ❌ (tui-client deviations) |
| Package manager (`oino install/remove/update/list/config`), `oino auth`, `oino update` | ❌ | install packages by copying into `~/.oino/extensions` |
| `/share`, `/bug`, pi telemetry, pi-mcp, codemode | ❌ | out of scope (charter) |
| Extension API (pi `ExtensionAPI`) via WIT | 🟡 | events, tools, commands, shortcuts, flags, renderers, providers, virtual models, UI dialogs/fire/panels, settings read, models.complete, nested agents (`agents.*`), turn arbiter, `control.{run-state,shutdown,system-prompt-options,new-session,fork,switch-session,reload}` ✅; settings **write**, provider hooks, editor read, `control.{bash-detach,running-bash}` ❌ |
| ACP (Zed etc.) | ✅ | protocol v1 incl. permissions, elicitation, client fs/terminal; Zed interop not yet tried by hand |
| `oino serve`, `--remote`, multi-client | ❌ | specified (modes §5) |

## 3. Parity matrix — unipi 3.0.0

All 23 ported components are bundled (load order = `unipi_kit::suite::LOAD_ORDER`). "Host" =
works only once the listed host capability exists.

| Package | Status | Missing / reason |
|---|---|---|
| core (hints, header, work tray) | ✅ | local-time hints need `host-info` UTC offset (unipi-core R3) |
| workflow (permissions, plan mode) | 🟡 | gate, rules, plan mode ✅; `auto` mode's jev judge now has `models.complete`; saved rules live in state files, not settings (no settings write) |
| long-horizon | ✅ | goal/ralph/swarm/graph state machines ✅; goal kickoff + continuation through the turn arbiter verified e2e (faux); graph runs use the subagents spawn path (not e2e-tested) |
| memory | 🟡 | markdown memories, tools, recall reminder, side-save pass (nested agent, memory tools only; e2e) ✅; MemPalace untested against a real install |
| dream | ✅ | digests, tray, proposals, dream runs (nested agent, `only: [unipi.memory]`, file persistence in the staging dir; e2e) ✅ |
| utility | ❌ | not ported (render styles, auto-rename, `/unipi:answer`, `/unipi:move`, image tools) |
| skill-registry | ✅ | jev judging via `models.complete`; skill-state edits stay in state files |
| info-screen | ✅ | panels now served by the core bridge |
| subagents | ✅ | `run_subagent` foreground/background, completion notification, `read_subagent` verified e2e; UI policy fixed at spawn (unipi-agents R2) |
| background-tasks | ✅ | notification `deliverAs: followUp` (unipi-agents R1) open |
| btw | ✅ | branch-copy aside, never reaches the session (e2e) |
| web-api | ✅ | TLS fingerprinting not implemented (`impersonated: false`) |
| ask-user | ✅ | |
| mcp | 🟡 | stdio + HTTP ✅; pi `mcp.json` not readable by the guest (R1); ACP `mcpServers` not forwarded (R3) |
| notify | ✅ | setup wizards write a local overlay (no settings write) |
| kanboard | 🟡 | CLI/daemon + panel ✅; core now calls its nudge provider (arbiter); monitor nudges not e2e-tested against a live `oino-kanboard` board; installers ship `oino-kanboard` |
| command-enchantment | 🟡 | completions need frontend `get_completions` (`_oino/*`) |
| compactor | ✅ | `llm` method's previous-summary handling (unipi-context R1) open |
| footer | ✅ | |
| updater | ✅ | never installs (by design) |
| input-shortcuts | 🟡 | needs `editor_event` from frontends (TUI/desktop do not send it) |
| fusion | ✅ | persistent sidekick child (file under `/ext-session/<sid>/sidekick`), handoff via `sidekick` verified e2e |
| watchdog | 🟡 | stuck detection + session caps ✅; process stats Linux-only |
| app-bridge | ✅ | control-centre panel; GPUI native widgets need `_oino/service_call` (R7) |

Not ported by decision (D22): `/unipi:cron`, browser skills/state, app push encryption,
Herdr, the unipi desktop app.

## 4. Known issues

1. Nested agents: `UiPolicy` is fixed at spawn (no `set-ui-policy`, unipi-agents R2); child
   depth is not visible on `call-context` (R3); `oino_agent` events are delivered only to the
   spawning extension (deviations/core-runtime #15–#20).
2. Turn arbiter: frontends get `run_state{waiting}` but `ctx.shutdown()` is not acted on by any
   mode yet; `control.bash-detach` / `running-bash` remain unsupported (watchdog kill,
   background-tasks adopt).
3. **No guest settings write path** (unipi-core R1 / RK3 / integrations R4 / context R2):
   extensions keep user choices in their state files; `extensionSettings` is read-only to them,
   and no frontend renders the extension settings hub.
4. Frontend `_oino/*` commands (`get_completions`, `ui_action`, `invoke_shortcut`,
   `editor_event`, settings schema, `service_call`, `login`) are missing: panel buttons in the
   desktop app and extension argument completion do nothing.
5. ACP sessions ignore `--model`/`--provider`.
6. The first start after an update compiles ~24 MB of WASM (≈ 7 s on 32 cores, longer on
   laptops); later starts use `~/.oino/cache/wasmtime` (≈ 0.3–1 s).
7. `models.stream`, `classify`, `generate-image`, `credential-for`, `ui.editor-*`,
   `clipboard-read-image`, `edit-externally` are unsupported in the core bridge.
8. `/tmp` quota on the dev machine: run builds/tests with `TMPDIR=$PWD/target/tmp`.
9. Mobile is an early preview (embedded core on Android; iOS host not built remotely).

## 5. Next steps

1. **core-runtime**: bash detach (`control.bash-detach`, `running-bash`, `tasks.adopt-bash`:
   keep the `BashDetachRegistry` of the session's built-in tools and hand the process to the
   task service); `agents.agent.set-ui-policy` (R2). **modes**: exit on
   `AgentSessionRuntime::shutdown_requested()` once idle (print/RPC/TUI/ACP).
2. **wit-v1 + core-runtime**: `registry.set-extension-setting` + `extension-settings` read,
   secret-field resolution; then the settings hub in TUI and desktop.
3. **modes**: `_oino/*` frontend commands (completions, ui actions, shortcuts, editor events,
   settings schema, service calls, login), `oino serve` + `--remote`, package manager
   subcommands, `oino update`; resolve `--model` for ACP sessions.
4. **unipi-core**: port `unipi.utility`.
5. **providers-v1**: Copilot / Codex OAuth, Azure, Bedrock, Gemini; drive Anthropic OAuth
   from `/login`.
6. **Release builds** (quality §6 `release` job, `cargo xtask remote-build <host>` to be
   written):
   - **coffee** (Windows, `ssh coffee`): `cargo xtask wasm` (or copy `extensions/dist`), then
     `cargo build --release -p oino -p oino-kanboard`, and
     `apps/oino-desktop/scripts/build.sh --release --formats nsis`.
   - **desmondgiam** (macOS): same CLI build for `aarch64-apple-darwin`,
     `apps/oino-desktop/scripts/build.sh --release --universal --formats app,dmg`, and
     `apps/oino-mobile/scripts/ios-remote.sh` for the iOS app (never run so far).
   - Then tag `v1.0.0-alpha.1` so CI assembles the GitHub release (`release-manifest.json`
     with `oino` and `oino-kanboard` per target, used by the installers).
7. **integration (P5 remainder)**: fold the deviation files into the specs; golden e2e
   suite under `tests/e2e/` with `PARITY.md` (quality §7); `cargo xtask layering`, `deny`,
   `desktop`/`mobile` delegating subcommands; criterion benches (quality §5).
