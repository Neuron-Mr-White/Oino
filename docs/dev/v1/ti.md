# `ti`: the GUI test host for Oino Desktop

Status: working (2026-10-28). Owner rules: [CHARTER.md](CHARTER.md) "Test hosts".

`ti` is the preferred headless GUI host for Oino Desktop: 14 threads, 30 GB RAM, ~229 GB free,
Ubuntu 26.04. It runs other services and has a logged-in desktop on `:0`/`:1`, so everything
here stays inside `~/oino-test`, renders on a **private `Xvfb :93` with its own dbus session**,
runs at `nice -n 10` with `cargo -j 6`, and only ever signals the PIDs it recorded itself.
Never use `:0`/`:1`, never touch the services, never `pkill` by pattern.

`cargo xtask ti …` (this doc + [`xtask/src/ti.rs`](../../../xtask/src/ti.rs)) automates the
whole loop; the manual commands below are what it runs, for when something needs a hand.

## 1. One-time host setup

System packages for GPUI on Linux (`gpui`, `gpui-webview`, `lavapipe`, Xvfb tooling):

```bash
ssh ti@ti 'sudo -S -p "" env DEBIAN_FRONTEND=noninteractive apt-get install -y \
  pkg-config cmake clang \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxdo-dev \
  libvulkan1 mesa-vulkan-drivers libgbm-dev libegl1 \
  libfontconfig-dev libfreetype-dev libssl-dev libasound2-dev libzstd-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev \
  xvfb xdotool imagemagick dbus-x11 python3-pil x11-utils' \
  < /home/oi/Projects/Neuron/credentials/ti/sudo.pass
```

The password file is piped in at use time and never printed, copied or committed.

Rust toolchain (matches `rust-toolchain.toml`; `~/.cargo/bin` is **not** on the login `PATH`,
every `ti` script exports it):

```bash
ssh ti@ti 'export PATH=$HOME/.cargo/bin:$PATH
  rustup toolchain install 1.99.0 --profile minimal
  rustup component add --toolchain 1.99.0 rustfmt clippy
  rustup target add --toolchain 1.99.0 wasm32-wasip2'   # target only for `cargo xtask wasm`
```

`wasm32-wasip2` is not needed for the desktop loop: `remote-build`/`ti sync` ship the local
`extensions/dist`, and `oino-bundle`'s `build.rs` embeds it at compile time. It is installed
so `cargo xtask wasm` also works on the host.

## 2. `cargo xtask ti <cmd>`

| Command | Does |
|---|---|
| `ti sync` | `HEAD` as a `git bundle` (incremental when the host has an ancestor) + the local `extensions/dist` tarball → `ti:~/oino-test/oino` (detached checkout). Uncommitted changes are **not** synced. |
| `ti build [--jobs 6] [--timeout-mins 180]` | `cargo build --release -p oino -p oino-kanboard` then `-p oino-desktop` (in `apps/`), then a symlink of `oino-kanboard` next to the desktop binary (the host looks for helper binaries beside the executable, as packaging ships them — without it the Board view sits on "Starting the kanboard daemon") at `nice -n 10`, log `~/oino-test/run/build.log`, streamed line by line. |
| `ti start [--demo] [--faux] [--cwd <dir>] [--env K=V]` | Scratch agent dir + `Xvfb :93 -screen 0 1600x1000x24 -nolisten tcp` + private `dbus-launch` session + the app at `OINO_DESKTOP_SIZE=1600x1000`, `OINO_REDUCE_MOTION=1`, lavapipe (`VK_ICD_FILENAMES=…/lvp_icd.json`). |
| `ti shot [--name <n>] [--out <dir>]` | PNG of the app window (`xdotool` → `import`, full display when the window is not found), fetched to `docs/dev/v1/design/shots/ti/<n>.png` by default. |
| `ti status` | PIDs and whether they are alive, `:93` free/busy, last build exit code, desktop binary. |
| `ti soak [--minutes 10] [--backend demo\|faux]` | Runs `apps/oino-desktop/scripts/soak.sh` there (its own `Xvfb :12x`, cleans up after itself) and fetches `summary.txt` / `samples.tsv` / `app.out` to `target/ti-soak/`. |
| `ti stop` | `SIGTERM` (then `SIGKILL` after 5 s) to the recorded app / dbus / Xvfb PIDs only, after checking `/proc/<pid>/cmdline` still belongs to `~/oino-test` — anything else is left alone. |

