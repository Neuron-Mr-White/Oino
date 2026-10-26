# Oino Desktop

Oino Desktop (`apps/oino-desktop`, GPUI via `gpui-kit` 0.8) embeds `oino-core` with every
bundled provider and unipi component, so installing the app is enough — no separate `oino`
install is needed. It shares `~/.oino` (settings, credentials, sessions) with the CLI.

## Build and run

```bash
cargo xtask wasm                                   # components to bundle
cd apps
cargo run -p oino-desktop -- --cwd ~/code/project  # real core
cargo run -p oino-desktop -- --demo                # scripted demo, no API keys
oino-desktop/scripts/build.sh --release            # packages into apps/oino-desktop/dist
```

Linux builds produce a tarball (and AppImage / deb with `cargo-packager`); macOS (`.app`,
`.dmg`) and Windows (NSIS) packaging is configured but has not been run in this alpha. Build
memory is high: use `-j 2` on small machines.

## What it has

- Sidebar of sessions grouped by workspace with running / waiting / unread markers.
- Virtualised conversation with incremental markdown, thinking blocks, tool rows with inline
  diffs and output tails.
- Composer with model and thinking pickers, `/` completion, steering (`enter` while running)
  and follow-ups (`ctrl+enter`), `!cmd`.
- All extension dialogs (including approvals), panels as modals / side-pane tabs, status
  segments; command palette (`ctrl+k`).
- Settings: General, Models, Extensions (which components loaded or failed), Keybindings.

## Known gaps

Buttons inside extension panels do not act yet (no `extension_ui_action` command), no
extension settings hub, no `@file` completion, no split panes / tree view. Details:
[dev/v1/deviations/gpui-desktop.md](dev/v1/deviations/gpui-desktop.md). Oino Mobile
(`apps/oino-mobile`, Android + iOS) is an early preview.
