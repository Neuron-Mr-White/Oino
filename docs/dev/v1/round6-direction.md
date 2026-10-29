# Round 6 direction — remote anywhere (owner decision 2026-10-29)

Status: decided by the owner, spec to follow (round 6, after alpha.5). Supersedes the
TCP + TLS + token transport in [modes.md §5](modes.md) for remote use.

## Decisions

1. **Remote anywhere, opt-in.** Settings gets a "Remote access" toggle (off by default).
   Turning it on **asks the user for permission** to run Oino as a background daemon/service
   (explain what it does: keeps running after the window closes and after logout, starts at
   boot, reachable by paired devices from anywhere). Only after consent is the service
   installed and started; turning it off stops and uninstalls it. Never enable silently.
2. **Transport: iroh** (QUIC, stable endpoint id from a persistent secret key, n0 relays +
   DNS/pkarr discovery, hole punching to direct LAN/tailnet/public paths, relay fallback;
   optional self-hosted relay, relay-only mode). Works with or without a tailnet. Reference
   implementation: unipi `unipi-app/crates/host` (`endpoint.rs`, `pair_link.rs`,
   `pairing.rs`, `local.rs`, `main.rs` serve) and `apps/mobile/src-tauri/src/net.rs`,
   `host_supervisor.rs` (MIT).
3. **`oino serve`** is that daemon: it hosts the session hub and carries the existing RPC /
   `_oino/*` frontend API over iroh streams, plus a same-user local socket for apps on the
   same machine (works offline).
4. **Service install** per OS: systemd user service + linger on Linux (polkit asks once),
   LaunchAgent on macOS, a per-user startup task/service on Windows; supervised with restart
   backoff (1, 2, 4 … 30 s).
5. **Pairing**: QR + link with a minimal endpoint ticket (id + relay URL), short-lived
   pairing code with lockout, paired-devices list with revoke, in Settings → Remote access.
6. **Clients**: the central host switcher (desktop) and the phone attach through it;
   "Current PC" remains the default.
7. **Offline notifications**: ntfy / UnifiedPush with end-to-end encrypted payloads.
8. **Mobile catch-up**: bring the round-5 desktop UI to mobile, rebuild Android, test on a
   plugged-in phone; iOS device support (wasmtime Pulley) as a stretch.

## Open (owner to decide)

- Which unipi-app host features beyond chat/sessions should Oino's remote offer
  (port forwarding to the phone, push file to phone, remote host browser view, desktop
  streaming with control, git, files over remote, "ask the PC to turn this on" consent
  requests)? See GAPS.md.
