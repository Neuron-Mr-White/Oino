# Real-app QA rig

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

Parallel rigs: give each its own `QA_HOME`, `QA_DISPLAY` (e.g. `:151`) and `QA_PORT`
(e.g. `4451`). The rig copies `~/.oino/auth.json` and `settings.json` into its own
agent dir; it never touches `~/.oino`, `~/.unipi` or the apps the owner is running.
Needs `~/.local/bin/WebKitWebDriver` (from the `webkitgtk-webdriver` deb, extracted)
and `cargo install tauri-driver`.
