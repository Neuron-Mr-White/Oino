# Spec — unipi-host: process, lifetime, transport, protocol, host services

Status: proposed (work id `spec-host`). Scope: [DIRECTION.md](../DIRECTION.md) §2 (architecture,
host lifetime, transport, protocol), the **host side** of §4.3 (terminal, fs, git, push, forward,
sharing, pairing, updater, settings, cron firing), and the host side of §3.1.5 (session takeover).
Everything UI-rendering, WIT and browser-stealth is owned by sibling specs and only referenced:

- `specs/core.md` — oino-core durability/efficiency (session log, single-writer lock, lazy load,
  shared Engine, panic isolation), the pi-like repo/package split.
- `specs/ext-abi.md` — WIT changes, `host-service.call(name, op, json)` registry, extension
  packages of the UniPi distribution.
- `specs/ui-protocol.md` — extension UI protocol v2 (ratatui vocabulary, slots, reserved panel
  ids), the JSON node/request shapes.
- `specs/browser.md` — browser engine choice, CDP hygiene, cloak/cookies/gate ports.

Source material studied: `apps/unipi` (unipi-app 1.0.0 import, commit `12f6660`), oino crates
(`oino-core`, `oino-client`, `oino-rpc`, `oino-cron`, `oino-browser`), [modes.md](../../v1/modes.md)
(RPC command set, framing), [frontend-api.md](../../v1/frontend-api.md) (GUI backend surface),
[round6-direction.md](../../v1/round6-direction.md) (superseded transport/service parts).

---

## 1. Current state (findings, file:line)

### 1.1 unipi-app host (`apps/unipi/crates/host`, bin `unipi-host`)

- **One host per data dir**: `instance.rs` takes an OS-level exclusive lock on `<dir>/host.lock`,
  records `{pid, version, mode, started_at}` (`instance.rs:85-130`, `Mode::{Service,App,Manual}`
  at `:28`), exit code 75 on contention (`:15`), `exit_with_parent(pid)` poll + Linux
  `PR_SET_PDEATHSIG` (`:186-210`). `status()` probes without holding (`:141`).
- **Transport**: `main.rs:577-609` builds the iroh `Endpoint` (N0 preset, `secret.key`,
  ALPN `unipi/app/1`, custom `--relay` URL, `--relay-only`), `endpoint.rs` persists the ticket
  cache (`endpoint.json`); `local.rs` serves `<dir>/app.sock` (unix only, 0600, same-user uid
  check `:22`; **Windows `serve` is a no-op stub** `:73-76`); `ipc.rs` has unix/pipe stream
  helpers; `pairing.rs` (10-char Crockford code, TTL, single-use, lockout), `pair_link.rs`
  (ticket + `unipi://pair?t=&c=` link + terminal QR), `devices.rs` (allowlist with revoke,
  last_seen).
- **Protocol**: `protocol.rs:72-90` — iroh `ProtocolHandler::accept` spawns one task per
  **bi-stream**; `Handler::handle` (`:120`) reads exactly one `Hello{proto, req}` control frame,
  applies pairing/allowlist/local-only-admin gates (`:150-215`), dispatches the `Request`, writes
  `Ok{data}`/`Err{AppErrorDto}`, shuts down. Framing `proto/src/frame.rs`: `u8 kind · u32BE len
  · payload`, kinds 0=JSON control / 1=output bytes / 2=input bytes, max 1 MiB, incremental
  `Decoder`. Version gate: major must match (`protocol.rs:135-141`), `PROTO_VERSION = "1.20.0"`
  (`proto/src/version.rs:6`).
- **Ops** (`proto/src/control.rs:942-1146`, dispatch `host/src/protocol.rs`): `list/tree/
  shortcuts/preview/attach/shell/mux_action` (herdr/tmux), `board_http/board_sse` (kanboard
  daemon relay), `chat_list/chat_open/chat_new/chat_close/chat_config(_set)` (pi app-bridge
  relay via `~/.unipi/bridge/<pid>.{json,sock}`, `chat.rs:1-40`), `blob_put/blob_get`,
  `fs_roots/list/read/write/stat/search/mkdir/rename/import/delete` (+`fs_scope.rs` allow/deny,
  `outside` escape hatch), `git{path,cmd}` (`git.rs`, `GitCmd` = repo/status/diff/stage/unstage/
  discard/commit/log/branches/switch/create_branch/pull/push/fetch/stash/pop/amend etc.,
  `proto/src/git.rs:150+`), `notify_config/notify_register/notify_test` (ntfy E2E key
  `notify.rs`), `ports_list/tcp_forward` (`forward.rs`), `push_list{wait}/push_done`
  (`push.rs`, long-poll `PushHub`, local `control.sock` for the CLI `push.rs:190-218`),
  `browser_status/open/close/window/profiles/takeover` (`browser/`, 2401-line `mod.rs`),
  `desktop_status/open` (`desktop/`, 12 files, openh264/x11/portal/mutter/windows capture),
  `feature_request/feature_requests/feature_leave` (owner consent, `sharing.rs`,
  `FeatureKind::{Forward,Browser,Desktop,Push,Cron}` `proto/src/sharing.rs:40`),
  `host_info/pair_begin/pair_status/devices_list/device_revoke` (local-only admin,
  `protocol.rs:194-198`), 18 `cron_*` ops (`cron_serve.rs` → `cron/scheduler.rs`,
  `CronControl` at `scheduler.rs:455`).
- **Cron**: host-owned scheduler always on in `serve` (`main.rs:596-618`), fires jobs by
  spawning `pi -p --mode json` (`cron/exec.rs:1-10,303`), notify channels, session wake links,
  `control.sock` shares it with the CLI.
- **Updater**: `update.rs` checks `https://unipi.nrn.one` releases, sha256-verified atomic
  replace, restart via systemd/LaunchAgent/logon task (`Service` enum `update.rs:255`),
  background check in serve (`main.rs:648-660`).
- **Serve loop extras**: tmux orphan sweep, browser agent endpoint, sharing sweeper,
  shutdown-signal cleanup (`main.rs:611-677`).

### 1.2 unipi-app Tauri backend (`apps/unipi/apps/mobile/src-tauri`)

- `net.rs`: per-process iroh `NetState` (persistent secret key, connection cache, `open_op` =
  one bi-stream per op, 10 s dial/stream timeouts, `unipi://pair` link parse `:171-215`, path
  classification for the LAN/Direct/Relay chip `:264`).
- `local_host.rs`: "This computer" synthetic host from `<host dir>/app.sock` + `endpoint.json`
  (`detect_in` `:38`, `merge` `:56`), `open_op` prefers the socket and falls back to
  iroh (`net.rs:120-140`), local-only `admin` op forwarding (`:195`), `paired_devices()` count
  for the tray (`:246`). Offline e2e test vs a real `unipi-host serve` (`:132`).
- `host_supervisor.rs`: decide **Adopt / StartService / Spawn** (`plan` `:62`), spawn =
  `serve --mode app --exit-with-parent <pid>` with 1..30 s backoff (`run_child` `:159`),
  version-skew detection, `host.log` tailing.
- `lifecycle.rs`: close-to-tray **always** (`close_action` `:100`), Quit confirm only when
  app-managed + phones paired (`:82`), autostart flag + `tauri-plugin-autostart` (`:55-75`),
  tray status lines.
- `desktop.rs`: tray (Show/Hide/Quit, `TRAY_ID`, `setup_tray` `:155`), OS notifications, window
  title, mascot window. `screen.rs` = desktop-streaming core side (delete).
- `always_on.rs`: systemd user unit + linger install (delete — no service install in v2).
- `term.rs`: `term_open/input/resize/close` Tauri commands wrapping host `attach`/`shell` op
  streams (mux-based). `chat.rs`: `chat_list/new/open/send/stop/close/config` wrapping the
  bridge relay. `hosts_commands.rs`, `host_store.rs` (`hosts.json`), `push.rs`, `forward.rs`,
  `fs.rs`, `blob.rs`, `browser.rs`, `sharing.rs`, `cron.rs`, `pair_phone.rs`, `app_update.rs`
  (tauri-plugin-updater), `setup.rs` (+`setup/install.rs`, `setup/updates.rs`), `op_stream.rs`
  (OpSend/OpRecv over iroh or unix socket), `retry.rs`, `path_kind.rs`, `local_files.rs`
  (desktop attach files), `ios_webview.rs` (iOS), `commands.rs` (app_info).

