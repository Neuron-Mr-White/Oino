# WP-H9 — updater + app lifetime: QA

Spec: `docs/dev/v2/specs/host.md` §5 (host lifetime), §9.7 (updater, de-service),
§11.2 (keep/change/delete). Branch `v2`. Owner decisions: RULINGS (tray label
**UniPi (oino)**, host §14 Q1), DIRECTION §2 (no service install, owner Q4).

## Automated evidence

| Gate | Command | Result |
|---|---|---|
| Host updater (faux feed, restart-exit, notices) | `cargo test -p unipi-host update::` | 8/8 (`decide_picks_the_host_artifact_of_this_target`, `manifest_parses_and_checks_origin`, `faux_feed_installs_a_newer_host_and_says_how_to_restart`, `prompt_mode_queues_one_update_notice`, `install_refuses_wrong_version_and_keeps_old`, `status_reports_available_from_state`, `config_round_trip_keeps_other_keys`, `versions_compare_numerically`) |
| Host crate | `cargo test -p unipi-host` | green (293 unit + integration tests) |
| Host lints | `cargo clippy -p unipi-host --all-targets -- -D warnings` | green |
| App lifetime units | `cargo test --manifest-path apps/unipi/src-tauri/Cargo.toml` | see below — the app crate's test target is blocked by other wave-2 WPs (`chat.rs` H4, `term.rs` H6, `board.rs` A21, `cron.rs`/`hosts_commands.rs`), not by H9 |

The app crate's H9 modules (`host_supervisor`, `lifecycle`, `desktop`) were
compiled and tested against the same sources in a scratch copy with the
still-broken modules unregistered: **131 passed / 0 failed**, including
`host_supervisor::{the_decision_is_adopt_or_spawn_only, spawn_runs_the_host_as_an_app_child_and_logs_it,
crashes_restart_with_backoff_and_banner_after_three, a_host_that_exits_75_means_someone_else_owns_the_dir}`,
`lifecycle::{closing_the_window_follows_the_remembered_answer, start_at_login_starts_hidden_in_the_tray,
quit_asks_only_for_an_app_managed_host_with_phones, app_settings_round_trip_in_the_v2_config_dir}` and
`desktop::{the_tray_line_is_phones_sessions_and_needs_you, status_lines, ...}`.
Re-run the real command once H4/H6/A21 land.

## Manual tray / lifetime checklist (Linux + Windows)

Not yet executed: the Tauri crate does not compile in this tree until the wave-2
app/host WPs land (`cargo check --manifest-path apps/unipi/src-tauri/Cargo.toml`).
Run each row on **both** Linux (this PC, GTK tray) and Windows (coffee) and tick
the expected result.

Setup for the host rows: use a throwaway host dir (`OINO_HOST_DIR=/tmp/qa-h9/host`)
so the owner's real `~/.oino/host` and the live old unipi are never touched.

| # | Step | Expected |
|---|---|---|
| 1 | Start the app with no host running | It spawns `unipi-host serve --mode app --exit-with-parent <app pid>`; Settings → This computer says `spawn`; `<dir>/host.log` gets lines; `unipi-host status` → `running — mode app`; one `unipi-host` process only |
| 2 | With that host running, start a second app instance / run `unipi-host serve` by hand | `adopt` (or the manual one wins, `Held` → adopt); no second host process; the tray/`host_runtime` still answer |
| 3 | Tray label, menu, tooltip | Icon title/tooltip **UniPi (oino)**; tooltip `UniPi (oino) — N phones · M sessions running · K needs you`; menu has the status row, Show/Hide, Mascot, Quit; a left click toggles the window on Windows/macOS (Linux opens the menu) |
| 4 | Close the window with no `closeAction` stored (default `ask`) | One dialog: "Close UniPi, or keep it running in the tray so phones and cron keep working?" with **Minimize to tray** (default) and **Quit**, plus "Remember my choice"; the app keeps running until answered |
| 5 | Choose "Minimize to tray" + remember; close twice more | First close hides the window; the next closes hide silently (no dialog); Settings → General shows `tray`; the host keeps running |
| 6 | Choose "Quit" + remember; close the window | The app quits (with the phone warning if one is paired, see 7); the next launch-close also quits |
| 7 | Tray → Quit with a paired phone and an app-managed host | Confirm sheet names the phone count and says phones can't reach this PC until UniPi is opened again (main window is brought up first); Quit exits the app **and** the host (no `unipi-host` left, `host.lock` released); Keep in tray leaves both running |
| 8 | Quit with an adopted (manual) host | No warning (the manual host keeps running), the app just exits |
| 9 | Settings → General → Start at login ON | Linux: `~/.config/autostart/dev.unipi.oino.desktop` with `--hidden`; Windows: `HKCU\…\Run` value for UniPi with `--hidden`; no systemd unit, LaunchAgent, logon task or admin prompt anywhere. Toggle OFF removes it |
| 10 | Log out / in (login item active) | No window flash: the app starts in the tray (`--hidden`), the tray is present, the host is adopted/spawned and phones can connect |
| 11 | Old unipi still installed and running | Two tray icons with distinct labels ("UniPi" vs "UniPi (oino)"); the old host's data dir `~/.unipi/app-host` is untouched; the new app's host lives in `~/.oino/host` |
| 12 | Host update in `prompt` mode (`unipi-host update --mode prompt`), then point `UNIPI_UPDATE_FEED` at a local manifest with a newer version and restart `serve` | Within a minute `<host dir>/notices.json` holds one `{"kind":"update-available", …}`; nothing is installed; `unipi-host update --check` says it is available |
| 13 | Host update in `auto` mode with the same faux feed | `serve` replaces the binary, logs `binary replaced; exiting for a restart`; the app supervisor respawns it within its backoff and the new version reports `--version`; a **manual** `serve` prints `restarted binary; rerun unipi-host serve` and exits |
| 14 | Crash banner | Kill the app-managed host 3× in a row: the UI banner appears after the third restart (`Failing { failures: 3 }`); restarting the app adopts/spawns again |
