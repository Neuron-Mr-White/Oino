# Deviations: unipi-kanboard

Work item `unipi-kanboard` (unipi-port §3.16). Owned paths: `crates/oino-kanboard/`,
`extensions/unipi/kanboard/`, this file. One shared line was touched: the root `Cargo.toml`
`members` gained `"crates/oino-kanboard"`; integration owns that file.

## Crate `oino-kanboard` (from unipi `crates/kanboard`)

| # | unipi | Oino | Why |
|---|---|---|---|
| K1 | binary `unipi-kanboard`, lib `kanboard` | binary `oino-kanboard`, lib `oino_kanboard`, version `1.0.0-alpha.0` | spec |
| K2 | env `UNIPI_KANBOARD_*` (HOME, PROJECT, ACTOR, SESSION, PID, MAX_SESSIONS, CHAIN_GATE, CHILD, UNDO_TTL_SECS, NO_WATCH, DEBUG_WATCH, ARCHIVE_AFTER_DAYS, RETENTION_DAYS) | the same names with the `OINO_` prefix | Oino naming; no reading of the unipi names |
| K3 | home `~/.unipi/kanboard` | `<agent dir>/state/unipi.kanboard` (agent dir is `$OINO_AGENT_DIR`, else `~/.oino`) | spec. There is no import of old boards, because charter/D10 import only memory |
| K4 | the daemon serves the embedded web UI (`ui-dist`, rust-embed, `build.rs`, `web/`) | removed. `/` and board/asset paths return 404, and `/api/health` no longer has `ui` | D22. The board is the panel in the extension |
| K5 | pi-side settings `~/.unipi/config/kanboard/config.json` | `<agent dir>/settings.json` → `extensionSettings.kanboard` (merge-write, other keys kept). `load/patch_pi_settings_at` are also exposed for tests | Oino settings layout (D11) |
| K6 | summaries run `piCommand` (pi argv) | the extension writes `piCommand = ["oino"]`. When the head is a bare `oino`, the binary uses the `oino` installed next to it (`settings::resolve_program`). Messages say "Oino" | the guest cannot know the Oino executable path |
| K7 | model list: unipi `~/.unipi/config/models-cache.json`, then `piCommand --list-models` | only `oino --list-models` (the cache file does not exist in Oino) | — |
| K8 | `fs2` locks, `libc kill` via `unsafe extern` | std `File::lock/try_lock`; `nix` signals | charter: `unsafe_code = forbid`, fewer dependencies |
| K9 | `serde_norway` | `serde_yaml_ng` (workspace dependency, same API for the one scalar unquote) | workspace dependency policy |
| K10 | — | `status` also reports `binary` (absolute path, from `current_exe`) and `hostname` | the guest needs the absolute path for the agent's CLI prefix, and has no OS host name or interface API |
| K11 | — | `add --attach-existing <F>` (hidden, repeatable): attaches only files that exist, and expands `~/` | unipi checked host paths with `existsSync`, which a guest cannot do |
| K12 | integration tests that clear ambient env with `unsafe remove_var`, the HOME `set_var` settings test, the embedded-UI test, and the models-cache unit test | env clearing dropped (the variables are renamed, so the ambient shell does not set them); settings test uses an explicit path; UI test replaced by `the_daemon_serves_no_web_ui`; auth tests target `/api/projects` | `unsafe` forbidden; web UI removed |

## Extension `unipi.kanboard` (from unipi `packages/kanboard`)

