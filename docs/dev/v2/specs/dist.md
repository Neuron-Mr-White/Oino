# Installers and distribution — spec (v2)

Status: **spec, implementation-ready** (dist work package, DIRECTION §8 + §1). Inputs studied:
[DIRECTION.md](../DIRECTION.md) §1/§8, current tooling (`scripts/install.sh`, `scripts/install.ps1`,
`scripts/install-smoke.sh`, `xtask/src/{main,remote}.rs`, `scripts/release/build-host.sh`,
`.github/workflows/ci.yml`, `crates/oino-bundle`, `extensions/Cargo.toml`, `extensions/dist`),
the imported app at `apps/unipi` (`apps/mobile/src-tauri/tauri.conf.json`, `Cargo.toml`,
`unipi-host-sidecar.sh`, `gen/android/app/build.gradle.kts`, `package.json`), and — read-only —
the unipi-app 1.0.0 release tooling in `/home/oi/Projects/Personal/archived/unipi/unipi-app`
(`scripts/release.sh`, `scripts/mac-build.sh`, `scripts/setup-release-tools.sh`,
`releases/1.0.0/`).

Rule carried over from DIRECTION: **one installer per platform, nothing else to install.**
Everything below is decided unless marked *(open)*.

---

## 1. Product identity and versioning

