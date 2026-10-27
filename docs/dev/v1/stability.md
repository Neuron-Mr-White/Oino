# Oino Desktop stability (freezes, crashes, leaks)

Work id `[stability]` on `v1-alpha`. This covers `apps/oino-ui` and `apps/oino-desktop`, plus
the core paths they call.

## Root causes and fixes

| # | Symptom | Root cause | Fix |
|---|---|---|---|
| 1 | **Crash** a few seconds after typing in the composer (real sessions) | `SessionPane::on_text_changed` ran `backend.editor_event()` on a GPUI background executor. `oino-core` `AgentSession::editor_event` calls `tokio::spawn` for non-consumable intents, which panics off-runtime ("there is no reactor running"). The panic killed GPUI worker `Worker-N`, and the main thread then panicked polling the dead task ("Task polled after completion"). Reproduced on the first baseline run. | The debounced editor event now runs through `rt::Tokio::spawn`. The stored task aborts the previous debounce when the next keystroke replaces it. (`views/session_pane.rs`, `on_text_changed`) |
| 2 | Crash when using **Extensions → Install from file** | `EmbeddedHost::install_extension` uses `spawn_blocking`, but its future was awaited on the GPUI foreground executor. | Awaited through `Tokio::spawn`. (`views/extensions_page.rs`) |
| 3 | **Leak**: RSS, fds and threads grew with every session opened | Panes were never dropped: `close_pane` only removed a pane from the layout. Nothing called `SessionBackend::shutdown`. `EmbeddedHost` also kept a strong `Arc<AgentSessionRuntime>` for every session ever opened, so each session's extension state, event hub and tool processes stayed alive. | Hidden idle panes beyond `MAX_LIVE_PANES` (8) are released, least recently used first (`Workspace::release_idle_panes` / `drop_pane`, pure policy `panes_to_release` with a unit test). Visible, running and waiting panes are never released. `drop_pane` calls `backend.shutdown()` on tokio, and the host now keeps `Weak` runtimes. Deleting a session also stops its pane. |
| 4 | Lingering process / unreaped children at quit | No session shutdown at quit. `drop(runtime)` could hang on a blocking task. The process stayed alive while the opt-in mascot window was open. | `SessionHost::shutdown` (default no-op; the embedded host disposes every live runtime) starts in `on_app_quit` and is awaited after the event loop with a 3 s grace. Then `runtime.shutdown_timeout(3 s)` runs. Closing the main window quits the app. (`oino-desktop/src/main.rs`) |
| 5 | **Jank / freeze** with the Files side tab open on big trees (network or slow fs) | `FileTree::rows` walked the directory tree (up to 2000 `read_dir` entries) on every render of the side pane. Opening a file read it whole with `read_to_string` on the UI thread. | The walk is cached for 2 s and invalidated on toggle. File views read at most 512 KiB (`side_pane::read_head`). |
| 6 | Unbounded global | `ous::Drains` (countdown meters) kept one entry per meter ever shown. | Pruned to entries younger than 10 min once it reaches 128 entries. |
| 7 | Soak driver crash (found by the soak, driver-only) | Keystrokes were dispatched inside `Workspace::update_in`. Drawing re-entered the workspace (GPUI double lease). | Keys are dispatched outside the update. |

Reviewed and judged OK (no change): `rt::Tokio` (aborts on drop), the event pump (it exits when
the pane is dropped, and core `EventHub` is bounded with lag-disconnect), `Ticker` (stops when
idle, slot table capped at 64), toast timer (stops when no timed toasts remain), `TerminalView`
(`Msg::Shutdown` on drop, `drain_on_exit`), browser load channel (task stored in the view),
the mascot `request_animation_frame` (only while a bubble is up), and the `cx.notify` paths in
render (none unconditional).

## Diagnostics added

- `oino_ui::stability::install_panic_hook`: every panic writes
  `~/.oino/logs/desktop-crash-<UTC>-<pid>.log` with the thread, message, location, the UI
  activity breadcrumb and a full backtrace. A panic on `main` or on a GPUI `Worker-*` thread
  **aborts after logging**, because unwinding there leaves the app frozen or crashing later in
  a confusing place. Panics on tokio threads unwind as before (tokio contains them).
- `oino_ui::stability::Watchdog`: the UI thread beats every 250 ms through a GPUI timer. A
  watchdog thread writes `desktop-stall-<UTC>-<pid>.log` when the UI thread is silent for
  ≥ 2 s. The log holds the breadcrumb and the state, wait channel and syscall of every thread.
  With `OINO_WATCHDOG_GDB=1` it also holds a gdb main-thread backtrace, which needs ptrace;
  `ptrace_scope` is 1 on the dev box. A recovery line is logged when the UI thread beats again.
- Breadcrumbs: `stability::activity("…")` guards around the workspace render, pane batch apply
  and history load.
- `FrameStats::run_max`: the slowest frame since start.

## Soak

