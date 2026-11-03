# Building Unipi

How to build the Unipi desktop app (and phone app) from this checkout. For what
ships in a release, see `.github/workflows/release.yml`.

## Never collide with the installed app

The installed Unipi release uses the identifier **`dev.unipi.app`** and keeps
its data in `~/.oino` (host state: `~/.oino/host`). Locally built dev/test
artifacts must never share either, or a test run would lock/overwrite the real
app's data. That is why every local task here merges
[`tauri.dev.conf.json`](tauri.dev.conf.json):

| | Installed release | Local dev/test build |
|---|---|---|
| identifier | `dev.unipi.app` | `dev.unipi.app.dev` |
| productName (identity, not shown) | `UniPi Oino` desktop, `Unipi` phones | `UniPi Dev` (own app-data dir) |
| visible name | `Unipi` | `Unipi` |
| host data dir | `~/.oino/host` | `apps/unipi/target/unipi-dev/host` (`OINO_HOST_DIR`) |

CI release builds are the **only** `dev.unipi.app` builds.

## The visible name is "Unipi"; productName is identity

On desktop, tauri derives install identities from `productName`, so it stays
`UniPi Oino` (dev: `UniPi Dev`) and the visible name is set separately:

- **Linux**: the deb `Package:` is `kebab(productName)` = `uni-pi-oino`
  (no override in tauri-bundler 2.10), plus `/usr/share/applications/UniPi Oino.desktop`,
  `/usr/lib/UniPi Oino/` and the `UniPi Oino_<v>_amd64.{deb,AppImage}` file
  names. Renaming would ship a new package `unipi` that fights `uni-pi-oino`
  over `/usr/bin/unipi-oino`. The menu entry's `Name=Unipi` comes from
  `src-tauri/linux/unipi.desktop` (`bundle.linux.deb.desktopTemplate`, also
  used by the AppImage). Binaries stay `unipi-oino` / `unipi-oino-host` / `oino`
  (`mainBinaryName`, sidecar), data stays under `dev.unipi.oino` / `~/.oino`.
- **macOS**: `src-tauri/Info.plist` sets `CFBundleName`/`CFBundleDisplayName`
  to `Unipi`; the bundle stays `UniPi Oino.app` because `Unipi.app` would be
  the old UniPi app's `/Applications/UniPi.app` on case-insensitive APFS.
- **Windows**: NSIS uses `productName` for the install dir
  (`%LOCALAPPDATA%\UniPi Oino`), the uninstall key and the shortcut names;
  `Unipi` would land in the old app's `%LOCALAPPDATA%\UniPi` (NTFS is
  case-insensitive), so the Start menu entry still reads `UniPi Oino` until a
  custom NSIS template overrides the shortcut name.
- **Phones**: `productName` is only the display name there (the Android/iOS
  identity is `identifier` + the Cargo crate name), so it is `Unipi`; the
  Android launcher label comes from `@string/app_name` (release) or the
  `appLabel` placeholder (`UNIPI_ANDROID_ID_SUFFIX=.dev` builds,
  `dev.unipi.app.dev`), both `Unipi`.
- Window title, tray, menus and every in-app string say `Unipi`.

## Prerequisites

- Rust per the repo's `rust-toolchain.toml` (+ `rustup target add wasm32-wasip2`
  — the extension components are built into `extensions/dist` by
  `cargo xtask wasm`).
- Node ≥ 22 + npm (the frontend builds with vite; `npm ci` in `apps/unipi`).
- `cargo tauri-cli` v2 (`cargo install tauri-cli --version ^2`).
- Linux: webkitgtk dev packages —
  `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`
  (Ubuntu 22.04 names; 24.04: `libwebkit2gtk-4.1-dev` too).
- The static-musl host sidecar on Linux is built with **cargo-zigbuild**
  (`pip install cargo-zigbuild` + a `zig` on PATH) — see
  [`src-tauri/unipi-host-sidecar.sh`](src-tauri/unipi-host-sidecar.sh).
- macOS: Xcode Command Line Tools; for the universal dmg both
  `aarch64-apple-darwin` and `x86_64-apple-darwin` targets.

## Dev loop

```sh
mise run unipi:dev        # = scripts/unipi-dev.sh
```