| Item | Value |
|---|---|
| Product name | **UniPi** (app), Tauri identifier **`dev.unipi.oino`** |
| Package/OS names | deb package `unipi-oino`; Android `applicationId dev.unipi.oino`; app bundle `UniPi.app` / `UniPi.exe`; install dir `%LOCALAPPDATA%\UniPi` |
| Distribution version | one semver `X.Y.Z` shared by app, `unipi-host`, proto and every installer, **starting at `2.0.0`** (clean break from archived unipi-app 1.0.0; its channel is frozen) |
| Version bump | `cargo xtask dist version <X.Y.Z>` rewrites `apps/unipi/apps/mobile/src-tauri/tauri.conf.json`, `apps/unipi/{crates/host,crates/proto,apps/mobile/src-tauri}/Cargo.toml` (same four files unipi-app's release.sh bumped) and commits |
| Android versionCode | `major*1_000_000 + minor*1_000 + patch` (cargo-tauri derives it from the conf version; `2.0.0` → `2000000`). Verified in smoke tests |
| Git tag | `v{X.Y.Z}` on `v2` (e.g. `v2.0.0`); one GitHub Release per tag carrying **all** installers + `latest.json` + `release-manifest.json` |
| `oino` core crates | keep their own semver (today `1.0.0-alpha.5`), bumped independently; `oino --version` reports it and it is recorded in each artifact's `build-info.txt` |
| Extension packages | per-package semver in `extension.json` (e.g. `unipi.kanboard 3.x`); a release pins the `extensions/dist` snapshot via `checksums.json` at build time |

Decision: tag names `v{X.Y.Z}` continue the existing CLI-release flow (`ci.yml` already fires on
`v*`); the same Release gains the installers, so there is exactly one release channel.
*(Open Q1: repo visibility — see §12.)*

## 2. Artifacts (exactly one installer per platform)

| Platform | Host that builds it | Artifact | Name |
|---|---|---|---|
| Windows x64 | `coffee` (`ssh coffee`, Windows, native MSVC) | NSIS per-user installer | `UniPi_{v}_x64-setup.exe` |
| macOS universal | `desmondgiam` (ssh alias today: `ssh mac`, user `desmondgiam`) | `.dmg` + updater tarball | `UniPi_{v}_universal.dmg`, `UniPi_{v}_universal.app.tar.gz` (+`.sig`) |
| Linux amd64 | local (this PC) | `.deb` (post-processed, §5) | `unipi-oino_{v}_amd64.deb` |
| Android arm64 | local, phone plugged in over USB | signed `.apk` | `unipi-oino-{v}.apk` |
| iOS | — | **not shipped**; build-compatible only: `cargo check -p unipi-mobile --manifest-path apps/unipi/src-tauri/Cargo.toml --target aarch64-apple-ios` on desmondgiam stays green; `tauri.ios.conf.json`/`Info.ios.plist` kept in tree |

Secondary (unchanged, not an "installer"): the CLI channel `install.sh` / `install.ps1` +
`release-manifest.json` keeps shipping the bare `oino` binary per target (§7.4).

### 2.1 macOS signing / notarization status

- **No Apple Developer account** ⇒ no Developer ID, no notarization. What is possible and what
  we do: **ad-hoc codesign** (`codesign --force -s - --identifier dev.unipi.oino …`) for every
  Mach-O in the bundle (app binary, `oino`, `unipi-host` sidecars), exactly like unipi-app 1.0.0
  (`scripts/mac-build.sh`).
- Gatekeeper: first launch needs **right-click → Open** (documented in the dmg README +
  release notes). Updates installed by the in-app updater do **not** carry a quarantine xattr,
  so post-update launches do not re-prompt (proven by unipi-app's updater in production).
- The desktop updater's minisign signatures are independent of Apple signing; ad-hoc builds
  self-update fine (§8).
- If an account is bought later: drop in Developer ID Application signing + `notarytool`
  stapling in `scripts/dist/macos-build.sh`; nothing else changes.

Windows is unsigned (same as unipi-app today): SmartScreen may warn on first run; NSIS
per-user install needs no admin. Accepted.

## 3. Bundle contents (what "nothing else to install" means)

Every installer carries, per platform arch:

1. **The Tauri app** (`UniPi.exe` / `UniPi.app` / `/usr/bin/unipi-mobile` + `.desktop` + icons;
   `unipi-oino.apk` on Android — a pure remote client, nothing else below ships on the phone).
2. **`unipi-host`** as the app's **sidecar** (`bundle.externalBin`, the imported
   `unipi-host-sidecar.sh` pattern): `Contents/Resources/` on macOS (universal lipo + both
   thin triples), `%LOCALAPPDATA%\UniPi\` on Windows, `/usr/lib/unipi-oino/` on Linux.
   In v2 the host links oino-core in-process (DIRECTION §2). **It is never put on PATH and
   never installs a service** — the app spawns/adopts it (tray lifetime, owner Q4). This is
   what guarantees coexistence with the owner's live old `~/.local/bin/unipi-host`.
3. **`oino` CLI/TUI binary**, put on PATH per OS (§4).
4. **Unipi extension packages** — the `extensions/dist` snapshot for the release:
   all groups except fixtures (`oino.fixture.*` excluded). That is the `unipi.*` set plus the
   provider extensions (`oino.anthropic`, `oino.google`, `oino.openai-codex`,
   `oino.openai-compat`, `oino.openai-responses`, `oino.azure`, `oino.bedrock`,
   `oino.router`). Layout per package: `extension.json`, `component.wasm`, `resources/`
   (skills, prompts, docs), plus new **`bin/<cli>`** native CLIs (§6). They are installed into
   the **distribution share dir** (§5) that `oino`, the TUI and `unipi-host` all scan.
5. **`unipi-kanboard`** (the kanboard CLI, renamed from `oino-kanboard` by the §3.4 work):
   shipped **inside the `unipi.kanboard` package** at `bin/unipi-kanboard-<triple>` — not on
   PATH; the extension resolves it package-relative via `process.exec`
   (`OINO_KANBOARD_BIN` → package `bin/` → PATH).

**Not bundled:** CloakBrowser. unipi-host downloads and Ed25519-verifies it on first use into
`~/.oino/host/cloakbrowser/<version>/` (ported `cloak.rs`; never redistributed — license and
fingerprint hygiene, DIRECTION §6). The installers contain no browser binaries.

## 4. `oino` on PATH, per OS

| OS | Mechanism | Undo |
|---|---|---|
| Windows | NSIS installer hook (`bundle.windows.nsis.installerHooks`, `installMode: "currentUser"`) writes the install dir into the **per-user PATH** (`HKCU\Environment\Path`, broadcast `WM_SETTINGCHANGE`); `oino.exe` sits in the install dir | uninstaller hook removes the entry (idempotent: only the string we added) |
| macOS | app first-run (SetupWizard) + Settings → "Command line": symlink `oino` (and nothing else) from `UniPi.app/Contents/Resources/bin/oino`. Default target **`~/.local/bin/oino`** (no admin; prompt if not on PATH); optional `/usr/local/bin/oino` via one `osascript` admin prompt | Settings action "Remove command line tools" deletes the symlink it created (path recorded in app settings) |
| Linux | the deb ships `/usr/bin/oino` directly | `dpkg -r` |

The TUI itself needs no terminal integration beyond PATH; no shell rc edits anywhere
(no writing `~/.bashrc` from installers).

## 5. Distribution share dir and discovery

New concept (replaces "extensions compiled into the `oino` binary", DIRECTION §1): the
installer owns an immutable, versioned tree of extension packages — the **share dir**:

| OS | Share dir |
|---|---|
| Linux | `/usr/share/oino/extensions/<id>/…` |
| macOS | `/Applications/UniPi.app/Contents/Resources/extensions/<id>/…` (exe-relative `<prefix>/extensions`) |
| Windows | `%LOCALAPPDATA%\UniPi\extensions\<id>\…` |

Discovery moves **into `oino-core`** (today it lives in `crates/oino/src/loader.rs`, which the
host never links). Scan order, lowest → highest precedence, reusing the loader's existing
same-id replacement semantics:

1. distribution share dir (resolved relative to the running exe, product-neutral: `<exe>/../share/oino/extensions`
   heuristics per OS above; overridable with **`OINO_DIST_EXTENSIONS=<dir>`** for dev/tests);
2. `~/.oino/extensions` (user agent dir — a user-installed package with the same id shadows
   the distribution one);
3. trust-gated `<cwd>/.oino/extensions`;
4. `-e/--extension` (wins over everything, as today).

The bundled-components table (`oino-bundle`) is removed from the `oino` binary in the same
change (it already exists only as the loader's first source; `OINO_BUNDLED` retires).
`OINO_AGENT_DIR` keeps selecting `~/.oino`. `unipi-host` gets the same discovery by linking
core; it passes its sidecar-adjacent share dir explicitly when it adopts packages.

**CLI-only installs** (no app): `install.sh` / `install.ps1` additionally fetch
`unipi-extensions-{v}-{target}.tar.gz` (the share-dir tree, built by the same step) and unpack
it into `~/.oino/extensions` — one extra command, documented, keeps the blank-core philosophy
for source installs.

## 6. Extension package format additions

- `extension.json` gains an optional **`"bin": ["unipi-kanboard"]`** field. For each listed
  name the dist build stages `bin/<name>` (a native binary for the installer's arch) into the
  package directory.
- `xtask wasm` stays wasm-only; the new **`cargo xtask dist stage`** builds the native CLIs
  (`cargo build --release -p oino-kanboard` → renamed `unipi-kanboard` per §3.4) per target and
  stages them into a **staging copy** of `extensions/dist` (`target/dist-stage/extensions/…`),
  then extends the per-package `checksums` map and `checksums.json` with `bin/<name>` entries.
  `extensions/dist` itself (committed) stays wasm+resources only.
- The kanboard extension's `process.exec` allowlist/paths move from `oino-kanboard` to
  `unipi-kanboard` in the same §3.4 change; resolution order `OINO_KANBOARD_BIN` (env allow)
  → `<package dir>/bin/unipi-kanboard` → PATH.

## 7. Build hosts and commands

Canonical releases are built **by the owner's machines**, driven from this PC through the
existing xtask ssh machinery (`xtask/src/remote.rs`: git-bundle sync + `extensions/dist`
tarball + detached build + artifact fetch). Pattern (extend, don't replace):

```sh
mise run dist -- version 2.0.0          # bump + commit (see §1)
mise run dist -- wasm                   # cargo xtask wasm (local; needs wasm toolchain)
mise run dist -- linux                  # local: CLI + host + tauri deb + post-process + smoke
mise run dist -- windows                # on coffee over ssh
mise run dist -- macos                  # on desmondgiam over ssh
mise run dist -- android                # local build; phone plugged in via adb
mise run dist -- all                    # the four, in dependency order, then assemble release/
mise run dist -- publish [--dry-run]    # GitHub Release + latest.json + manifest (§8)
```

Under the hood `cargo xtask dist <platform>` = `remote-build`-style sync of the repo
(including `apps/unipi`; **excludes** `node_modules`/`target`) + `extensions/dist` tarball +
the staged native CLIs, then runs `scripts/dist/<platform>-build.sh` on the host. Host
prerequisites (documented in `scripts/dist/README.md`, one-time):

| Host | Needs |
|---|---|
| local (Linux) | rust (rust-toolchain.toml), wasm32-wasip2, wasm-tools, `cargo-tauri-cli`, node ≥ 22 + npm, Android SDK + `UNIPI_ANDROID_*` signing env (credentials dir, never committed), `adb`, podman (for the deb smoke container) |
| coffee (Windows) | rust MSVC + `cargo-tauri-cli`, node + npm, Git Bash (`OINO_REMOTE_BASH` default in remote.rs) |
| desmondgiam (macOS) | rust + both darwin targets, `cargo-tauri-cli`, node + npm, Xcode CLT (for the iOS check) |

Placeholder patterns (exact paths serials TBD on first run, recorded into
`scripts/dist/README.md`): `ssh coffee` / `ssh mac` names come from `~/.ssh/config`
(`coffee` = 100.78.125.45, `mac` = desmondgiam@100.64.52.29); android uses
`adb -d install -r …` against the plugged-in phone (serial captured by `adb devices`).

Per-platform build scripts (new, adapted from the archived unipi-app ones, MIT, credited in
the header):

- `scripts/dist/linux-build.sh` — `unipi-host-sidecar.sh` (x86_64-linux), `cargo tauri build
  --bundles deb`, then **post-process** (`scripts/dist/deb-postprocess.sh`): unpack with
  `dpkg-deb -x`, rename package to `unipi-oino`, add `/usr/bin/oino`, `/usr/share/oino/extensions`
  (staged tree + native CLI), conffiles-free `postinst` (only `update-desktop-database`),
  repack with `dpkg-deb -b`. Tauri's own deb cannot place files in `/usr/bin` or rename the
  package; repacking is ~30 lines and the archived release.sh already unpacks debs the same way.
- `scripts/dist/windows-build.sh` (runs on coffee via Git Bash) — `unipi-host-sidecar.sh`
  natively for `x86_64-pc-windows-msvc` (no cargo-xwin cross needed on a real Windows host),
  copy `oino.exe` + staged `extensions/` into `src-tauri/binaries/…` as bundle resources,
  `cargo tauri build --bundles nsis` with the `installerHooks` nsh from §4.
- `scripts/dist/macos-build.sh` — port of archived `scripts/mac-build.sh`: universal
  `unipi-host` + `oino` (lipo of both darwin targets, ad-hoc codesign each), all three
  sidecar names into `binaries/`, `cargo tauri build --target universal-apple-darwin
  --bundles app`, ad-hoc sign the `.app`, `hdiutil` dmg (the manual folder+Applications-link
  trick — tauri's `bundle_dmg.sh` needs Finder and fails over ssh), `.app.tar.gz` updater
  artifact, `cargo check --target aarch64-apple-ios` build-compat check.
- `scripts/dist/android-build.sh` — source `UNIPI_ANDROID_*` env, `cargo tauri android build
  --apk --target aarch64`, `apksigner verify --print-certs` against the pinned key
  fingerprint, `aapt2 dump badging` versionCode check (both checks ported from release.sh).

`oino`/`unipi-host` release profiles: keep the root workspace release profile (`strip`,
`lto`) for the CLI; the `apps/unipi` workspace keeps its own (`opt-level 3`, `panic=abort`).

## 8. Updater channel

Keep the app updater (`tauri-plugin-updater`, already wired in the imported app:
`app_update.rs`, endpoints + minisign pubkey in `tauri.conf.json`), pointed at **this
project's GitHub Releases**:

- **New minisign keypair** for the UniPi-on-oino channel (`~/Projects/Neuron/credentials/
  oino-updater/`, private key + password file, never committed). Old `unipi-updater` key keeps
  signing only the frozen old channel. *(Open Q3: owner may prefer reusing the old key.)*
- Endpoint: `https://github.com/Neuron-Mr-White/Oino/releases/latest/download/latest.json`
  (stable permalink for release assets), with the per-platform URLs in it pointing at the same
  Release's assets. Requires the repo (or at least its releases) to be **public** — *(Open Q1)*.
  Fallback if it must stay private: publish `latest.json` + artifacts to a public static host
  (the existing `unipi.nrn.one` server pattern); only the `endpoints` URL changes.
- Signed artifacts: `UniPi_{v}_x64-setup.exe` (+`.sig`), `UniPi_{v}_universal.app.tar.gz`
  (+`.sig`) — `cargo tauri signer sign` during `dist publish`, exactly as release.sh did.
- Platform support matrix (tauri-plugin-updater): Windows ✅, macOS ✅, **Linux deb ❌**
  (updater supports AppImage only). On Linux the app's update check becomes "new version
  available → open the release download page". No AppImage in v2 (DIRECTION: one installer
  per platform; revisit if Linux self-update is missed).