`apps/oino-desktop/scripts/soak.sh --minutes 30 --backend demo|faux [--bin …] [--max-panes N]`
runs one instance with `--soak <s>`; the in-app driver is `oino_ui::views::soak`. By default it
runs on its own Xvfb with lavapipe (software Vulkan, which keeps the GPU out of unattended runs).
The cycle is: new session, long streamed prompt, typing, Files, Tree, theme toggle, palette
open/close, terminal open, settings and back, Review (or Browser with `OINO_SOAK_BROWSER=1`),
split / close split, close bottom frame + terminals, reopen a listed session, board, and settle.

The script samples `/proc` RSS, threads and fds every 5 s. It aborts at 4 GB RSS. It fails on
any of these:

- RSS slope over the second half > 8 MB/min
- threads growing by > 16 over the first-quarter maximum
- fds growing by > 64 over the first-quarter maximum
- a crash log, a stall log, or a non-zero exit
- max frame time, only when `SOAK_MAX_FRAME_MS` is set (Xvfb frame times are software
  rendered)

`faux` is the real in-process `oino-core`: every bundled extension plus the provider-faux
fixture, so WASM, tools and sessions on disk are all exercised, at no network cost.

### Results (30 min each, release build with line tables, Xvfb + lavapipe)

| Run | Steps | RSS start→end (max) MB | RSS slope 2nd half | Threads q1max→end | fds q1max→end | Frame max / p99 ms | Verdict |
|---|---|---|---|---|---|---|---|
| baseline binary (pre-fix) | — | crashed in < 20 s (fix 1) | | | | | **crash** |
| faux, fixes, panes never released (`--max-panes 1000`, i.e. old behaviour) | ≈2560 | 5→1731 | **19.9 MB/min** | 276→333 | **500→842** | 195 / 18.7 | FAIL |
| faux, fixes (cap 8), run 1 | 2559 | 10→1229 | 7.2 MB/min | 225→260¹ | 400→386 | 120 / 18.0 | FAIL (threads) |
| faux, fixes + Drains bound, run 2 | 2564 | 4→1030 (1072) | **−3.4 MB/min** | 225→226 | 405→396 | 112 / 34.4 | **PASS** |
| demo, fixes | 2552 | 6→316 (360) | −1.8 MB/min | 184→182 | 73→61 | 92 / 15.9 | **PASS** |

¹ Run 1 gained about 30 threads in one step at t≈1515 s and kept them to the end. The same
workload in run 2, with a per-minute thread census (`oino-core` threads by wait channel), stayed
flat at 41–46 `oino-core` threads. Not reproduced. See the risks below.

Most of the 1 GB plateau in `faux` comes from one wasmtime `Services` per workspace with the
roughly 30 bundled components, each instantiated per session (8 live panes). It is flat once
the pane cap is reached.

## Whole-PC crash (previous boot, 2026-10-27 ~10:50)

`journalctl -b -1` ends at 10:50:36 with routine docker, veth and UniPi log lines. There is no
kernel oops, no NVRM/Xid, no OOM kill, no hung-task report, and `coredumpctl` has no entries.
earlyoom reported **swap full (0–27 MiB free of 8 GiB) all night**, with 44–64 % of RAM
available. The log shows a hard reset or power loss, not a kernel-detected failure.

My work running then was the release builds and one ~20 s baseline app run on Xvfb (it crashed
by itself, fix 1). No soak was running at that point, so the soaks are unlikely to be the
trigger. Full swap with heavy concurrent builds from several agents is the more likely pressure.

From now on: `cargo -j 8`, one app instance at a time, soaks on Xvfb + lavapipe (no NVIDIA
driver use), and a 4 GB RSS abort in `soak.sh`. The peak in the soaks above was 1.7 GB, in the
no-release run.

## Remaining risks

- **Thread step (run 1)**: about 30 threads appeared at once and stayed, most likely tokio
  blocking-pool threads stuck in a `spawn_blocking` or `block_in_place` path. Candidates are
  `oino-wasm-host` `call_blocking` / `core_bridge::block` and the `fs.lock` acquire. It did not
  reproduce. Re-run `soak.sh --backend faux` with `apps/target/tmp/tsample.sh`-style census if
  it shows up again.
- `block_in_place` on tokio workers for synchronous WASM exports can still starve the 4-worker
  runtime under a burst of extension calls. The UI does not block on it (every host call goes
  through `Tokio::spawn`), but replies are delayed.
- Released panes reopen from the sidebar with a fresh core session (history loads from the
  file). Their scroll position and unsent composer draft are lost, because both live in the
  pane. Unpersisted new sessions with no messages simply disappear.
- Frame times were measured on lavapipe only. Run a short soak on `DISPLAY=:0` with
  `SOAK_MAX_FRAME_MS=50` to judge the GPU path.
- The embedded browser (`OINO_SOAK_BROWSER=1`) was not soaked: WebKitGTK cannot embed on Xvfb
  without a session bus.
- The real-provider path (`oino/deepseek/deepseek-flash`) was not soaked. It uses the same
  `faux` code path except for the HTTP provider component.