Typical loop:

```bash
export TMPDIR=$PWD/target/tmp
cargo xtask ti sync
cargo xtask ti build                 # ~9 min for the CLI, ~5 min for the desktop (cached)
cargo xtask ti start --demo
sleep 12
cargo xtask ti shot --name desktop
cargo xtask ti stop
```

### Layout on `ti`

```
~/oino-test/oino                 detached checkout of HEAD (+ extensions/dist)
~/oino-test/oino/apps/target/release/oino-desktop
~/oino-test/agent                scratch OINO_AGENT_DIR (auth.json mode 600, logs/, sessions/)
~/oino-test/project              scratch --cwd
~/oino-test/run                 xvfb.pid dbus.pid dbus.address app.pid app.log / build.* / soak.*
~/oino-test/soak                last soak.sh output
```

### Credentials and real sessions

`ti start` copies this PC's `~/.oino/auth.json` to `~/oino-test/agent/auth.json` (mode 600) for
real and `--faux` runs (`--demo` uploads nothing) so the app has provider keys; the file is read and re-uploaded in memory, never printed,
logged or copied into the repository. `OINO_AUTH=/path/to/auth.json` overrides the source.

* `--demo` — the scripted demo scene, no providers, no network. `--env
  OINO_DESKTOP_DEMO_SCENE=<welcome|run|approval|question|plan|board|info|stream>` picks the
  scene (see [gui-driver.md](gui-driver.md) §4).
* `--env KEY=VALUE` (repeatable) adds an environment variable to the app process — how the
  automation driver enables its socket (`OINO_DESKTOP_AUTOMATION=…`).
* `--faux` — a **real** session (oino-core + bundled WASM extensions + tools, no network bill):
  the provider-faux fixture from `extensions/dist` is copied into the scratch agent dir and
  `settings.json` selects `faux/faux-1`; a `faux` key is added to the uploaded `auth.json`
  because oino-core requires one.
* no flag — real providers from the copied `auth.json`; a fresh agent dir opens onboarding, so
  set a default provider/model in `~/oino-test/agent/settings.json` first. The core keys are
  pi's `defaultProvider`/`defaultModel` pair (`{"defaultProvider":"oino","defaultModel":
  "deepseek/deepseek-flash"}`), and the router provider needs its credential under its own id:
  `upload_auth` adds `auth.json`'s legacy `router` entry as `oino` (the component's env fallback
  is `OINO_API_KEY`). A dynamic provider also needs its cached model list in
  `<agent>/cache/models/<provider>.json`; `scripts/gui-drive.py` copies this PC's
  `~/.oino/cache/models/oino.json` for `backend: real` scenarios.

Driving the window (no window manager on `:93`): `xdotool search --onlyvisible --class
dev.oino.desktop | head -1`, then `xdotool windowfocus <id>`, `xdotool type --window <id> "hi"`,
`xdotool key --window <id> Return` with `DISPLAY=:93` exported.

## 3. Manual equivalent

```bash
# sync: HEAD over a bundle, extensions/dist as a tarball
git bundle create /tmp/oino.bundle HEAD
scp /tmp/oino.bundle ti@ti:oino-release.bundle
(cd extensions && tar -czf /tmp/dist.tgz dist) && scp /tmp/dist.tgz ti@ti:oino-release-dist.tgz
ssh ti@ti 'set -e; mkdir -p ~/oino-test/oino; cd ~/oino-test/oino
  [ -d .git ] || git init -q .; git fetch -q ~/oino-release.bundle HEAD
  git checkout -q -f --detach FETCH_HEAD
  rm -rf extensions/dist; mkdir -p extensions; tar -xzf ~/oino-release-dist.tgz -C extensions'