### 1.3 oino side (current)

- `oino-core`: `AgentSession` (`session/mod.rs:251`) with prompt/steer/follow-up/abort, models,
  thinking, queue, entries/tree, `EventHub`/`EventStream` broadcast (`hub.rs:134`), `ClientEvent
  {Session, Ui, ExtensionError, Oino}` (`events.rs:471`), `Services`/`ServicesOptions{loader,
  mode}` per cwd (`services.rs:108,475`), `SessionManager::{create,open,fork_from,find_by_id,
  list,list_all}` (`oino-session/src/manager/mod.rs:195-346`), settings paths `~/.oino` +
  `<cwd>/.oino` (`oino-settings/src/paths.rs:56`).
- `oino-client`: `SessionBackend` trait (`backend.rs:29`), `InProcessBackend` over
  `Arc<AgentSessionRuntime>` (`in_process.rs:46`), `RpcCommand`/`RpcResponseData` protocol
  records, `FrontendApi` typed calls.
- `oino-rpc`: JSONL framing + command set (pi commands verbatim + `_oino/*`, modes.md §3–§4);
  events-before-reply ordering rule (modes.md appendix #8).
- `oino-cron`: scheduler + store + control + `serve` daemon; **fires jobs by spawning
  `oino -p --mode json --session-id <run id>`** (`exec.rs:2,50,250`), owner lock
  (`serve.rs:37`), wake links. `oino cron serve` subcommand to be removed (DIRECTION §2).
- `oino-browser`: pipe transport, isolated worlds, AX snapshots, session-owned tabs, profiles.
- **No single-writer session lock exists yet** — §3.1.5 is spec-core work; this spec defines
  the host's use of it (§8).

---

## 2. Target architecture

```
  phone app ──iroh──┐                       apps/unipi (Tauri 2 + SolidJS) — pure client
  other PC app ─iroh┤                         │ "This computer" = local socket, else iroh
                    ▼                         ▼
              unipi-host  ◄── unix socket / named pipe (same protocol, same frames)
              crates/unipi-host (links oino-core in-process, one tokio runtime)
              session hub · cron (oino-cron) · browser (spec-browser) · fs · git · pty ·
              push/notify · forward · sharing/blob · pairing · updater · settings
                    │
              oino-core (+ wasm host, shared Engine)  ←  ~/.oino/extensions (UniPi distribution
                    │                                    packages), ExtensionMode::Desktop
              oino TUI / CLI (same session files, same lock — takeover §8)
```

Rules (DIRECTION §2, binding):

1. **One unipi-host process per user** runs all agent sessions in one tokio multi-thread
   runtime. It links `oino-core` as a library and is a *client of core*, exactly like the TUI.
   It is **never** linked into the UI process; the app is a pure client of a host.
2. The PC app is host + client at once ("This computer" over a local socket, works offline);
   the phone is a **remote-only** client (paired hosts over iroh, no local host code paths).
3. `oino --mode rpc` / ACP stay untouched for third-party clients.
4. Removed from the host and protocol: **herdr, tmux, the pi app-bridge, desktop streaming,
   kanboard daemon relay, service install**. The terminal becomes host-owned PTYs (§9.3).

---

## 3. Target repo layout

Coordinates with `specs/core.md` (pi-like monorepo). The import at `apps/unipi` is dissolved;
nothing else moves.

| v2 location | Comes from | Notes |
|---|---|---|
| `crates/unipi-proto` | `apps/unipi/crates/proto` | Rust wire types (frames, Control/Request, service DTOs). Joins the **root cargo workspace** (members `crates/*`) so it can use `oino-core` event types in the session layer (§7). |
| `crates/unipi-host` | `apps/unipi/crates/host` | `unipi-host` bin + `unipi_host` lib. Depends on `oino-core`, `oino-cron`, `oino-browser`, `unipi-proto`. Root workspace member. |
| `apps/unipi` | `apps/unipi/apps/mobile` | The Tauri 2 app (`src/`, `src-tauri/`, `package.json`), still its own cargo workspace (src-tauri) + npm workspace with `apps/unipi/packages/protocol`. |
| `apps/unipi/packages/protocol` | stays | TS/zod mirror of `unipi-proto` (+ new session module, §7.6). |
| `extensions/unipi/*` | stays | UniPi distribution extension packages (cron, browser, kanboard CLI, …; `specs/ext-abi.md`). |
| deleted | `apps/unipi/crates`, `apps/unipi/apps`, root `apps/unipi/{Cargo.toml,package.json,Cargo.lock,DESIGN.md}` | after the move; `UPSTREAM.md` provenance moves to `apps/unipi/UPSTREAM.md`. |

`unipi-proto` in the root workspace is deliberate: the session/event DTOs reference
`oino_core::events` serde shapes, and the root workspace is where core lives. The Tauri app
stays out of the root cargo workspace (Tauri builds and target dirs differ).

---

## 4. The unipi-host process

### 4.1 Crate and binary

- `crates/unipi-host`: `src/main.rs` (CLI), `src/lib.rs` (library: everything testable).
  Binary name `unipi-host`. Subcommands (see §11 table for per-file fate):
  - `serve [--relay <url|n0>] [--relay-only] [--mode app|manual] [--exit-with-parent <pid>]
    [--exclude-extension <id>]… [--no-extensions]` — the daemon.
  - `status [--json]`, `pair [--ttl] [--json]`, `devices`, `revoke <target>`,
    `forward …`, `browser …`, `fs …`, `requests …`, `sharing status`, `push …`,
    `cron …`, `update [--check]`.
  - Removed: `desktop`, `linger` (no service install).
- Subcommands other than `serve` are thin clients over the running host's local control socket
  (existing `push.rs` `ControlRequest`/`ControlEvent` JSONL pattern), so they share one
  scheduler/browser state with the daemon.

### 4.2 Session hub

New module `crates/unipi-host/src/sessions/`:

```
sessions/
  hub.rs        SessionHub — the owner of all live sessions
  card.rs       SessionCard / project grouping (pure)
  stream.rs     per-client session stream + snapshot/resume + coalescing (§7.3-7.5)
  crash.rs      panic isolation, safe mode (§4.4)
```

- `SessionHub` holds `HashMap<SessionId, HubSession>` behind one `RwLock`, plus a services
  cache keyed by `(cwd, agent_dir)` of `oino_core::Services`. `HubSession { runtime:
  Arc<AgentSessionRuntime>, events: broadcast::Sender<SeqEvent>, ring: VecDeque<SeqEvent>,
  state: HubState }` where `HubState = Idle | Working | Crashed{error} | TakenOver{from}`.
- **One tokio runtime** (multi-thread) for everything: sessions, iroh, PTYs, cron, browser.
  Sessions share the core `Services`/wasmtime `Engine` (spec-core §3.2.1).
- **Lazy loading**: the session list is built from `SessionManager::list_all` (headers only,
  no runtime) merged with live hub entries; a session becomes a runtime only on
  `session_open` (spec-core §3.2.2). Cards carry `{session_id, file, name, cwd, updated_at,
  message_count, run_state, needs_you?, model?}` grouped by `cwd` (project grouping).
- **Create / open / fork / archive / close**:
  - `session_new{cwd, name?}` → new runtime (fresh session file per core rules).
  - `session_open{session: path|id|partial, since_seq?}` → load or reuse, **takeover** (§8),
    snapshot or resume patch (§7.4).
  - `session_fork{session, entry_id?, name?}` → core fork, opens the fork as the stream's
    session (replaces the old app-bridge world where forking wasn't possible remotely).
  - `session_archive{session}` → core delete-to-trash (`~/.oino/trash/`, the same API the ACP
    `session/delete` path uses) — "Archive" in the UI, restorable from disk.
  - `session_close{session}` → dispose the runtime when idle and no other subscriber;
    the session file stays. Crashed sessions are disposed on next open.
- **Concurrency**: many sessions live at once (DIRECTION §4.3 "many running at once");
  per-session event taps never block each other (§7.5).
- **Extension loading**: the host builds each project's `Services` with oino's standard
  loader (same discovery as the `oino` binary: `~/.oino/extensions` + trust-gated
  `<cwd>/.oino/extensions` + `-e`), `mode: ExtensionMode::Desktop`. The UniPi distribution's
  packages are installed into `~/.oino/extensions` by the installer (`specs/core.md` repo
  split); the host loads **nothing** compiled in. `--exclude-extension <id>` (repeatable) and
  `--no-extensions` are honoured per DIRECTION §3.3.4, plus a per-extension enable toggle via
  the `_oino/set_extension_enabled` command.
- **Host services** (per `specs/ext-abi.md` registry: `host-service.call(name, op, json)`,
  permission per service name): the host registers, for every session's services hub:
  - `unipi.cron` — ops mirroring `CronControl` (list/add/edit/rm/enable/disable/run_now/stop/
    runs/log/pause/resume/status/settings). The cron *extension* writes job files and reads
    run state through this service; the scheduler itself is host-owned (§9.8).
  - `unipi.browser` — ops mapped onto `oino-browser`'s service surface (spec-browser). Under
    a plain TUI the browser tools report "unavailable" because only the host registers it.
  - `unipi.kanboard` — thin wrapper executing the `unipi-kanboard` CLI (spec-ext-abi,
    kanboard extension package) for the Board screen; `unipi.host` — `info`, `shell-env`
    (read-only), used by extensions that need host facts.
  Registration happens through the core host-router service registry
  (`oino-wasm-host/src/hub.rs:514` `register_service`); exact host-side handler trait is
  `specs/ext-abi.md`'s to define — this spec only fixes the names and the host behaviours.

### 4.3 Removals from the host

| Removed | Replacement |
|---|---|
| `mux/` (herdr, tmux, shell), `list/tree/shortcuts/preview/attach/shell/mux_action` ops | host PTY terminal service (§9.3) |
| `chat.rs`, `chat_ctl.rs`, `chat_*` ops, app-bridge relay | sessions run in-process (§7) |
| `desktop/` (all capture/encode), `desktop_*` ops, openh264/x11/portal/mutter/windows-capture deps | none (desktop streaming deferred; revisit later without reopening this spec) |
| `board/` + `board_http`/`board_sse` | `unipi.kanboard` host service over the CLI |
| systemd/LaunchAgent/logon-task service install, `linger` | tray lifetime only (§5) |

### 4.4 Crash isolation and safe mode (host side of DIRECTION §3.3)

- **Per-session panic isolation**: each hub session's runtime tasks are wrapped so a panic
  or trap marks that session `Crashed{error}`, emits `{kind:"crashed"}` to its streams and a
  `SessionChanged` card update, and **the other sessions keep running**. The crashed session
  is disposed and re-openable (resume layer repairs the torn tail per spec-core §3.1.4).
  A trapping *extension* is disabled for that session by core (spec-core §3.3.1); the host
  surfaces the notice.
- **Safe mode**: the host records start outcomes in `~/.oino/host/start-history.json`
  (`{ts, outcome, uptime_s}`); after **2 consecutive failed starts** (exit within 60 s), the
  next start runs with extensions disabled and emits a `HostNotice` banner naming the
  suspect extension(s) from `host.log`. A successful long-running start clears the counter.
- `--exclude-extension <id>` (repeatable) and `--no-extensions` flags, plus the app's
  per-extension toggle (`_oino/set_extension_enabled`), give the user manual control.

---

## 5. Host lifetime

Decisions (DIRECTION §2, owner Q4 = no service install):

1. **Adopt or spawn-as-child.** The app's supervisor (ported `host_supervisor.rs`, plan
   simplified) decides, in order: **Adopt** — `<host dir>/app.sock` answers → use the running
   host; **Spawn** — run `unipi-host serve --mode app --exit-with-parent <app pid>` as a child,
   stdout/stderr → `<host dir>/host.log`, restart backoff 1, 2, 4 … 30 s, UI banner after 3
   consecutive crashes. The `StartService` branch and `always_on.rs` (systemd unit + linger)
   are **deleted**. Manual `unipi-host serve` in a terminal keeps working (`--mode manual`).
2. **Single host instance per user.** Keep `instance.rs` verbatim: exclusive lock on
   `~/.oino/host/host.lock`, owner record `{pid, version, mode, started_at}`, exit 75 when
   held, `status` for the app/tray. `Mode::{App, Manual}` (drop `Service`).
3. **Tray.** Tray icon (keep `desktop.rs` tray code): title *UniPi*, menu **Show / Hide /
   Quit** + status line ("N phones · M sessions running · needs-you count") and tooltip.
   Left click toggles the window (Windows/macOS). Quit exits the app; an app-managed host
   dies with it (`--exit-with-parent`).
4. **Ask-on-close.** On main-window `CloseRequested`: if the remembered setting
   (`closeAction`, app settings, values `ask | tray | quit`, default `ask`) is `ask`, show a
   dialog once: **Minimize to tray** (default) or **Quit**, with "Remember my choice";
   the answer is stored and changeable in Settings → General. This *replaces* unipi-app 1.0's
   "close always hides" (`lifecycle.rs:100`) per DIRECTION §2 ("asks once: close or minimize
   to tray").
5. **Quit semantics.** Quit stops the host (cron and remote access stop with it). The quit
   confirm keeps unipi-app's warning when the host is app-managed and ≥1 device is paired
   (`lifecycle.rs:82`): "phones can't reach this PC until you open UniPi again".
6. **Start at login.** `tauri-plugin-autostart` writes the per-user login item (Windows Run
   key, macOS login item, Linux XDG autostart `.desktop`), launching **the app** hidden in the
   tray (`--hidden` arg; no window); the app then adopts/spawns the host. No admin, polkit,
   systemd, LaunchAgent or scheduled task anywhere. The host itself has no autostart flags.
7. **Cron + remote alive while the tray is alive.** `serve` runs the cron scheduler tick
   (§9.8) and the iroh endpoint for the whole process life; Quit stops both (running cron
   jobs get the 30 s drain grace from `oino-cron`).
8. **Data dir**: `~/.oino/host` (env override `$OINO_HOST_DIR`). Contents: `secret.key`,
   `devices.json`, `pairing.json`, `endpoint.json`, `config.json`, `notify-key`,
   `blobs/`, `host.lock`, `host.log`, `start-history.json`, `app.sock` / pipe, `control.sock`.
   The host never reads or writes `~/.unipi` or `~/.pi` (migration is a skill, DIRECTION §4.1).
9. **Tauri identifier** `dev.unipi.oino`; product name *UniPi*; app config dir
   `dev.unipi.oino` (the `lifecycle.rs:11` pattern with the new id).

### 5.1 Coexistence with the owner's old unipi

| | old unipi (live) | v2 |
|---|---|---|
| data dir | `~/.unipi/app-host` | `~/.oino/host` |
| iroh ALPN | `unipi/app/1` | `unipi/oino/1` |
| local socket | `<dir>/app.sock` (unix) | `<dir>/app.sock` (unix) + `\\.\pipe\unipi-oino-app` (Windows) |
| control socket | `<dir>/control.sock` | `<dir>/control.sock` (different dir ⇒ no clash) |
| iroh secret key | `~/.unipi/app-host/secret.key` | `~/.oino/host/secret.key` (own endpoint id) |
| Tauri identifier | `dev.unipi.app` | `dev.unipi.oino` |
| tray label | "UniPi" | **"UniPi (oino)"** — tray title/tooltip differ so both can sit in the tray |
| updater feed | unipi.nrn.one host releases | the oino/UniPi release feed (`specs/core.md` distribution; URL from release-manifest.json) |

Both daemons can run side by side; different keys, sockets, ports and dirs. The v2 host must
not parse or lock anything under `~/.unipi`.

---

## 6. Transport

**Keep unipi-app's implementation** (DIRECTION §2 names these files as the reference).

### 6.1 Local ("This computer", PC app only)

- Unix: `<dir>/app.sock`, 0600, same-user uid check (`local.rs`). **Windows (new)**: named
  pipe `\\.\pipe\unipi-oino-app` with an owner-only DACL, same frames, same `Hello`; this
  closes the gap where unipi-app's Windows local socket is a stub (`local.rs:81`). `ipc.rs`
  already provides pipe helpers used by the bridge; reuse for the listener.
- The socket is authoritative for adopt-detection, `local_admin` ops (host_info, pair_begin,
  pair_status, devices_list, device_revoke stay local-only), and offline operation; iroh is
  the fallback when the socket is gone (`net.rs:120` behavior kept).

### 6.2 iroh (paired hosts)

- iroh (QUIC) with the N0 preset: stable endpoint id from `secret.key`, n0 relays + discovery,
  hole punching to LAN/direct paths, relay fallback; `--relay <https url>` custom relay
  (self-hosted), `--relay-only` mode. Relay URL is also a host setting (`config.json`
  `relay`) so the app can change it without flags. All kept from `endpoint.rs`/`main.rs`.
- One connection cached per remote (`net.rs`), one **bi-stream per logical op** — kept. New
  persistent streams (§7) are bi-streams that stay open.
- App side (`net.rs`, `op_stream.rs`, `path_kind.rs`) kept; ALPN string changes; ticket /
  deep-link scheme changes to `unipi://…` **with a new host marker**: the pairing link stays
  `unipi://pair?t=<ticket>&c=<code>` (tickets are self-describing; a v1 phone scanning a v2
  QR fails the ALPN/protocol check with the existing "Update the host or app" error).

### 6.3 Pairing

Kept from unipi-app: QR + link via `pair_cli`/`pair_link` (10-minute TTL, single-use 10-char
Crockford code, lockout after repeated failures in `pairing.rs`), `PairBegin/PairStatus`
(terminal QR or in-app "Pair a phone" dialog, `pair_phone.rs`), device allowlist
(`devices.json`) with revoke in Settings, LAN discovery via iroh mDNS (phone). Devices are
iroh endpoint ids; `Peer::{Iroh, Local}` distinction and local-only admin ops kept
(`protocol.rs:28-48`).

### 6.4 Phone = remote only

`cfg!(desktop)` gates every local-host path (`local_host.rs:17` pattern kept): on Android the
app has no host dir, no supervisor, no tray, no autostart; it dials paired hosts over iroh
only. The terminal/files/git/browser/cron screens all work remotely through the same ops.

---

## 7. App ↔ host protocol

Keep `unipi-proto` / `packages/protocol` (frames, Control, error envelope, service DTOs,
zod schemas, fixtures); **rewrite only the chat/session layer**; delete mux/board/desktop ops.

### 7.1 Framing and versioning (kept, extended)

- Frames stay `u8 kind · u32BE len · payload`, kinds 0 (JSON Control) / 1 (binary output) /
  2 (binary input), 1 MiB cap, incremental decoder (`frame.rs`). Terminal output/input, blob
  bytes and fs_read bodies already ride kinds 1/2; JSON stays JSON.
- `Hello{proto, req}` first frame; major version must match or `E_PROTO_VERSION` with the
  existing friendly text. **`PROTO_VERSION` bumps to `2.0.0`** (chat/session rewrite is
  breaking); minors add ops additive (existing convention, e.g. 1.20.0).
- One bi-stream per request/response op (kept). Persistent streams: `session_open`/
  `session_new`/`session_fork` (one per open chat), `events_open` (one per connection),
  `term_open`/`term_attach`, `browser_open`, `tcp_forward`, `push_list{wait}` (long-poll).
  QUIC bi-streams are cheap and give per-stream ordering for free; the local socket
  multiplexes the same way with one connection per op.

### 7.2 Op set after the rewrite

- **Deleted**: `list`, `tree`, `shortcuts`, `preview`, `attach`, `shell`, `mux_action`,
  `chat_list`, `chat_open`, `chat_new`, `chat_close`, `chat_config`, `chat_config_set`,
  `board_http`, `board_sse`, `desktop_status`, `desktop_open` (+ their `Control` variants
  `Chat`, `BoardEvent`, `DesktopCmd`, `DesktopEvent` and `proto/src/{chat,board,desktop}.rs`).
- **Kept as-is**: `pair`, `host_info`, `pair_begin`, `pair_status`, `devices_list`,
  `device_revoke`, `blob_put`, `blob_get`, `fs_roots`, `fs_list`, `fs_read`, `fs_write`,
  `fs_stat`, `fs_search`, `fs_mkdir`, `fs_rename`, `fs_import`, `fs_delete`, `git`,
  `notify_config`, `notify_register`, `notify_test`, `ports_list`, `tcp_forward`, `push_list`,
  `push_done`, `browser_*`, `feature_request(s)`, `feature_leave`, `cron_*` (all 18).
- **Changed**: `host_info` (tools list: `oino`, `unipi-kanboard`, browser engines; drop
  herdr/tmux/pi fields).
- **New**: `sessions_list`, `events_open`, `session_new`, `session_open`, `session_fork`,
  `session_archive`, `session_rename`, `session_close`, `session_cmd`, `term_open`,
  `term_attach`, `term_list`, `term_kill`, `host_settings_get`, `host_settings_set`.
  Git gains `GitCmd::TurnFiles` (§9.4). `Control` gains `Events` and `SessionCmd` (terminal
  streams reuse the existing `Resize`/`Exit` controls).

### 7.3 Session streams

`session_open` / `session_new` / `session_fork` open a persistent bi-stream for **one**
session (mirrors today's `chat_open` relay shape, but both directions are ours):

```
client → host : Hello{proto, req: session_open{session, since_seq?}}        (or new/fork)
host → client : Ok{data: SessionSnapshot}                                   (first frame)
client → host : Control::SessionCmd{id?, cmd: <RpcCommand JSON>}            (repeatable)
host → client : Control::Ok{id?, data} | Control::Err{id?, error}           (correlated)
host → client : Control::Events{items: [SessionEventDto…]}                  (live, batched)
client → host : (stream close = unsubscribe; session keeps running)
```

- `SessionSnapshot` = `{ session_id, reset: bool, seq, state: RpcSessionState, entries:
  [SessionEntry] (current branch), leaf_id, commands: [CommandInfo], ui: [UiRequest]
  (retained + pending dialogs, `_oino/ui_snapshot` shape), topics: [TopicValue] (sticky
  snapshot for the session's subscriptions) }`. Fields reuse existing RPC response shapes so
  the TS side mirrors known schemas.
- `SessionCmd.cmd` is **any oino RPC command verbatim** (pi commands of modes.md §3.2 plus
  `_oino/*` of §4): `prompt`, `steer`, `follow_up`, `abort`, `clear_queue`, `get_state`,
  `set_model`, `cycle_model`, `get_available_models`, `set_thinking_level`,
  `cycle_thinking_level`, `get_available_thinking_levels`, `set_steering_mode`,
  `set_follow_up_mode`, `compact`, `set_auto_compaction`, `set_auto_retry`, `abort_retry`,
  `bash`, `abort_bash`, `get_session_stats`, `switch_session`, `fork`, `clone`,
  `get_fork_messages`, `get_entries`, `get_tree`, `set_session_name`, `get_messages`,
  `get_commands`, `extension_ui_response`, and `_oino/hello`, `_oino/ui_snapshot`,
  `_oino/ui_action`, `_oino/editor_event`, `_oino/invoke_shortcut`, `_oino/get_completions`,
  `_oino/render`, `_oino/get_settings_schema`, `_oino/get_extension_settings`,
  `_oino/set_extension_setting`, `_oino/get_settings`, `_oino/set_setting`,
  `_oino/get_keybindings`, `_oino/bus_get`, `_oino/topics`, `_oino/navigate_tree`,
  `_oino/set_label`, `_oino/list_sessions`, `_oino/extensions`, `_oino/set_extension_enabled`,
  `_oino/login`/`_oino/logout`/`_oino/set_api_key`/`_oino/auth_providers`,
  `_oino/get_context_usage`, `_oino/agents`, `_oino/agent_messages`, `_oino/agent_abort`,
  `_oino/queue_edit`, `_oino/checkpoints`, `_oino/rewind`, `_oino/running_bash`,
  `_oino/service_call`, `_oino/get_shortcuts`. The host forwards them to the session's
  `SessionBackend` (an `InProcessBackend` over the hub runtime) — one implementation, zero
  re-interpretation. `_oino/hello` is sent implicitly by the host at stream open with the
  app's `UiCapabilities` (spec-ui-protocol §10) so Oino-only dialogs work.
- Ordering guarantee (modes.md appendix #8 kept): the stream writer drains queued events
  before writing a command reply, so events a command caused precede its reply on the same
  stream.

### 7.4 Event stream, resume, backpressure

- `SessionEventDto = { session: SessionId, seq: u64, event: <ClientEvent JSON> }`. The
  `event` payload is **exactly oino's `ClientEvent` serde JSON** (`oino-core/src/events.rs:471`
  — `AgentSessionEvent` with the JSON-mode `message_update` transform, `extension_ui_request`,
  `extension_error`, and `OinoEvent` topic/agents/queue/run-state/shutdown notifications).
  No second vocabulary: the TS package gains a `session.ts` mirroring these shapes with zod,
  and the app's chat reducers mirror `oino-view`'s `Transcript`/`UiState` reducers. Extension
  UI requests (dialogs, panels, slots) ride `UiRequest` per `specs/ui-protocol.md` v2 — this
  spec only transports them.
- Host-only events (same envelope, reserved kinds, never produced by core):
  `{kind:"crashed", error}`, `{kind:"taken_over", from}`, `{kind:"reloaded"}`.
- **seq + ring buffer**: each session numbers its events from 1; the hub keeps the last
  4096 serialized events. `session_open{since_seq}` resumes: if `since_seq+1` is still
  buffered the reply is `Ok{data:{reset:false, replay:[SessionEventDto…]}}` and the stream
  continues; otherwise a full snapshot with `reset:true` (client rebuilds from `entries`).
  Reconnect after iroh flap = new `session_open` with the last seen seq.
- **Delta coalescing** (host side, per stream): consecutive `message_update` text/thinking
  deltas for the same message merge into one event per ≤16 ms flush window; a non-delta event
  or window end flushes. All events are then batched: one `Control::Events` frame per flush
  tick per stream (≤60 fps of frames, each carrying n events). Non-delta events are never
  dropped or reordered.
- **Backpressure**: the host writer awaits stream writes (QUIC/local socket flow control).
  A slow client's per-session tap is a bounded broadcast channel (256): delta events are
  collapsed (they're cumulative-append), non-delta events suspend the tap until sent —
  correctness first, deltas are lossy-by-design after coalescing.
- **Binary framing**: deliberately not introduced. JSON control frames with 16 ms batching
  already amortize framing cost; terminal/blob bodies are binary today. Revisit with
  measurements (acceptance H10) — the frame format reserves kind ≥ 3 so a future
  `EventsBinary` kind is additive.
- **Hub event stream**: `events_open` opens one per-connection stream receiving
  `HubEvent`s: `SessionChanged{card}` (created/renamed/state/needs-you/removed — keeps the
  Sessions list live without polling), `HostNotice{level, text}` (safe-mode banner, updater
  prompt, crash notices). This replaces nothing in unipi-app (it polled); it is the one new
  push channel.

### 7.5 Performance rules

1. Snapshot size: `entries` of the current branch only; the app lazily fetches full tool
   outputs via existing commands (`get_entries`, `_oino/render`) when a row expands.
2. Sessions list: headers only (core `list_all`), never loading runtimes; live cards come
   from hub events.
3. Event throughput target: a 100k-token streamed reply produces ≤ 1 frame per 16 ms with
   coalescing (measured in WP-H10), and a phone on relay stays interactive (frame budget
   ≪ link capacity).
4. `fs_read`/blob/terminal already stream binary in ≤256 KiB chunks (kept).

### 7.6 Protocol packages

- `crates/unipi-proto`: keep modules (frame, error, version, admin, blob, fs, git, push,
  forward, sharing, browser, cron, control minus deleted ops); new `session.rs` (snapshot,
   event DTOs, cmd envelope), `term.rs`, `settings.rs`; shared fixtures keep the
   JSON-driven test convention (`proto/tests/*_fixtures.rs`).
- `apps/unipi/packages/protocol`: same in TS/zod; `chat.ts`, `desktop.ts`, `board.ts`,
  mux pieces of `control.ts` deleted; new `session.ts`, `term.ts`, `settings.ts`.

---

## 8. Session takeover (single-writer interplay, DIRECTION §3.1.5)

Core owns the lock mechanism (`specs/core.md`); the host defines the *behaviour*:

1. Every `session_open`/`session_new`/`session_fork` in the host opens the session through
   core **with takeover**: if another runner holds the session (the `oino` TUI in a terminal,
   an `oino -p` job, another host, another app), that runner is asked to close (core's lock
   signaling per spec-core), the session is reloaded **here**, and the turn continues
   (Devin-like). The user never sees a "busy" error from their own machines.
2. On success the host emits `{kind:"taken_over", from:"tui"|"host:<name>"|"cli"}` as the
   first session event after the snapshot; the app shows a one-line notice ("Taken over from
   the terminal"). The losing runner (TUI) shows its own notice per spec-core/tui-client.
3. Takeover waits at most 5 s for the holder to yield; if the holder is dead-but-locked the
   lock is stale and core reclaims it (spec-core). If the holder is alive and refuses
   (a long `oino -p` run mid-turn), the host waits for the current tool call to settle —
   core's flush points (§3.1.2) bound this — then reloads. `session_open` replies only after
   the session is exclusively ours.
4. Symmetrically: when **this host's** session is taken over, the hub marks it
   `TakenOver{from}`, emits `{kind:"taken_over"}` on its streams, disposes the runtime, and
   the card returns to "idle on disk" state (openable again = takes it back).
5. Hub sessions and the TUI share everything through core: same session files, same lock
   (DIRECTION §2 diagram), same settings — no host-side parallel state.

---

## 9. Host services (proto + behaviour)

All kept services keep their request DTOs, scopes, error codes and consent gates; only what
the table says changes.

### 9.1 fs (`fs.rs`, `fs_scope.rs`) — keep

Ops unchanged. Scope: `$HOME` + configured roots, allow/deny lists (`config.json` `fs`),
`outside:true` escape hatch with per-request consent, atomic writes with `base_hash`
conflict detection, `fs_read` cap and `as_text`. Defaults for safe roots kept. `fs_roots`
now also lists hub session cwds (live projects) instead of mux pane cwds.

### 9.2 blob + sharing (`blob.rs`, `sharing.rs`) — keep

Content-addressed `blobs/` store, `blob_put` dedup/resume, `blob_get` by sha or allow-listed
path (pushed files), `fs_import` to place uploads. Sharing consent gates
(`feature_request`/`feature_requests`/`feature_leave`, time-limited approvals, sweeper) kept
with `FeatureKind::{Forward, Browser, Push, Cron}` — **`Desktop` deleted** (streaming
removed). Push-file authorization interplay (`push.rs::authorize_fetch`) kept.

### 9.3 pty terminal (new `term.rs` in host) — ZCode-style

- Host-owned PTYs (`portable-pty`, reusing the proven `attach_stream` pump
  (`protocol.rs:898-975`) minus mux): `term_open{cwd, cols, rows, shell?}` → `Ok{data:
  {term_id, shell}}` then the stream carries output (kind 1), input (kind 2),
  `Control::Resize{cols,rows}` and `Control::Exit{code}`. **The PTY lives in the host**,
  keyed by `term_id`, scoped to a workspace (the session/project cwd; default `$SHELL -l` in
  `cwd`), so it survives panel/window close, works from the phone, and dies with the host.
- Multiple tabs = multiple `term_id`s; `term_list` → `[{term_id, cwd, shell, alive,
  last_activity}]`; `term_kill{term_id}`; `term_attach{id, cols, rows}` re-attaches an open
  stream to an existing PTY (resizes it, then streams on). Idle terminals are kept until
  host quit (ZCode parity); a cap (default 16, setting `term.max`) closes the oldest idle
  one first.
- **Resize throttling**: the host applies at most one resize per PTY per 100 ms (last-wins);
  the app may also throttle locally. Agent bash output stays in chat tool rows (never
  mirrored into terminals) per DIRECTION §4.3.
- Reattach shows new output only (no scrollback replay from the host); the app keeps its own
  xterm buffer per live tab, and re-paints via `term_attach` size resync. Scrollback
  restoration after a full app restart is accepted as lost (risk R7).
- tmux/herdr remain usable *inside* a terminal if the user types them — the host just
  doesn't model them.

### 9.4 git (`git.rs`, `proto/src/git.rs`) — keep + last-turn scope

All existing `GitCmd`s kept (status/diff/stage/unstage/discard/commit/log/branches/switch/
create/pull/push/fetch/stash/pop/amend, ff-only pull default, auth hints). New
`GitCmd::TurnFiles{path, files: [GitPath], staged?, untracked?}` — status + lazy per-file
diffs **restricted to the given files**: the app passes the last turn's touched files
(collected from the turn's tool rows in the chat transcript), giving the ZCode "last-turn
scope" filter without new snapshot machinery. Branch/ahead-behind data unchanged
(`Repo`/`Status`).

### 9.5 push + notify (`push.rs`, `notify.rs`) — keep

ntfy/UnifiedPush with E2E (`unipi1:` AES-256-GCM payloads, per-pairing key at
`~/.oino/host/notify-key`, neutral "UniPi" title), `notify_config` ("Get from this PC"),
`notify_register` (UnifiedPush endpoint fan-out list), `notify_test`, `push_list` long-poll
with `wait`, `push_done` outcomes, CLI `unipi-host push <file>` via `control.sock`, pending
expiry. The cron extension's run notifications and the needs-you notices go through the
same channels (§9.8).

### 9.6 forward (`forward.rs`) — keep

`ports_list` (loopback listeners), `tcp_forward{port}` raw pipe, config
`{"forwarding":{enabled, ports: "any"|[…], devices}}`, per-device consent. Unchanged.

### 9.7 updater (`update.rs`) + app updater (`app_update.rs`) — keep, de-service

- Host self-update: sha256-verified atomic replace kept; release feed moves to the
  oino/UniPi distribution manifest; **restart without services**: after replace, the host
  exits — the app supervisor respawns it within its backoff (app-managed case), or a manual
  `serve` prints "restarted binary; rerun unipi-host serve". `update.rs`'s
  `Service::{SystemdUser, LaunchAgent, LogonTask}` detection is deleted.
- App updater: `tauri-plugin-updater` flow kept (`app_update.rs`, `setup/updates.rs`).
- The background check in `serve` (every few hours, prompt|auto) is kept; "prompt" raises a
  `HostNotice` on `events_open` instead of a desktop-app-only hook.

### 9.8 cron firing (`oino-cron` merged in) 

- `unipi-host serve` embeds the **`oino-cron` scheduler** (store/schedule/exec/notify/wake),
  ticking every second (existing pattern `main.rs:615`). `oino cron serve` and its control
  socket are removed from the `oino` binary (DIRECTION §2); the crate stays as the library
  the host calls. The owner lock (`serve.rs:37`) now guards against a second scheduler
  inside the host only (one host per user ⇒ one scheduler).
- Jobs are files under `~/.oino/cron` (oino-cron paths) **written by the cron extension**
  through the `unipi.cron` host service (spec-ext-abi; jobs-as-files per DIRECTION
  §3.4.2). The 18 `cron_*` app ops map onto `CronControl` (adapter in `cron_serve.rs`
  port); sharing/consent for phones (`FeatureKind::Cron`) kept.
- **Firing model: keep spawn-based exec** (`oino -p --mode json --session-id <run id>`,
  `oino-cron/src/exec.rs`): crash isolation from the host process, time limits and output
  capture already solved, and the binary ships beside the host. In-process firing through
  the hub is a later optimization once per-session panic isolation (spec-core §3.3.2) is
  proven; revisit after WP-H10 measurements, not now.
- Wake links (`wake_session`/`wake_here`): after a run, the host opens the linked session
  in the hub (takeover semantics) and prompts the wake message — now first-class instead of
  socket-poking a live pi.
- Run notifications reuse `notify.rs` channels (ntfy/gotify/telegram channel set is
  oino-cron's; keep).

### 9.9 settings (new `settings.rs`) — one Settings screen

- **Host settings** `~/.oino/host/config.json` (name kept for loader reuse), schema:
  `{ relay: "n0"|url, relay_only: bool, forwarding: {…}, push: {…}, fs: {allow:[], deny:[]},
  browser: {…}, sharing: {…}, cron: {…}, term: {max: 16}, update: {mode: "prompt"|"auto"} }`.
  Ops: `host_settings_get` → the whole object (secrets masked); `host_settings_set{key,
  value}` (dotted key, validated by the same proto schemas; unknown key ⇒ `E_SCHEMA`).
  Files are read on every request (kept behaviour — no restart needed).
- **App settings** live in the app config dir (`dev.unipi.oino`): `closeAction`, autostart,
  per-host names, UI prefs. App-local, never sent to the host.
- **pi agent settings** (`~/.oino/settings.json`, project `.oino/settings.json`) and
  extension settings schemas are read/written through the session command set
  (`_oino/get_settings`, `_oino/set_setting`, `_oino/get_settings_schema`,
  `_oino/get_extension_settings`, `_oino/set_extension_setting`) — the app renders **one**
  Settings screen combining all three (DIRECTION §4.3); this spec adds no new surface for
  them, it only guarantees the commands work over session streams.

### 9.10 browser — reference only

The browser is a host service built on `oino-browser` plus the unipi-app ports (cloak
downloader/verifier, cookie import, handoff gate, profiles, agent endpoint). Engine choice,
CDP hygiene and CI canaries: `specs/browser.md`. The wire ops (`browser_status/open/close/
window/profiles/takeover`, `BrowserCmd`/`BrowserEvent` + JPEG frames) are kept as-is from
unipi-proto with only path/data-dir changes; `AGENT_BROWSER_CDP` stable endpoint kept.

---

## 10. Android / phone build

- Same Tauri codebase, `tauri android` build (kept from unipi-app, `src-tauri/gen/android`
  incl. `UnicrabView.kt`, `MascotOverlayService.kt`); **no host binary, no supervisor, no
  tray, no autostart, no local socket** (`cfg!(desktop)` gates, existing pattern).
- Remote-only: iroh dialing of paired hosts; QR scan to pair; needs-you UnifiedPush path
  with native key storage (kept, `push.rs` Tauri side).
- Sessions, terminal (host PTYs), files, git, browser view, cron, push, forward, settings —
  all work remotely over the same ops. iOS stays build-compatible (no new native code beyond
  what unipi-app had; `ios_webview.rs` kept).

---

## 11. Keep / change / delete — every file

### 11.1 `apps/unipi/crates/host/src` → `crates/unipi-host/src`

| File | Verdict | Notes |
|---|---|---|
| `admin.rs` | **change** | `tools()` → `{oino, unipi_kanboard, browser_engines, cloak}`; drop herdr/tmux/pi |
| `blob.rs` | keep | path prefix change only |
| `board/` (`discovery.rs`, `mod.rs`, `sse.rs`) | **delete** | kanboard daemon relay; Board goes through `unipi.kanboard` service |
| `browser/` (11 files) | **change** | port per `specs/browser.md`: onto oino-browser base; cloak/cookies/gate/profiles/agent_endpoint kept; data dir `~/.oino` |
| `chat.rs` | **delete** | app-bridge relay → in-process sessions |
| `chat_ctl.rs` | **delete** | herdr/tmux chat control |
| `cron/` (7 files) | **delete** | replaced by `oino-cron` crate (feature-equivalent port already in-tree) |
| `cron_serve.rs` | **change** | thin adapter: 18 `cron_*` ops → `CronControl` on oino-cron; local-only sharing gates kept |
| `data_dir.rs` | **change** | `~/.oino/host`, `$OINO_HOST_DIR`; drop Windows legacy `~/.unipi` fallback; add `settings`/`term` keys docs |
| `desktop/` (12 files) | **delete** | desktop streaming removed; openh264/x11rb/zbus/ashpd/windows-capture deps leave Cargo.toml |
| `devices.rs` | keep | as-is |
| `endpoint.rs` | keep | as-is |
| `forward.rs` | keep | as-is |
| `fs.rs` | keep | `fs_roots` lists hub session cwds |
| `fs_scope.rs` | keep | as-is |
| `git.rs` | **change** | add `TurnFiles` |
| `instance.rs` | **change** | drop `Mode::Service`; rest verbatim |
| `ipc.rs` | keep | used by named-pipe local socket + control socket |
| `lib.rs` | **change** | new module tree (sessions/, term.rs, settings.rs) |
| `local.rs` | **change** | path; **implement Windows named-pipe listener** (owner-only DACL) |
| `logging.rs` | keep | as-is |
| `main.rs` | **change** | drop `desktop`/`linger`; `--exclude-extension`/`--no-extensions`; embed oino-cron; new serve wiring |
| `mux/` (`herdr.rs`, `tmux.rs`, `shell.rs`, `mod.rs`) | **delete** | no multiplexer modelling |
| `notify.rs` | keep | path change |
| `pair_cli.rs` | keep | as-is |
| `pair_link.rs` | keep | as-is |
| `pairing.rs` | keep | as-is |
| `protocol.rs` | **change** | dispatch minus mux/chat/board/desktop; plus session/term/settings ops; PTY pump moves to `term.rs` |
| `push.rs` | keep | `control.sock` kept for the CLI |
| `secret.rs` | keep | as-is |
| `sharing.rs` | **change** | drop `Desktop` feature kind |
| `update.rs` | **change** | de-service restart; new feed URL |
| new: `sessions/` (`hub.rs`, `card.rs`, `stream.rs`, `crash.rs`), `term.rs`, `settings.rs` | **new** | §4.2, §7, §9.3, §9.9 |
| `examples/`: `pair_client.rs` keep; `big_transfer.rs` keep; `desktop_bench.rs`, `desktop_probe.rs` **delete** | | |
| `tests/`: keep `admin_ops, blob, browser, cron*(re-targeted), first_frame, forward, fs_scope, git, instance, local_socket, notify_config, pair_json, pairing_atomic, push, sharing, sharing_leave, transport`; **delete** `board_relay, chat_ctl, chat_relay, cron_open_session, desktop*, herdr_m2, mux_action_m2, tmux_m2`; **new** `sessions_hub, session_stream, session_takeover, term, settings` | | |

### 11.2 `apps/unipi/apps/mobile/src-tauri/src` → `apps/unipi/src-tauri/src`

| File | Verdict | Notes |
|---|---|---|
| `always_on.rs` | **delete** | systemd unit + linger; no service install |
| `app_update.rs` | keep | tauri-plugin-updater |
| `blob.rs` | keep | |
| `board.rs` | **change** | transport → `unipi.kanboard` service ops (app side; screens spec) |
| `browser.rs` | keep | |
| `chat.rs` | **rewrite** | session streams (open/cmd/events, snapshot+resume) over `net.rs`/`local_host.rs` |
| `commands.rs` | keep | `app_info` proto string bump |
| `cron.rs` | keep | op forwarding unchanged |
| `desktop.rs` | **change** | keep tray/mascot/notify/title; tray label "UniPi (oino)"; drop nothing else here (streaming was `screen.rs`) |
| `forward.rs` | keep | |
| `fs.rs` | keep | |
| `host_store.rs` | keep | |
| `hosts_commands.rs` | **change** | drop mux ops (`list_sessions`, `mux_tree`, `mux_preview`, `mux_action`); keep pair/list/forget |
| `host_supervisor.rs` | **change** | Adopt/Spawn only; drop StartService/systemctl paths |
| `ios_webview.rs` | keep | |
| `lifecycle.rs` | **change** | ask-on-close remembered (`closeAction`); config dir `dev.unipi.oino`; quit confirm kept |
| `lib.rs` | **change** | command registry updates; `--hidden` arg handling |
| `local_files.rs` | keep | |
| `local_host.rs` | **change** | `~/.oino/host`; Windows named-pipe connect; admin op list unchanged |
| `main.rs` | keep | |
| `net.rs` | **change** | ALPN `unipi/oino/1`; rest kept |
| `op_stream.rs` | keep | |
| `pair_phone.rs` | keep | |
| `path_kind.rs` | keep | |
| `push.rs` | keep | |
| `retry.rs` | keep | |
| `screen.rs` | **delete** | desktop streaming core side |
| `setup.rs`, `setup/` | **change** | drop mux/herdr detection; keep PATH merge (host needs `oino` on PATH), first-run wizard |
| `sharing.rs` | keep | Desktop kind gone from proto |
| `term.rs` | **rewrite** | `term_open/attach/input/resize/close/kill/list` on host PTYs |

`tauri.conf.json`: identifier `dev.unipi.oino`, `externalBin` `binaries/unipi-host` kept
(bundled sidecar), plugins unchanged (autostart, updater, deep-link, barcode-scanner, …).

---

## 12. Work packages

Dependency order; each ends green (`cargo fmt/clippy/test`, `npm test`, `tsc`) before the
next builds on it. Prefix `[host]`.

### WP-H1 — Repo move + proto v2 skeleton
- Files: move `apps/unipi/crates/proto` → `crates/unipi-proto`, `apps/unipi/crates/host` →
  `crates/unipi-host`, `apps/unipi/apps/mobile` → `apps/unipi` (+ `packages/protocol`);
  root `Cargo.toml` members already `crates/*`; delete `apps/unipi/{crates,apps}` remnants
  and workspace manifests; `UPSTREAM.md` → `apps/unipi/`.
- In `unipi-proto`: bump `PROTO_VERSION` to `2.0.0`; delete mux/board/desktop/chat DTOs and
  ops from `control.rs` (keep `Control::{Hello,Ok,Err,Resize,Exit,Title,BrowserCmd,
  BrowserEvent}`); add empty `session.rs`, `term.rs`, `settings.rs` modules; prune fixtures.
- Deps: none (first).
- Accept: `cargo check --workspace` (root incl. new crates), `cargo test -p unipi-proto`,
  `cargo clippy -p unipi-proto -p unipi-host --all-targets -- -D warnings`,
  `(cd apps/unipi && npm install && npm test && npm run typecheck)` — after
  H1 the TS suite may temporarily skip deleted modules; delete their tests here too.

### WP-H2 — Host skeleton: lifetime + transport
- Files: `crates/unipi-host/src/{main.rs,data_dir.rs,instance.rs,local.rs,endpoint.rs,
  secret.rs,pairing.rs,pair_link.rs,pair_cli.rs,devices.rs,logging.rs,protocol.rs}`,
  `ipc.rs`; `apps/unipi/src-tauri/src/{net.rs,local_host.rs,host_store.rs}`.
- Do: `~/.oino/host` paths; `Mode::{App,Manual}`; Windows named-pipe local listener +
  client; ALPN `unipi/oino/1`; delete `desktop/linger` subcommands; local-only admin ops
  gate kept.
- Deps: H1.
- Accept: `cargo test -p unipi-host` (ported `local_socket`, `instance`, `pairing_atomic`,
  `transport`, `pair_json`, `admin_ops`), plus a Windows CI run of `local_socket` (coffee).

### WP-H3 — Session hub in the host
- Files: `crates/unipi-host/src/sessions/{hub.rs,card.rs,crash.rs}`; `main.rs` serve wiring
  (extension loader, `ExtensionMode::Desktop`, host-service registration `unipi.cron`,
  `unipi.browser`, `unipi.kanboard`, `unipi.host`); `admin.rs` tools.
- Do: hub create/open/fork/archive/close over `oino-core` `Services` + `AgentSessionRuntime`
  + `InProcessBackend`; lazy `sessions_list` from `SessionManager::list_all` merged with
  live cards; per-session panic isolation (`HubState::Crashed`); safe mode
  (`start-history.json`, 2 fast-fails ⇒ `--no-extensions` + `HostNotice` naming suspects).
- Deps: H2, `specs/core.md` hub/durability landing (session runtime API stable).
- Accept: `cargo test -p unipi-host --test sessions_hub` (faux provider: create, prompt,
  concurrent sessions, crash marks one, others continue, archive → trash).

### WP-H4 — Session protocol + TS client
- Files: `crates/unipi-proto/src/session.rs`; `crates/unipi-host/src/sessions/stream.rs`
  (snapshot, seq ring, coalescer, batching, backpressure tap, `events_open` hub stream);
  `crates/unipi-host/src/protocol.rs` dispatch; `apps/unipi/packages/protocol/src/session.ts`;
  `apps/unipi/src-tauri/src/chat.rs` rewrite.
- Do: §7.3–7.5 exactly: `SessionSnapshot`, `SessionCmd` envelope with `id` correlation,
  `Events` batches, `since_seq` resume, `_oino/hello` implicit capabilities, events-before-
  reply ordering, hub `SessionChanged`/`HostNotice`.
- Deps: H3.
- Accept: `cargo test -p unipi-host --test session_stream` (faux stream of 5k deltas ⇒
  ≤ ceil(events_per_16ms) frames; resume via `since_seq`; reset path; two subscribers);
  `(cd apps/unipi && npm test --workspace)` with new `session.test.ts` fixtures
  (JSON-fixture convention kept).

### WP-H5 — Session takeover
- Files: `sessions/hub.rs` (open-with-takeover, `TakenOver` state, host events), 
  `crates/unipi-host/tests/session_takeover.rs`.
- Do: §8 (5 s yield wait, holder-refusal bounded by core flush points, taken-over notice
  both directions).
- Deps: H4 + `specs/core.md` single-writer lock landing.
- Accept: `cargo test -p unipi-host --test session_takeover`: (a) TUI-style holder process
  (spawn `oino --mode rpc` on the same session file) loses the session on host
  `session_open`; (b) second host opening marks the first's stream `taken_over`.

### WP-H6 — Terminal service
- Files: `crates/unipi-host/src/term.rs` (+ `unipi-proto/src/term.rs`);
  `apps/unipi/src-tauri/src/term.rs` rewrite; `tests/term.rs`.
- Do: §9.3 (host PTY registry, attach, 100 ms resize throttle, idle cap, per-workspace).
- Deps: H2 (independent of H3/H4).
- Accept: `cargo test -p unipi-host --test term` (open→write→read, resize throttle counts,
  detach/reattach continuity, kill, cap eviction).

### WP-H7 — Host services port (fs, git, push/notify, forward, sharing, blob, settings)
- Files: keep-set of §11.1 with changes: `git.rs` (+`TurnFiles`), `sharing.rs` (drop
  Desktop), `settings.rs` new + `host_settings_get/set`, `notify/push/forward/fs/blob`
  path/scope updates; `admin.rs`.
- Deps: H2.
- Accept: ported tests green (`git, fs_scope, forward, push, notify_config, sharing*,
  blob, admin_ops`) + new `settings` test; `git` TurnFiles fixture test.

### WP-H8 — Cron merge
- Files: `crates/unipi-host/src/cron_serve.rs` (adapter), `main.rs` (scheduler tick,
  wake-via-hub), root `Cargo.toml` dep `oino-cron`; remove `oino cron serve` from
  `crates/oino/src/cli*` (per `specs/core.md` coordination); delete
  `apps/unipi`-era cron tests, re-target `cron_ops/cron/cron_notify/cron_wake`.
- Deps: H3 (hub for wake), H7 (notify channels).
- Accept: `cargo test -p unipi-host --test cron_ops` (add/edit/fire with
  `OINO_CRON_BIN` faux, pause/resume, notify test, wake opens hub session).

### WP-H9 — Updater + app lifetime
- Files: `crates/unipi-host/src/update.rs`; `apps/unipi/src-tauri/src/{host_supervisor.rs,
  lifecycle.rs,desktop.rs,app_update.rs,setup.rs,setup/,lib.rs}`; delete `always_on.rs`,
  `screen.rs`.
- Do: §5 (adopt/spawn only, tray label, ask-on-close remembered, start-at-login hidden,
  quit warning), §9.7 (de-service restart, `HostNotice` prompt).
- Deps: H2; UI wiring with the app spec.
- Accept: `cargo test -p unipi-host` (update check faux feed, restart-exit behaviour);
  `cargo test -p unipi-app-src-tauri` equivalents of supervisor/lifecycle unit tests
  (ported `host_supervisor`, `lifecycle` tests minus service branches); manual tray QA
  checklist on Linux + Windows.

### WP-H10 — Integration + performance gates
- Files: `crates/unipi-host/tests/integration.rs` (+ CI wiring in `.github/workflows/ci.yml`
  v2 job), `apps/unipi` e2e reuse of the offline e2e pattern (`local_host.rs` test).
- Do: headless host + client e2e — spawn `unipi-host serve --data-dir <tmp> --relay
  https://127.0.0.1:9 --relay-only`; a Rust test client (frames + proto) over `app.sock`:
  pair (local admin), `sessions_list`, `session_new` with a faux provider env, stream a
  100k-token reply asserting coalescing bounds and `seq` monotonicity, drop and resume with
  `since_seq`, open a terminal, run a cron job, `push_list` round-trip. Performance gates:
  ≤ 1 `Events` frame / 16 ms / stream under streaming; 50-session list < 150 ms cold;
  RSS of 10 idle hub sessions within spec-core §3.2 budget.
- Deps: H1–H8.
- Accept: `cargo test -p unipi-host --test integration -- --ignored` green locally and in
  CI (Linux; Windows secondary), numbers recorded in `docs/dev/v2/qa/host-perf.md`.

---

## 13. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | Core single-writer lock + flush points (spec-core §3.1) not landed when H5 starts | H5 is last of the session WPs; stub behind a core trait (`SessionHandle::open_with_takeover`) with a no-op impl until core lands; acceptance test gated on the real one. |
| R2 | Windows named-pipe local socket is new code (unipi-app never shipped it) | H2 delivers it first with a dedicated test; uid-equivalent = owner-only DACL + `GetNamedPipeClientProcessId` user check; fall back to iroh loopback if the pipe fails. |
| R3 | JSON event overhead on relay links for fast streams | 16 ms batching + delta coalescing measured in H10; kind-3 binary event frame sketched as an additive fallback (§7.4). |
| R4 | iroh version drift (1.3 pinned in unipi-app) vs oino workspace deps | H1 pins the same iroh in the root workspace; upgrade deliberately, once, with the transport tests as the gate. |
| R5 | One host process for everything: a browser/cron panic can kill sessions | panic isolation per session (H3) + safe mode; browser and cron already isolate (browser is a subprocess; cron spawns `oino`). |
| R6 | Cron + old `oino cron serve` both firing during migration | owner lock is per data dir (`~/.oino/cron`), old daemon lives in `~/.pi`-era paths — disjoint; H8 removes the subcommand so new users can't start two. |
| R7 | Terminal scrollback lost on full app restart (host keeps PTY, not buffer) | accepted (ZCode-like); document; optional later `term_capture` (VTE host-side) not in scope. |
| R8 | Coexistence mistakes (touching `~/.unipi`, killing the old host) | paths table §5.1 + a test asserting the host never opens anything under `~/.unipi`; never `pkill` by name — only our child pids. |
| R9 | Session list grouping by cwd hits network-mounted homes / huge session dirs | headers-only listing is core's problem (`list_all`); hub adds caching + `SessionChanged` deltas so listing is rare. |

---

## 14. Owner questions

1. **Tray label** — "UniPi (oino)" while the old app also runs (§5.1), or plain "UniPi" on
   both and live with the ambiguity? (Default taken: "UniPi (oino)".)
2. **Session archive** — archive = move to `~/.oino/trash/` (restorable by hand only), or a
   first-class archived folder browsable in the UI? (Default taken: trash, per ACP precedent.)

No other decisions are open; everything else above follows DIRECTION §2/§4.3 or the cited
sibling specs.
