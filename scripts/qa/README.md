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
