# Install and update

Oino ships as two binaries:

| Binary | What it is |
|---|---|
| `oino` | the agent: terminal UI, print / JSON / RPC / ACP modes. Every provider and unipi extension component is compiled into it (`crates/oino-bundle`). |
| `oino-kanboard` | the kanboard CLI and daemon used by the bundled `unipi.kanboard` extension. |

## Installers

```bash
sh scripts/install.sh                 # macOS / Linux (also: curl … | sh)
.\scripts\install.ps1                 # Windows PowerShell
```

Both installers:

1. download `release-manifest.json` of the latest (or `OINO_REF`) GitHub release, fetch the
   binaries for the current target and verify their sha256;
2. otherwise build from source: clone into `~/.cache/oino/source` (or use the current
   checkout), run `cargo xtask wasm` (builds the WASM components into `extensions/dist`) and
   `cargo build --release -p oino -p oino-kanboard`.

| Variable | Meaning |
|---|---|
| `OINO_PREFIX` | install prefix, binaries go to `$OINO_PREFIX/bin` (default `~/.local`) |
| `OINO_REF` | release tag or git ref (source default: `v1-alpha`) |
| `OINO_FROM_SOURCE=1` | skip the release download |
| `OINO_DIR` | source checkout for the fallback |
| `OINO_DRY_RUN=1` | print the commands only |

`scripts/install-smoke.sh` installs into a temporary prefix and runs one prompt against the
faux provider fixture; CI runs it on every push.

## Building by hand

```bash
cargo xtask wasm                              # extensions/dist/<id>/{extension.json, component.wasm, resources}
cargo build --release -p oino -p oino-kanboard
```

Without `extensions/dist` the build still succeeds, but the binary bundles no components
(cargo prints a warning) and you have to install provider packages yourself
([packages.md](packages.md)).

## First run

The bundled components are extracted once (resources only: skills, prompts, docs) to
`~/.oino/bundled/<id>@<version>-<hash>/`; compiled WASM is cached in
`~/.oino/cache/wasmtime/`. The first start therefore takes a few seconds longer than later
ones.

## Updating

Run `oino update` (`--check` only reports): it reads the release manifest, verifies the
checksum and replaces the binary. Re-running the installer works too. The bundled
`unipi.updater` extension checks GitHub releases and tells you when a newer version exists
(`/changelog`).

## Uninstall

Delete `oino` and `oino-kanboard` from the install directory. `~/.oino` contains settings,
credentials (`auth.json`, `secrets.json`), sessions and extension state; remove it only if you
no longer need them.