| # | unipi | Oino | Why |
|---|---|---|---|
| E1 | `process.env.UNIPI_KANBOARD_{SESSION,PID,PROJECT,MAX_SESSIONS}` is inherited by the agent's bash | the `tool_call` handler puts `export OINO_KANBOARD_SESSION=… OINO_KANBOARD_PID=… OINO_KANBOARD_MAX_SESSIONS=… OINO_KANBOARD_PROJECT=…` (PowerShell: `$env:…`) in front of every bash/powershell command that calls the binary, via the returned `input` | guests cannot change the host environment. Only kanboard commands are touched |
| E2 | session id `pi-<sessionId>`, claim pid = pi pid | `oino-<sessionId>`, pid = `process.self-pid()` (the Oino process) | — |
| E3 | binary search: `UNIPI_KANBOARD_BIN` → npm platform package → dev build | `$OINO_KANBOARD_BIN` (permission `env`) → `oino-kanboard` on the host search path. After the first `status`, the reported absolute path is cached and handed to the agent | D22 (`kanboard-bin` replaced by the installer) |
| E4 | slug cached with `setSettings("kanboard", {slug, root}, "project")` on onboard | settings `slug` if set, else an in-memory cache, else the binary resolves the project for the cwd/git root (`project show`) | guests cannot write settings (unipi-core R1). The binary's own resolution already does this |
| E5 | `SettleInfo` from the unipi arbiter | built in the guest: tool calls are counted between `agent_start` and settle, and the last assistant text and tool calls come from `agent_end.messages` | the host `propose-nudge` has no settle info |
| E6 | nudge `details.unipiHarness` | `details.oinoProvenance {source, title, synopsis, severity?, delivery}`; delivery is counted on `nudge-delivered(id)` | runtime §2 provenance |
| E7 | web board + `openBrowser` | `/unipi:kanboard open` starts the daemon, prints the API URLs and opens the board **panel** (`sidePane`: lane lists in a `split`, a detail section with allowed-move buttons, a comment prompt when a move needs one, keys r/a/s), live from SSE `/events`. `openBrowser` is kept as a stored setting (label "Open board panel") | D22 |
| E8 | wildcard bind lists every reachable IPv4 (`os.networkInterfaces`) | lists the hostname (from `status`) and the tailnet address | no interface API in guests |
| E9 | the in-chat board renderer budgets widths in cells | OUS `text` rows with `wrap: false`; the frontend truncates to the width | ui-protocol: frontends own layout |
| E10 | `/unipi:kanboard status`, `doctor` and `open` run in the background (`void work()`) | awaited inside the command | host calls need a live call context |
| E11 | doctor checks that `piCommand[0]` exists on disk | reports the argv; the daemon resolves it (K6) | the guest cannot stat host paths |
| E12 | R1 annotation `details.unipiHarnessAnnotations[].meta` from core `harnessMetadata` | same shape, with ids generated in the guest | — |
| E13 | debug log `~/.unipi/logs/kanboard.log` behind `UNIPI_DEBUG_KANBOARD=1` | dropped (failures stay silent or go to `notify`, as in unipi) | unipi-port §2.3 extension log |
| E14 | `captureToBacklog({cwd, text})` on `globalThis` | service `kanboard.capture{text\|body, title?, priority?}` → `{ok, id, attachments}` / `{ok: false, reason}` | unipi-port §2.2 |
| E15 | move handler also rewrote the project settings cache | `project rebind` through the binary, then the in-memory slug cache is updated; the result goes back as `MoveReport.log` lines | settings not writable (E4) |
| E16 | permissions in the spec: `process.spawn (oino-kanboard, git)`, `fs.project: read` | `process.allow: [oino-kanboard, tailscale]` and `detached`, `network.hosts: [127.0.0.1, localhost]`, `env: [OINO_KANBOARD_BIN, OINO_KANBOARD_CHILD, OINO_KANBOARD_HOME]`, `sessionControl`. No `git` and no `fs.project` | minimal: neither is used; `tailscale` is needed for `--host tailscale` |

## Requests

- **RK1 (core-runtime / installer, integration):** put the Oino install directory on the
  program search path for extension `process.exec`/spawn (unipi-port §3.16 assumes it). Also
  ship `oino-kanboard` next to `oino` in the installers and bundles.
- **RK2 (core-runtime):** implement the turn arbiter (`propose-nudge` / `nudge-delivered`).
  `oino-core` does not call it yet, so the kanboard monitor is reachable only from tests.
- **RK3 (wit-v1):** add a guest way to write extension settings (same as unipi-core R1). It
  would let onboarding persist `slug` the way unipi did (E4).
- **RK4 (integration):** add `crates/oino-kanboard` to the root workspace lints when they are
  switched on (it mirrors quality.md §2 locally, plus a crate `clippy.toml`). Run
  `extensions/unipi/kanboard/harness/run.sh` in CI, and regenerate `extensions/dist/checksums.json`.
- **RK5 (frontends):** render the `sidePane` panel with a horizontal `split` of `section{list}`
  columns. `list.selectable` must send `extension_ui_action{actionId: "select", value: id}`.
