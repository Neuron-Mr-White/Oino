# Perf bench (`mise run perf`)

Repeatable RAM / CPU / IPC / disk / prompt-cache measurements of the real Unipi host and
desktop app. **Benches never run on a workstation that runs Unipi** (`run.sh` refuses when a
`unipi-oino` process is up); `mise run perf` creates an Agent37 VM, builds HEAD there, runs the
bench, copies the results to `target/perf-out/<vm>/` and deletes the VM.

```sh
mise run perf                                  # HEAD .deb built on the VM, scenarios a,b,d
mise run perf -- --deb unipi.deb -- --scenarios a,b,c,d --soak-hours 8 --owner-file chat.jsonl
mise run perf -- -- --scenarios b --sizes 50 --profile perf        # + perf flamegraph data
# on a bench VM directly:
PERF_FORCE=1 scripts/perf/run.sh --deb unipi.deb --out /root/perf-out --scenarios b --sizes 10
```

| File | Role |
|---|---|
| `fakellm.py` | deterministic OpenAI-compatible model (SSE helpers from `scripts/qa/phone-fakellm.py`): `PERF:spawn n= stream= secs=` (background `run_subagent` fan-out), `PERF-CHILD stream T / idle`, `PERF:stream T`, `PERF:tools`; records every request's system-prompt / tool-list / history prefix per conversation and reports cache hits and the break reason (`GET /stats`) |
| `bench.py` | host scenarios, each on a FRESH `unipi-oino-host serve` in an isolated root (`HOME`, `OINO_AGENT_DIR`, `OINO_HOST_DIR`, XDG) over `app.sock`: (a) long session open / scroll (`get_entries`) / stream / evict, (b) subagent graph N = 10 / 50 / 200 (half streaming), idle-live and post-hibernate phases, (c) soak (short sessions + tools + small graphs + a cron job) |
| `app.py` | (d) the installed desktop app (`.deb` tree) under Xvfb via `scripts/qa/rig.sh`: app/webview RSS + CPU, DOM size, rAF frame meter (fps, frames > 50 ms; exported as Chrome-trace JSON — WebKitGTK has no CDP) for the 5k-turn chat scroll and a 50-subagent streaming graph |
| `lib.py` | app.sock client (counts events, never stores them), `/proc` sampler (RSS / anon / PSS, CPU ticks, threads, fds, procs, wasm linear-memory reservations = anonymous mappings ≥ 1 GiB) |
| `gen-long-session.py` | synthetic 5,000-turn ≈ 200 MB session (v3 layout, compaction every 500 turns) |
| `run.sh` / `remote.sh` / `vm-setup.sh` | VM entry, Agent37 lifecycle, VM toolchain + profilers |
| `report.py` | `results.json` → `summary.md` tables |

Profiles: `--profile perf` records `perf record -g` of the host during the busy phase of the
`--profile-size` graph (fold with `/root/FlameGraph`); `--profile heaptrack --host-bin
/root/unipi-oino-host-gnu` runs a glibc build under heaptrack (the shipped host is static musl,
which heaptrack cannot preload into).

Outputs are bounded: CSV samples are streamed to disk, the fake model keeps ≤ 4096
conversations of hashes, event streams are counted only. Baseline numbers and budgets:
`oino-internal/docs/dev/v2/qa/perf-baseline.md`.
