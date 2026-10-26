# Oino apps (GPUI)

Separate cargo workspace (D3) pinned to `gpui-kit =0.8.0` / `gpui-pre =0.3.9` (D17).

| Crate | What |
|---|---|
| `oino-ui` | toolkit-free view models (`model::*`, tested with `--no-default-features`) + GPUI views (`views::*`, feature `views`) + tokio↔GPUI bridge (`rt::Tokio`) + `host::SessionHost` contract |
| `oino-desktop` | window shell: in-process `oino-core` with bundled components (`EmbeddedHost`), `--demo` scripted backend, preferences, packaging |

## Build and run (Linux)

```sh
# system deps (Ubuntu/Debian names)
sudo apt install clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libssl-dev libzstd-dev libasound2-dev libvulkan1
cargo xtask wasm                                   # bundle input: extensions/dist (repo root)
cd apps
cargo run -j 2 -p oino-desktop -- --cwd ~/some/project   # real core, bundled providers + unipi
cargo run -j 2 -p oino-desktop -- --demo                 # scripted demo, no API keys
```

Tests and gates:

```sh
cd apps
cargo fmt --all --check
cargo clippy -j 2 --workspace --all-targets -- -D warnings -W clippy::pedantic
cargo test -j 2 --workspace
cargo test -p oino-ui --no-default-features         # view models without GPUI
```

## Packaging

`oino-desktop/scripts/build.sh [--release] [--universal] [--no-wasm] [--formats …]` builds the
components, the app and the `oino` CLI, then packages into `oino-desktop/dist/`:
Linux tar.gz (+ AppImage/deb), macOS `.app`/`.dmg` (`--universal` = lipo), Windows NSIS
(Git Bash on the host). Installers need `cargo install cargo-packager --locked`
(config: `[package.metadata.packager]` in `oino-desktop/Cargo.toml`).

`oino-desktop/scripts/screenshot.sh [out]` renders the demo headless (Xvfb + lavapipe) and
captures `docs/dev/v1/design/shots/desktop/*.png`.

## Keys

Ctrl/Cmd+K palette (`>` commands, `#` sessions) · +N new session · +O new in folder · +L model ·
+T thinking · +B sidebar · +J side pane · +. abort · +, settings. Composer: Enter send (steer while
running) · Ctrl+Enter follow-up · Shift+Enter newline · ↑/↓ history · Tab completes `/command` ·
Esc aborts.

## Design system (design-desktop)

`apps/oino-ui` follows [docs/dev/v1/design.md](../docs/dev/v1/design.md):

| Module | Contents |
|---|---|
| `design::{tokens, type_scale, tones, brand, perf}` | Oino Light/Dark palettes (Zai values), spacing/radii/sizes, text roles, OUS tone map, Unicrab data, frame-time ring (toolkit-free, tested) |
| `design::{theme, icons, syntax, ticker}` | `OinoTheme` global + derived gpui-kit theme, bundled Inter / JetBrains Mono, Lucide `OinoAssets` (vocabulary only + gpui-kit defaults), highlight theme, shared 12 fps ticker that stops when idle |
| `components::*` | Button, controls, menu/popover, tabs, badges, toasts, dialogs, sidebar, header, timeline rows, composer + completion menu, dock (widgets, hints, queue, run state), docked interactions (approval / question / plan review / dock dialog), meta row, status panel, query rail + find bar, side and bottom frames, OUS renderer (+ kanban board), palette, model menu, settings parts, Unicrab |
| `model::{timeline, interaction, status, kanban, palette, composer}` | toolkit-free view models for the above |
| `views::{shell, session_pane, settings_page, board, extensions_page, onboarding}` | D1–D14 screens |

`design::tests::no_literal_styles` fails on `hsla(`/`rgb(`/`px(<number>` in `src/views`,
`src/components` (and `src/mobile`, currently excepted). Colors come only from `OinoTheme`.

### Screenshots and perf

```sh
cargo build -j 8 -p oino-desktop
oino-desktop/scripts/screenshot.sh                       # → docs/dev/v1/design/shots/desktop/*.png
oino-desktop/scripts/screenshot.sh /tmp/shots "" 07-board  # one shot
cargo build -j 8 --release -p oino-desktop
oino-desktop/scripts/perf.sh target/release/oino-desktop 25 4   # 4 streaming panes, p99 budget 10 ms
```

Demo hooks (`--demo`): `OINO_DESKTOP_DEMO_SCENE=welcome|run|approval|question|plan|board|info|stream`,
`OINO_DESKTOP_PANES=N`, `OINO_DESKTOP_OPEN=palette|settings[:ns]|board|extensions|side|model`,
`OINO_DESKTOP_THEME=light|dark`, `OINO_DESKTOP_SIZE=WxH`, `OINO_DESKTOP_DEMO_PACE=N`,
`OINO_LOCAL_HOUR`. Frame times: `OINO_FRAME_LOG=1` logs p50/p99/max every 120 frames to
`~/.oino/logs/desktop.log`; `Ctrl+Alt+Shift+F` (or `OINO_FRAME_OVERLAY=1`) shows the overlay.
