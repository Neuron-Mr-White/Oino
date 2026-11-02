# Install and update

Oino ships as one binary plus its extension packages:

| Piece | What it is |
|---|---|
| `oino` | the agent: terminal UI, print / JSON / RPC / ACP modes. |
| `share/oino/extensions` | the extension packages that came with the release (`~/.oino/extensions` shadows them per id; `OINO_DIST_EXTENSIONS` overrides everything). |
| Unipi desktop app | optional: the Tauri app around the same host ([docs/unipi.md](unipi.md), [docs/desktop.md](desktop.md)). |

Unipi's native extras (e.g. the `unipi-kanboard` CLI) are not on PATH: they ship
inside their extension packages in the share tree.

## One-liners

```bash
curl -fsSL https://oino.dev/install.sh | sh                # macOS / Linux
curl -fsSL https://oino.dev/install.sh | sh -s -- --app    # … + the Unipi desktop app
```

```powershell
irm https://oino.dev/install.ps1 | iex                      # Windows
irm https://oino.dev/install.ps1 -OutFile install.ps1; .\install.ps1 -App
```

Until oino.dev serves them, the same scripts work from the repo
(`sh scripts/install.sh`, `.\scripts\install.ps1`) or via raw GitHub URLs on the
`v2` branch. Every download is verified against the release's `SHA256SUMS`.

## What gets installed

| | Binary | Extensions | PATH |
|---|---|---|---|
| macOS / Linux | `~/.local/bin/oino` | `~/.local/share/oino/extensions` | added to `~/.local/bin` (a marked block in `~/.profile`, when it exists) |
| Windows | `%LOCALAPPDATA%\oino\bin\oino.exe` | `%LOCALAPPDATA%\oino\bin\extensions` | added to the per-user PATH |

Linux release binaries are **static musl** — one binary per arch that runs on
glibc and musl distributions alike. `oino update` resolves the same artifact
via `release-manifest.json`.

### `--app` / `-App`: the Unipi desktop app

| OS | Steps |
|---|---|
| Linux | AppImage into `~/.local/bin/UniPi.AppImage` + `~/.local/share/applications/unipi.desktop` + icon (pulled out of the AppImage). `--deb` instead installs the release `.deb` through `apt-get` (needs `sudo`) when you prefer the package. |
| macOS | mounts the `universal.dmg`, copies `UniPi.app` to `/Applications`, then clears the `com.apple.quarantine` xattr **with a printed notice** — the build is ad-hoc signed, not notarized; removing quarantine is your trust decision ([apps/unipi/BUILDING.md](../apps/unipi/BUILDING.md)). First launch may still need right-click → Open. |
| Windows | runs the NSIS installer silently (`/S`, per-user, no admin). |

## Fallbacks and knobs

Without a matching release binary the installers build from source: clone into
`~/.cache/oino/source` (or use the current checkout), run `cargo xtask wasm`
(builds the WASM components into `extensions/dist`) and
`cargo build --release -p oino-cli`, then stage the same share tree.

| Variable | Meaning |
|---|---|
| `OINO_INSTALL_BASE_URL` | asset base instead of the GitHub release permalink; may be `http(s)://…` or `file:///…` (used by `scripts/install-test.sh` against a fake release dir) |
| `OINO_PREFIX` | install prefix (default `~/.local` / `%LOCALAPPDATA%\oino`) |
| `OINO_REF` | release tag or git ref (source fallback default: the default branch) |
| `OINO_FROM_SOURCE=1` | skip the release download |
| `OINO_DIR` | source checkout for the fallback |
| `OINO_DRY_RUN=1` | print the plan, touch nothing |

`scripts/install-test.sh` runs the installer against a local fake release in a
throwaway HOME (CI runs it on every push); `scripts/install-smoke.sh` does a
full source install and one faux-provider prompt.

## Building by hand

```bash
cargo xtask wasm                              # extensions/dist/<id>/{extension.json, component.wasm, resources}
cargo build --release -p oino-cli
```

The Unipi desktop app builds with `mise run unipi:dev` / `mise run unipi:build`
([apps/unipi/BUILDING.md](../apps/unipi/BUILDING.md)); macOS and iOS specifics
(including signing) are documented there.

## Updating

Run `oino update` (`--check` only reports): it reads the release manifest,
verifies the checksum and replaces the binary. Re-running the installer works
too. The bundled `unipi.updater` extension checks GitHub releases and tells you
when a newer version exists (`/changelog`).

## Uninstall

Delete `oino` from the install directory (and the share tree under
`~/.local/share/oino` / `%LOCALAPPDATA%\oino`). `~/.oino` contains settings,
credentials (`auth.json`, `secrets.json`), sessions and extension state; remove
it only if you no longer need them.
