# Real-app QA rig

> **The owner uses UniPi Oino daily on this PC (since 2026-11-02).** Never touch the live
> app or its state: no requests to `~/.oino/host/app.sock`, no reads/writes of `~/.oino`
> sessions, no `pkill` of `/usr/bin/unipi-oino` or its host. Every test, probe and rig
> runs with its own `QA_HOME` (isolated `HOME`/XDG dirs, `OINO_AGENT_DIR`,
> `OINO_HOST_DIR`). Heavy builds go to the Agent37 sandbox, not this PC.

UI work is verified on the **real installed app** (the `.deb` file tree, the real
`unipi-oino-host`, the real bundled extensions, real providers), never on the mock
capture harness. The app runs in a private Xvfb display, driven over WebDriver.

```sh
scripts/qa/rig.sh up                 # Xvfb + tauri-driver + the app (isolated data in ~/.cache/oino-qa)
node scripts/qa/rig.mjs shot name    # screenshot of the real window -> prints the PNG path
node scripts/qa/rig.mjs click 'css'  # click / type / key / hover / eval / text / wait / size / logs
scripts/qa/rig-rebuild.sh [--host]   # rebuild from the working tree and restart the app
scripts/qa/rig.sh down
```

Resource policy: at most 2 rigs at once (QA_MAX_RIGS), only with >= 6 GB RAM free; rig-rebuild takes a global build lock and builds with 4 jobs at nice 10. Always `rig.sh down` when done.

Parallel rigs: give each its own `QA_HOME`, `QA_DISPLAY` (e.g. `:151`) and `QA_PORT`
(e.g. `4451`). The rig copies `~/.oino/auth.json` and `settings.json` into its own
agent dir; it never touches `~/.oino`, `~/.unipi` or the apps the owner is running.
Needs `~/.local/bin/WebKitWebDriver` (from the `webkitgtk-webdriver` deb, extracted)
and `cargo install tauri-driver`.

## Phone sandbox (Android, off this PC)

`scripts/qa/phone-sandbox.sh` runs the Unipi Android app against a throwaway PC host on **one Agent37
`agent37-android` VM** (Android 14 x86_64 with ARM translation, so the arm64 release APK runs). Nothing runs
on this PC except ssh/adb-over-ssh and a CDP client. The VM side uses isolated `/root/qa/{home,agent,hostdir}`,
the static `unipi-oino-host` and bundled extensions from a `.deb`, a fake OpenAI-compatible model
(`phone-fakellm.py`, keywords: `tools`, `edit`, `image`, `ask`, `error`, `long`) and no owner secrets.

```sh
S=scripts/qa/phone-sandbox.sh
$S up                      # VM: 4 vCPU/16 GB, auto-sleep OFF, emulated cameras; waits for Android (~1 min)
$S install [APK] [DEB]     # default: ~/oino-qa-release/v9/*; also installs a current WebView (the image has 113)
$S cdp                     # restart the app, forward its WebView devtools to localhost:9333
$S pair                    # host `pair --json` → cold-start the app on the unipi://pair deep link → tap Pair
$S shot 10-cron            # PNG into docs/dev/v2/design/shots/phone-audit (PHONE_SHOTS=dir to change)
$S tap 'New chat'          # real touch on an element found by text, aria-label or CSS
$S measure                 # targets < 44 px, font-size histogram, tiny text, horizontal overflow
$S crash                   # Add a PC → Pair with QR → prints `logcat -b crash` (the R8/ML Kit crash)
$S adb shell …  |  $S ssh …  |  $S url (noVNC live screen, 1 h)  |  $S status
$S down                    # ALWAYS: deletes the VM (≈ $0.10/h while it exists)
```

Notes: the deep link needs a current WebView (`install` provides one; on the image's WebView 113 it is ignored).
By hand: Add a PC → Paste pairing link → type `<ticket> <code>` → Enter → Pair. The QR scan crashes release builds. `adb input text` needs `%s` for spaces. The emulator sometimes presents black frames
while overlays are open; `shot` then falls back to a CDP capture without the status bar. Retrace release stack traces
with `cmdline-tools/latest/bin/retrace` and `gen/android/app/build/outputs/mapping/universalRelease/mapping.txt`
of the same build. Findings from the first run: `oino-internal/docs/dev/v2/qa/phone-audit.md`.