# build (one build at a time; nice + -j 6)
ssh ti@ti 'export PATH=$HOME/.cargo/bin:$PATH; cd ~/oino-test/oino
  nice -n 10 cargo build --release -j 6 -p oino
  cd apps && nice -n 10 cargo build --release -j 6 -p oino-desktop'

# private display + dbus session + app
ssh ti@ti 'run=~/oino-test/run; mkdir -p $run
  Xvfb :93 -screen 0 1600x1000x24 -nolisten tcp >$run/xvfb.log 2>&1 & echo $! >$run/xvfb.pid
  sleep 1; eval "$(dbus-launch --sh-syntax)"; echo $DBUS_SESSION_BUS_PID >$run/dbus.pid
  DISPLAY=:93 DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS \
    OINO_AGENT_DIR=~/oino-test/agent VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
    nohup ~/oino-test/oino/apps/target/release/oino-desktop --demo >$run/app.log 2>&1 &
  echo $! >$run/app.pid'

# screenshot back (window id from xdotool; root works too)
ssh ti@ti 'DISPLAY=:93 import -window root ~/oino-test/run/shot.png'
scp ti@ti:oino-test/run/shot.png docs/dev/v1/design/shots/ti/desktop.png

# stop only what we started
ssh ti@ti 'for f in app dbus xvfb; do p=~/oino-test/run/$f.pid; [ -f $p ] && kill $(cat $p); rm -f $p; done'
```

## 4. Release artifacts on `ti`

`cargo xtask remote-build ti@ti --dir oino-test/oino --jobs 6` still works and is the path for
packaged artifacts (`scripts/release/build-host.sh` → AppImage/tar.gz + smoke tests, with the
desktop screenshot taken on the Xvfb it starts itself). `cargo xtask ti` is the faster
dev loop; `remote-build` additionally builds `oino-kanboard` and the installers.

## 5. Known wrinkles

* `libEGL warning: DRI3 error` on `:93` is expected (no GPU/DRI3 on Xvfb); lavapipe renders.
* `:93` busy → something else on the host holds it; `ti status` shows it, do not kill it.
* Desktop frames on Xvfb are software-rendered: soak thresholds judge memory/threads/fds, and
  `SOAK_MAX_FRAME_MS` stays 0 (report-only) as `soak.sh` documents.
* `ti sync` only sends committed `HEAD`; commit before syncing or the host builds the previous
  commit.

## 6. Verified runs

| Date | What | Result |
|---|---|---|
| 2026-10-28 | release build of `oino` + `oino-desktop` (HEAD `f7dc777`) | OK, `build exit 0`, desktop 140 MiB, ~14 min cold |
| 2026-10-28 | `ti start --demo` + screenshot | [design/shots/ti/demo.png](design/shots/ti/demo.png), 1598×1000 |
| 2026-10-28 | `ti start --faux` (real session, prompt `hi`) | `Hello from faux.` in 240 ms, [design/shots/ti/real-faux.png](design/shots/ti/real-faux.png) |
| 2026-10-28 | `ti soak --minutes 10 --backend demo` | **PASS** — see §6.1 |

### 6.1 Soak numbers (10 min, demo backend, Xvfb :120, lavapipe)

```
backend=demo minutes=10 samples=120 steps=848
rss_mb start=11 end=295 max=296 slope_2nd_half_mb_per_min=0.14
threads first_quarter_max=94 end=93
fds first_quarter_max=69 end=63
frame_ms run_max=41.29 run_p99=14.10
crash_logs=0 stall_logs=0 exit=0
RESULT: PASS
```

Thresholds from `soak.sh` (slope ≤ 8 MB/min, +16 threads, +64 fds, no crash/stall) were all met
with wide margin; frame times are software-rendered and report-only (`SOAK_MAX_FRAME_MS=0`).
Raw files: `target/ti-soak/{summary.txt,samples.tsv,app.out}` (fetched by `ti soak`).

After a run, `ti stop` and the soak/screenshot helpers leave nothing running on the host
(`pgrep -af oino-test` empty, no `Xvfb` beyond the owner's `:0`/`:1`). The checkout and its
`target/` (~6 GB) stay under `~/oino-test` for the next run; `rm -rf ~/oino-test/oino/target`
frees them when the host needs the space.
