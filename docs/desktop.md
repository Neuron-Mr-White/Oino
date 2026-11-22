# Desktop and mobile

Oino itself is a terminal tool, but the same core powers desktop and mobile apps through the
Unipi distribution.

## Unipi — the desktop and mobile app

Unipi ([apps/unipi](../apps/unipi)) is a Tauri app built **on** oino end to end: its host is
`oino-core` embedded in a native daemon (`apps/unipi/crates/unipi-host`), and its features are
oino WASM extensions (`extensions/unipi/*`). Installing the app is enough — no separate `oino`
install is needed. It shares `~/.oino` (settings, credentials, sessions) with the CLI.

What the app ships: a sessions sidebar, virtualised conversation with markdown and tool rows,
model / thinking menus, approvals and questions, extension panels, the kanboard board, a side
pane (review, files, plan, browser), real PTY terminals, a command palette, and a settings
screen rendered by the same form engine as the TUI.

Get it with the one-liner installers:

```bash
curl -fsSL https://oino.dev/install.sh | sh -s -- --app   # macOS / Linux (+ AppImage / .deb)
```

or build it from source — see [apps/unipi/BUILDING.md](../apps/unipi/BUILDING.md) for
prerequisites, dev builds with an isolated identity, and bundle targets (Linux AppImage/deb,
macOS dmg, Windows NSIS, Android apk).

## Editors and custom frontends

`oino --mode acp` ([acp.md](acp.md)) speaks the Agent Client Protocol for editors such as Zed;
`oino --mode rpc` ([rpc.md](rpc.md)) exposes a JSON-RPC API on stdio for building your own
frontends.
