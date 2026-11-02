# WSL host — plan (later feature, not implemented)

Status: **plan only** (owner-approved 2026-11-02 to build *after* the 1.0.0
release). Nothing here ships in 1.0.0.

## 1. Goal

A Windows user keeps their tools, repos and LLM setup inside WSL (Ubuntu &c.).
They want the **Windows** Unipi desktop app to drive a host that runs **inside
WSL** — agents run Linux tools on the Linux filesystem — with the same
"this PC" experience as the bundled host: no pairing ceremony, works offline,
no relay.

## 2. What already works in 1.0.0 (the interim answer)

- Inside WSL: `curl -fsSL …/install.sh | sh -s -- --host` installs
  `unipi-oino-host` + `oino` + the extensions; with systemd enabled in WSL
  (`/etc/wsl.conf` → `[boot] systemd=true`) the user unit keeps it running.
- The Windows app can already reach that host **as a paired remote PC** over
  iroh: run `unipi-oino-host pair` in WSL, paste the link into
  *Add a PC*. That works (direct UDP between Windows and the WSL VM, relay
  fallback) but it is a "remote PC", not "this PC": it needs pairing, it shows
  as a second computer, and offline it depends on iroh's local discovery.

The feature below makes the WSL host a first-class *local* host.

## 3. Design

### 3.1 Transport: localhost TCP

Today the app reaches its own host over `app.sock` (Unix) or the named pipe
`\\.\pipe\unipi-oino-app` (Windows) — host.md §6.1, `crates/unipi-host/src/local.rs`.
Neither crosses the Windows ↔ WSL boundary:

- a Windows process cannot connect to a Unix socket inside the WSL2 VM
  (WSL2 is a real VM; Windows AF_UNIX does not reach into its kernel), and
  the named pipe does not exist inside WSL;
- WSL2 **does** forward `localhost` TCP: a listener on `127.0.0.1:<port>`
  inside WSL is reachable from Windows as `127.0.0.1:<port>`
  (`localhostForwarding`, default on in NAT mode; native in
  `networkingMode=mirrored`).

So the host gets a third local listener: **loopback TCP**, opt-in.

- `unipi-oino-host serve --local-tcp 127.0.0.1:0` (or `config.json`
  `local_tcp: { enabled, port }`), bound to loopback only — never `0.0.0.0`.
- Same framing as `app.sock` (one connection = one op, `u8 kind · u32 len`
  frames, first frame `Hello`), handled by the same `Handler::handle`.
- The chosen port is written to `<host dir>/local-tcp.json` (0600):
  `{ "port": 47123, "token_sha256": "…" }`.

### 3.2 Trust: a local token instead of peer credentials

`app.sock` trusts the peer **uid** (SO_PEERCRED) and the pipe trusts the
**user SID**. Loopback TCP has neither — any local process (and on a shared
Windows box, any user) can connect. So:

- `serve` generates a 32-byte random **local token** at first start
  (`<host dir>/local-token`, 0600), rotated by `unipi-oino-host local-token rotate`.
- Every TCP connection's `Hello` must carry `local_token`; compared in constant
  time (`subtle`, already a dependency). Wrong/missing → close, no error detail.
- Rate limit failed hellos per minute (reuse the pairing lockout counters'
  shape) so the token cannot be brute-forced locally.
- The token is never on the wire beyond loopback, never logged, never in
  `host_info`.
- New error codes (central `AppErrorCode`, exhaustive): `host.local_tcp.unauthorized`,
  `host.local_tcp.disabled`, `host.local_tcp.bind_failed`.

### 3.3 How the Windows app finds and trusts the WSL host

The Windows app reads the token **through WSL itself**, which proves the user
owns that distro:

1. Discover distros: `wsl.exe --list --quiet` (UTF-16 output).
2. For a chosen distro: `wsl.exe -d <distro> -- unipi-oino-host local-tcp --json`
   → `{ "running": true, "port": 47123, "token": "…", "version": "1.x" }`
   (prints the token only to the calling user, who already owns the host dir).
   Not running → offer *Start host in WSL*:
   `wsl.exe -d <distro> -- systemctl --user start unipi-oino-host`
   (or `unipi-oino-host serve --mode service` detached when WSL has no systemd).
3. Connect to `127.0.0.1:<port>` with the token; cache `{distro, port}` (never
   the token at rest on Windows beyond the app's existing secret store).

App side (src-tauri `host_supervisor.rs`): the plan gains a fourth outcome,
`AdoptWsl { distro }`, chosen only when the user picked a WSL distro as
*this PC's host* in Settings › This computer (default stays the bundled
Windows host). The supervisor never spawns or kills the WSL host itself
beyond the explicit *Start* action.

### 3.4 Paths and files

- Session `cwd`s, Files and git are **Linux paths** (`/home/u/repo`). The app
  shows them as-is; *Open in Explorer* maps to `\\wsl.localhost\<distro>\home\u\repo`.
- Drag-and-drop / attachments from Windows: the app uploads through the host's
  existing blob path (`blob.rs`), not by passing Windows paths.
- Terminals are the WSL host's PTYs (Linux shells), unchanged protocol.

### 3.5 Coexistence

- The bundled Windows host and a WSL host may both run; they have separate
  data dirs and keys. Only one is *this PC's host* in the app at a time; the
  other can still be added as a paired PC.
- Phones pair with whichever host they scan — unchanged.

## 4. Work items (when scheduled)

1. Host: `--local-tcp` listener + `local-token` + `local-tcp --json` command;
   tests mirroring `local.rs` (wrong token refused, loopback-only bind,
   rate limit). No new per-connection buffers beyond the existing frame cap.
2. Proto: `Hello.local_token` (optional field, proto minor bump); error codes.
3. App (Windows only): distro discovery, Settings › This computer › *Host:
   Windows / WSL (<distro>)*, `AdoptWsl` in the supervisor, path display +
   Explorer mapping.
4. Installer: `install.ps1 -HostInstall -Wsl <distro>` runs `install.sh --host`
   inside the distro.
5. QA: Windows 11 VM with WSL2 (NAT and mirrored networking), systemd on/off,
   host restart, token rotation, offline.

## 5. Risks

- **Port forwarding quirks**: `localhostForwarding` can be disabled, and some
  VPN clients break it; the app must fall back to the iroh pairing path with a
  clear message.
- **Shared Windows machines**: loopback is reachable by other Windows users;
  the token is the only gate — keep it 256-bit, constant-time, rate-limited.
- **WSL without systemd**: no restart-on-failure; the app's *Start* action is
  the recovery path.
