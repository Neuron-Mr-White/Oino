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
captures `oino-desktop/screenshots/*.png`.

## Keys

Ctrl/Cmd+K palette (`>` commands, `#` sessions) · +N new session · +O new in folder · +L model ·
+T thinking · +B sidebar · +J side pane · +. abort · +, settings. Composer: Enter send (steer while
running) · Ctrl+Enter follow-up · Shift+Enter newline · ↑/↓ history · Tab completes `/command` ·
Esc aborts.