Builds the debug `unipi-host`, stages it as the sidecar
(`src-tauri/binaries/unipi-oino-host-<triple>`, required by tauri's build.rs),
builds `extensions/dist` if missing, sets `OINO_HOST_DIR` to the scratch dir
and runs `cargo tauri dev --config tauri.dev.conf.json`.

`mise run start` runs the same dev loop in the background (log and pid under
`apps/unipi/target/unipi-dev/`, log capped at 5 MB). It first stops a previous
run — its process group, the dev host holding `$OINO_HOST_DIR/host.lock`, and a
process of this checkout still listening on the dev port 1420 — so running it
twice leaves one clean set. A foreign process on the port is reported, never
killed. `mise run stop` stops it all; `mise run status` shows what runs.

## Local release bundle

```sh
mise run unipi:build      # = scripts/unipi-build.sh
```

Same steps as the release workflow for the current OS — Linux `.deb` +
`.AppImage`, macOS universal `.dmg`, Windows NSIS `.exe` — but with the
**dev identifier** (see above), and the extension packages staged in as a
bundle resource. Bundles land under
`src-tauri/target/[<target>/]release/bundle/`.

The bundled extension tree lands in each installer's resource dir
(`Contents/Resources/extensions` on macOS; `/usr/lib/UniPi/extensions` in the
deb/AppImage; `%LOCALAPPDATA%\UniPi` on Windows). Pointing the running app/host
at that tree (e.g. `OINO_DIST_EXTENSIONS` when spawning the host sidecar) is
the dist.md WP-D1/D5 runtime wiring and still open — today the app still
installs its extensions via the setup engine (npm) into `~/.oino`.

## macOS: build from source (unsigned)

There is no Apple Developer account, so release and local builds are **ad-hoc
signed only** (`codesign -s -`), never notarized:

- Gatekeeper flags a downloaded dmg on first launch: right-click
  `UniPi.app` → **Open** (once). The installer script
  (`install.sh --app`) instead clears the `com.apple.quarantine` xattr and says
  so — that is your trust decision.
- In-app updates (once the updater channel is wired) install without the
  quarantine xattr, so post-update launches do not re-prompt.

```sh
mise run unipi:build              # universal dmg into src-tauri/target/…/bundle/dmg
# or manually:
(cd apps/unipi && cargo tauri build --config target/tauri-local.conf.json \
   --target universal-apple-darwin --bundles dmg)
```

## iOS: build it yourself (nothing shipped)

The iOS build is build-compatible (Xcode project skeleton under
`src-tauri/gen`, `tauri.ios.conf.json`, `Info.ios.plist`, and the
`SystemConfiguration` link fix in `src-tauri/build.rs`) but **no ipa is
released**: it can only be installed through your own signing setup.

1. A Mac with **Xcode** and an **Apple Developer account / signing team**.
2. `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
3. Point the project at your team — Xcode → signing & capabilities of the
   generated project (or `DEVELOPMENT_TEAM=…` in the build), and adjust
   `tauri.ios.conf.json` if you need a different bundle id than
   `dev.unipi.app.dev` (dev identity).
4. Build:

```sh
cd apps/unipi
cargo tauri ios build            # device build (signed with your team)
cargo tauri ios dev              # simulator
```

An ad-hoc/development-signed ipa installs only on registered devices; that is
why the release pipeline skips iOS entirely.

## Android

CI builds a universal `.apk` (release.keystore via repo secrets, or a generated
debug key when they are absent). Locally:

```sh
cd apps/unipi
cargo tauri android build --apk --config tauri.dev.conf.json
# apk: src-tauri/gen/android/app/build/outputs/apk/universal/release/
```

Needs `ANDROID_HOME` (SDK + NDK) and a JDK 17; signing env for a release key:
`UNIPI_ANDROID_KEYSTORE` (path), `UNIPI_ANDROID_KEYSTORE_PASSWORD`,
`UNIPI_ANDROID_KEY_ALIAS`, `UNIPI_ANDROID_KEY_PASSWORD`.

Release builds run R8 in full mode (AGP 9). Libraries that create classes by
reflection need keep rules in `src-tauri/gen/android/app/proguard-rules.pro`
(tracked, unlike the regenerated `proguard-tauri.pro`). Full mode does not keep
the no-arg constructor of a class kept with a bare `-keep class`. ML Kit's
component registrars lost theirs, and QR scanning crashed on the first camera
frame. Debug builds don't minify, so they never show this. Check release-only
crashes against `app/build/outputs/mapping/<variant>/usage.txt`, which lists
what R8 removed.
