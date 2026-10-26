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
| Extension settings registry (schema, layered values, scoped writes, secret fields, change events) | ✅ | `oino_core::extensions::settings::ExtensionSettings` (`AgentSessionRuntime::extension_settings`); secrets in `secrets.json` at `<owner>/<namespace>/<key>`; no `oino:settings:<ns>` bus topic (core-runtime #22); `_oino/{get_settings_schema,get_extension_settings,set_extension_setting,unset_extension_setting}` ✅, TUI hub ✅ (frontend-cmds T3), desktop hub ❌ |
| Models: registry, `models.json`, auth.json, resolver, scopes, virtual models | ✅ | dynamic catalogs refreshed at startup (integration #5) |
| Providers (pi-ai wires) | 🟡 | Anthropic Messages, OpenAI Responses, chat completions, Oino Router ✅; Copilot, Codex sign-in, Azure, Bedrock, Gemini/Vertex native, Cloudflare, WebSockets ❌ (providers-v1) |
| OAuth `/login` `/logout` | ✅ | `_oino/{auth_providers,login,logout,set_api_key}` run the provider component's `oauth-login` (dialogs through the extension UI protocol) and store `auth.json`; tokens refresh through `oauth-refresh`; TUI `/login [provider]`, `/logout [provider]`; only the faux OAuth provider is tested end to end (frontend-cmds #7, #8) |
| CLI flags and help | ✅ | ACP applies `--model`/`--provider`/`--models` (frontend-cmds #12) |
| Print / JSON modes | ✅ | golden `print`, `json-mode` |
| RPC mode (pi commands, extension UI sub-protocol) | ✅ | golden `rpc`; `ctx.shutdown()` reaches core (`AgentSessionRuntime::shutdown_requested`) but no mode exits on it yet (modes #11) |
| Interactive TUI | 🟡 | full chrome, dialogs, panels, pickers, extension settings hub, backend completions (extension arguments, `complete-input`), editor events, panel actions, shortcuts, `/login`, regular (inline) mode ✅; inline images ❌ (frontend-cmds T1–T7) |
| Frontend commands (`_oino/*` completions, ui_action, editor_event, shortcuts, settings, service_call, login) | ✅ | in-process and RPC, typed `oino_client::FrontendApi`; RPC goldens `crates/oino/tests/frontend.rs`, contract test in `contract.rs` (deviations/frontend-cmds.md) |
| Package manager (`oino install/remove/update/list/config`), `oino update` | ✅ | dir / archive / https archive / `git+https` sources, checksums, `.source.json`, `--project`, `--yes`; self-update from `release-manifest.json` (frontend-cmds #13–#15); `oino auth` ❌ |
| `/share`, `/bug`, pi telemetry, pi-mcp, codemode | ❌ | out of scope (charter) |
| Extension API (pi `ExtensionAPI`) via WIT | 🟡 | events, tools, commands, shortcuts, flags, renderers, providers, virtual models, UI dialogs/fire/panels, `ui.editor-*`, `clipboard-read-image`, `edit-externally`, settings read + write (`registry.{register-settings,extension-settings,set-extension-setting}`), models.complete/stream/credential-for, nested agents (`agents.*`), turn arbiter, `control.{run-state,shutdown,system-prompt-options,new-session,fork,switch-session,reload}` ✅; provider hooks, `models.{classify,generate-image}`, `control.{bash-detach,running-bash}` ❌ |
| ACP (Zed etc.) | ✅ | protocol v1 incl. permissions, elicitation, client fs/terminal; Zed interop not yet tried by hand |
| `oino serve`, `--remote`, multi-client | ❌ | specified (modes §5) |

## 3. Parity matrix — unipi 3.0.0

All 23 ported components are bundled (load order = `unipi_kit::suite::LOAD_ORDER`). "Host" =
works only once the listed host capability exists.

| Package | Status | Missing / reason |
|---|---|---|
| core (hints, header, work tray) | ✅ | local-time hints need `host-info` UTC offset (unipi-core R3) |
| workflow (permissions, plan mode) | ✅ | gate, rules, plan mode ✅; `auto` mode's jev judge has `models.complete`; project mode and saved rules are written to the settings files (state files imported once) |
| long-horizon | ✅ | goal/ralph/swarm/graph state machines ✅; goal kickoff + continuation through the turn arbiter verified e2e (faux); graph runs use the subagents spawn path (not e2e-tested) |
| memory | 🟡 | markdown memories, tools, recall reminder, side-save pass (nested agent, memory tools only; e2e) ✅; MemPalace untested against a real install |
| dream | ✅ | digests, tray, proposals, dream runs (nested agent, `only: [unipi.memory]`, file persistence in the staging dir; e2e) ✅ |
| utility | ❌ | not ported (render styles, auto-rename, `/unipi:answer`, `/unipi:move`, image tools) |
| skill-registry | ✅ | jev judging via `models.complete`; skill states and the proxy toggle are written to the settings files (state files imported once) |
| info-screen | ✅ | panels now served by the core bridge |
| subagents | ✅ | `run_subagent` foreground/background, completion notification, `read_subagent` verified e2e; UI policy fixed at spawn (unipi-agents R2) |
| background-tasks | ✅ | notification `deliverAs: followUp` (unipi-agents R1) open |
| btw | ✅ | branch-copy aside, never reaches the session (e2e) |
| web-api | ✅ | TLS fingerprinting not implemented (`impersonated: false`) |
| ask-user | ✅ | |
| mcp | 🟡 | stdio + HTTP ✅; pi `mcp.json` not readable by the guest (R1); ACP `mcpServers` not forwarded (R3) |
| notify | ✅ | setup wizards and `/unipi:notify-event` write the global settings file, tokens go to secret fields (the old overlay is imported once) |
| kanboard | 🟡 | CLI/daemon + panel ✅; core now calls its nudge provider (arbiter); monitor nudges not e2e-tested against a live `oino-kanboard` board; installers ship `oino-kanboard` |
| command-enchantment | ✅ | `complete-input` groups merged by `_oino/get_completions` (TUI ✅, desktop ❌) |
| compactor | ✅ | `llm` method's previous-summary handling (unipi-context R1) open |
| footer | ✅ | `/unipi:footer on\|off` writes `footer.enabled` (old override imported once) |
| updater | ✅ | never installs (by design) |
| input-shortcuts | 🟡 | TUI sends `editor_event` ✅; desktop does not |
| fusion | ✅ | persistent sidekick child (file under `/ext-session/<sid>/sidekick`), handoff via `sidekick` verified e2e |
| watchdog | 🟡 | stuck detection + session caps ✅; process stats Linux-only |
| app-bridge | ✅ | control-centre panel; `_oino/service_call` exists, GPUI native widgets do not use it yet (R7) |

Not ported by decision (D22): `/unipi:cron`, browser skills/state, app push encryption,
Herdr, the unipi desktop app.

## 4. Known issues

1. Nested agents: `UiPolicy` is fixed at spawn (no `set-ui-policy`, unipi-agents R2); child
   depth is not visible on `call-context` (R3); `oino_agent` events are delivered only to the
   spawning extension (deviations/core-runtime #15–#20).
2. Turn arbiter: frontends get `run_state{waiting}` but `ctx.shutdown()` is not acted on by any
   mode yet; `control.bash-detach` / `running-bash` remain unsupported (watchdog kill,
   background-tasks adopt).
3. The desktop app does not render the extension settings hub yet (the `_oino/*` settings
   commands and the TUI hub exist). Only the rebuilt dists (core, workflow, skill-registry,
   notify, footer, kanboard, utility) carry the new `unipi_kit` settings store; other
   components pick it up on rebuild.
4. The desktop app does not use the `_oino/*` frontend commands yet (`oino_client::FrontendApi`):
   its panel buttons, shortcuts, completions and `/login` do nothing (TUI and RPC ✅).
5. OAuth sign-in is verified only with the faux provider; the real Anthropic / Copilot /
   Codex / ChatGPT flows were not run against live accounts.
6. The first start after an update compiles ~24 MB of WASM (≈ 7 s on 32 cores, longer on
   laptops); later starts use `~/.oino/cache/wasmtime` (≈ 0.3–1 s).
7. `models.classify` and `models.generate-image` are unsupported in the core bridge; TUI inline
   images are placeholders.
8. `/tmp` quota on the dev machine: run builds/tests with `TMPDIR=$PWD/target/tmp`.
9. Mobile is an early preview (embedded core on Android; iOS host not built remotely).

## 5. Next steps

1. **core-runtime**: bash detach (`control.bash-detach`, `running-bash`, `tasks.adopt-bash`:
   keep the `BashDetachRegistry` of the session's built-in tools and hand the process to the
   task service); `agents.agent.set-ui-policy` (R2). **modes**: exit on
   `AgentSessionRuntime::shutdown_requested()` once idle (print/RPC/TUI/ACP).
2. **gpui-desktop**: render the extension settings hub over `FrontendApi::settings_schema` /
   `extension_settings` / `set_extension_setting` (model: `oino_view::settings_hub`; the TUI hub
   is done). **tui-client**: core settings through `_oino/set_setting` instead of `LocalHost`'s
   own `SettingsManager` (its writes are not seen by the running session until restart).
3. **modes**: `oino serve` + `--remote` (the `_oino/*` frontend commands, package manager,
   `oino update` and ACP `--model` are done; a remote TUI still needs `_oino/list_sessions`,
   `_oino/get_keybindings`, `_oino/render` and core settings over RPC). **gpui-desktop**: wire
   `oino_client::FrontendApi` (deviations/frontend-cmds.md requests).
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