- The **old unipi app must never see the new channel**: different identifier
  (`dev.unipi.oino`), different pubkey, different endpoint. Its `unipi.nrn.one` feed is left
  exactly as is.

## 9. CI matrix (`.github/workflows/ci.yml`)

- Add **`v2`** to `on.push.branches` (today `[main, v1-alpha]` — nothing builds on v2 pushes;
  flagged by the checkpoint handoff). Keep `[main, v1-alpha]`.
- Existing jobs unchanged: `fmt`, `wasm` (+ committed checksums), `root`, `docs-and-installers`,
  `windows-installer` (ps1 dry-run), tag job `release-binaries`/`release-assets` (CLI channel).
- New job **`unipi-app`** (ubuntu): `npm ci` in `apps/unipi`, `npm run typecheck`, `npm test`
  (vitest), `cargo check` for the `apps/unipi` workspace (with the webkitgtk dev deps
  preinstalled). Fast fail before dist work.
- New tag job **`dist`** (`on.tags v*`, `needs: [wasm, root, unipi-app]`, matrix):
  `ubuntu-22.04` → deb + apk (unsigned/CI-keystore), `windows-latest` → nsis,
  `macos-14` → universal dmg. **CI never publishes** (no owner secrets on runners beyond
  nothing); artifacts are attached as workflow artifacts as a buildability gate. Canonical
  releases come from §7. *(If Q1 lands on GitHub Releases with a public repo, CI may later
  take over publishing with the updater key as a secret — noted, not spec'd.)*
- `cargo xtask dist` steps used by CI are the same scripts as §7 (they take
  `--stage-only` to stop before smoke/publish).

## 10. Smoke tests (per installer, all must pass before a release)

Common checks first (from the existing `build-host.sh` smoke): `oino --version` reports the
core version; the faux print run (`faux_run`: `oino -e <faux> --model faux/faux-1 -p hi` →
"Hello from faux.") passes with the share dir unset; `unipi-host --version` reports the
distribution version; `oino list` shows the distribution packages (and `OINO_DIST_EXTENSIONS=`
empties it). Then per installer:

| Installer | Steps (script) | Pass |
|---|---|---|
| Windows (`scripts/dist/smoke-windows.ps1`, run on coffee) | silent install `UniPi_…_x64-setup.exe /S`; **new** shell: `oino --version` resolves on PATH; `UniPi.exe` launches; `unipi-host.exe` sidecar process appears (spawned child); `oino list` sees distribution packages; `unipi-kanboard --version` via the package bin dir; uninstall silently; install dir + PATH entry gone | all green, `~/.oino` user data untouched by uninstall |
| macOS (`scripts/dist/smoke-macos.sh`, on desmondgiam) | mount dmg, `cp -R UniPi.app /Applications` (test: `~/Applications`), `codesign -v` (ad-hoc ok), `open` the app, `pgrep unipi-host`, Resources/bin/`oino --version`, CLI symlink action → `oino --version` from `~/.local/bin`, delete app + symlinks | all green; old `/Applications/UniPi.app` (if the old app is installed) untouched — new bundle id differs |
| Linux (`scripts/dist/smoke-linux.sh`, local **podman** ubuntu container — never the host: it carries the owner's live old unipi) | `dpkg -i unipi-oino_…_amd64.deb`; `/usr/bin/oino --version`; `dpkg -L unipi-oino` lists `/usr/share/oino/extensions`; launch `unipi-mobile` under `xvfb-run`, host sidecar appears; faux print run; `dpkg -r unipi-oino` → `/usr/bin/oino` and share dir gone | all green inside the container |
| Android (`scripts/dist/smoke-android.sh`, phone plugged in) | `adb install -r unipi-oino-{v}.apk`; launch main activity; app reaches the host list and pairs with the local host over LAN (QR/link); screenshot stored under `releases/{v}/qa/`; **old unipi app still installed and opens** (coexistence); `adb uninstall dev.unipi.oino` | all green; apksigner cert pinned (build-time check) |

Publish gate (`dist publish`): every smoke log in `releases/{v}/qa/` ends PASS, and the
release folder contains all §2 artifacts + `.sig` files + `latest.json` + `release-manifest.json`.

## 11. Coexistence with the owner's old unipi (hard requirements)

| | Old unipi (live) | UniPi on oino |
|---|---|---|
| Tauri identifier / APK id | `dev.unipi.app` | `dev.unipi.oino` |
| Data | `~/.unipi` | `~/.oino` (host: `~/.oino/host`) |
| Host binary | `~/.local/bin/unipi-host` (systemd/LaunchAgent) — **never touched** | sidecar inside the app bundle only, no PATH, no service |
| iroh key/socket/tray | own | own (separate key, socket, tray name) |
| Updater feed | `unipi.nrn.one` (frozen) | this repo's Releases |
| deb/AppImage on PCs | old `UniPi` packages stay | `unipi-oino` package name, separate files |
| Phone | old app stays installed | new app installs beside it |

Migration from `~/.unipi` stays a user-run skill (DIRECTION §4.1); installers do no import.
Never run smoke tests against `~/.unipi`, `~/.pi`, or the running old host.

## 12. Non-goals

iOS shipping (build-compatible only); rpm/AppImage/Flatpak; Linux arm64 installers
(share-dir layout is arch-clean, add later); headless `unipi-host` installers (the
sidecar is the host; the old product covers that need); third-party extension marketplace.

## 13. Work packages

Dependency notes: WP-D1 blocks everything; D3–D6 are parallel after D1+D2; D7 after D3/D4;
D8/D9 assemble. §3.4 (kanboard CLI rename) and the §1 bundle move are **inputs** to D1.

| WP | Title | Files (new/changed) | Deps | Acceptance |
|---|---|---|---|---|
| **WP-D1** | Extension distribution packages + share-dir discovery | `crates/oino-core/src/…` (discovery moves in from `crates/oino/src/loader.rs`), `crates/oino/src/loader.rs`, `crates/oino-wasm-host/src/packages.rs` (bin field), `xtask/src/dist.rs` (stage), `extensions/unipi/kanboard/extension.json` | §1 bundle move, §3.4 | `oino` with no bundle sees unipi packages from a share dir; user dir shadows by id; `OINO_DIST_EXTENSIONS` override works; `unipi-kanboard` staged into `bin/`, checksummed; `oino list` reflects it |
| **WP-D2** | Versioning + release manifest plumbing | `xtask/src/dist.rs` (version cmd), `.github/workflows/ci.yml` (v2 branch, `unipi-app` job) | — | `dist version 2.0.0` bumps the four files + commits; CI green on v2 pushes incl. the app job |
| **WP-D3** | Windows NSIS installer (coffee) | `apps/unipi/apps/mobile/src-tauri/tauri.conf.json` (identifier, nsis config, resources), `nsis/installer.nsh` (PATH hook), `scripts/dist/windows-build.sh`, `scripts/dist/smoke-windows.ps1` | D1, D2 | §10 Windows column green on coffee |
| **WP-D4** | macOS dmg (desmondgiam) | `scripts/dist/macos-build.sh` (port of mac-build.sh), `scripts/dist/smoke-macos.sh`, tauri.conf (identifier) | D1, D2 | §10 macOS column green; universal lipo verified; ad-hoc codesign `--verify` ok; iOS check compiles |
| **WP-D5** | Linux deb | `scripts/dist/linux-build.sh`, `scripts/dist/deb-postprocess.sh`, `scripts/dist/smoke-linux.sh` (podman) | D1, D2 | §10 Linux column green in container |
| **WP-D6** | Android apk | `tauri.android.conf.json` (identifier), `gen/android/app/build.gradle.kts` (env signing stays), `scripts/dist/android-build.sh`, `scripts/dist/smoke-android.sh` | D1, D2 | §10 Android column green on the plugged-in phone; old app coexists |
| **WP-D7** | Updater channel | `tauri.conf.json` (pubkey + endpoint), `scripts/dist/publish.sh` (latest.json + minisign sigs), new keypair in credentials dir | D3, D4, Q1/Q3 answered | signed `latest.json` validates against a dry-run `tauri updater` check; old channel untouched |
| **WP-D8** | `xtask dist` orchestrator + mise + host docs | `xtask/src/dist.rs`, `xtask/src/main.rs`, `.mise.toml` (`dist` task, `raw`), `scripts/dist/README.md` | D3–D6 | `mise run dist -- all` produces the full `releases/{v}/` tree from this PC driving all hosts; sync excludes node_modules/target |
| **WP-D9** | CI dist matrix + release assembly | `.github/workflows/ci.yml` | D2, D3–D6 scripts take `--stage-only` | tag `v2.0.0-rc*` run builds deb/nsis/dmg/apk on runners, artifacts attached, nothing published |

Gates for every WP: the repo's standard gates (fmt, clippy `-D warnings`, tests, docs-check)
plus the WP's acceptance row.

## 14. Risks

1. **Private repo vs public release assets** — the updater and one-line installs need
   anonymous downloads. *(Open Q1; fallback: static host, §8.)*
2. **Deb post-processing** depends on Tauri's internal deb layout; pin the tauri CLI version
   (the app already pins `=2.x` crates) and add a layout assertion to the postprocess script.
3. **Sidecar size**: unipi-host now links oino-core → installers grow (expect the macOS
   universal app ~2× the 45 MB of unipi-app 1.0.0). Measure in D3–D6; `strip` + `lto` already on.
4. **NSIS PATH hook edge cases** (PATH > 1024 chars, duplicate installs): write-then-read-back
   verification in smoke; per-user only.
5. **macOS ad-hoc only** — right-click-first-run friction; no notarization until an account
   exists (§2.1). Updater unaffected.
6. **Host build environment drift** (node/rust on coffee/desmondgiam): `scripts/dist/README.md`
   pins versions; `dist` checks them before building (fail with the exact install command).
7. **Android signing** env lives only on the owner's machine — CI builds unsigned apk
   (debug-key) and only the local `dist android` produces the shippable one.
8. **Share-dir heuristics** (exe-relative resolution) must match the three install layouts
   exactly; covered by per-installer smoke `oino list` checks and a unit test per OS layout.
9. `unipi-kanboard` staging adds native artifacts next to wasm packages — keep the committed
   `extensions/dist` wasm-only (staging copy only) or CI's committed-checksums check breaks.

## 15. Owner questions (defaults chosen so work can start)

1. **Repo visibility**: make `Neuron-Mr-White/Oino` (or just its releases) public for the
   updater/installs? Default: yes, public repo + GitHub Releases.
2. **First distribution version**: `2.0.0` (clean break, matches "v2")? Default: 2.0.0.
3. **Updater key**: new `oino-updater` minisign keypair (default) or reuse `unipi-updater`?
