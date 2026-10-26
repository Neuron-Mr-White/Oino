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
`.dmg`) and Windows (NSIS) packages are built on the build hosts with
`cargo xtask remote-build <host>` (unsigned in this alpha). Build
memory is high: use `-j 2` on small machines.

## What it has

The UI follows [dev/v1/design.md](dev/v1/design.md) (ZCode layout with the unipi palette):

- Sidebar of sessions grouped by workspace with needs-you / working / unread markers, session
  menu (rename, pin, fork, open in new pane, delete), up to four split panes.
- Virtualised conversation with incremental markdown, thinking blocks, one-line tool rows with
  inline diffs and output tails, subagent rows, query rail and find (`ctrl+f`).
- Composer with model and thinking menus, `/` `@` completion from the backend, steering
  (`enter` while running) and follow-ups (`alt+enter`), `!cmd`, context ring.
- Approvals, questions and plan reviews docked above the composer; extension panels as
  overlays, side-pane tabs or the native kanboard board; panel buttons act.
- Side pane (Review, Files, Plan, Agent, Tree, Browser) and bottom frame (Output, Tasks,
  Terminal with real PTYs); command palette (`ctrl+k`); OS notifications; optional mascot window.
- Settings page with the extension settings hub, Models & Providers sign-in, Extensions page.

## Known gaps

Four streaming panes do not always meet the 10 ms p99 frame budget; file checkpoints (undo /
rewind) and single-item queue edits wait for core; Windows and macOS builds are unsigned.
Details: `apps/oino-desktop/DESIGN-STATUS.md` and
[dev/v1/deviations/gpui-desktop.md](dev/v1/deviations/gpui-desktop.md). Oino Mobile
(`apps/oino-mobile`, Android + iOS simulator) embeds the same core; remote attach is not
available yet.
