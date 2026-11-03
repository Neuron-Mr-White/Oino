#!/usr/bin/env python3
"""results.json (+ app-results.json) -> markdown tables. Usage: report.py OUT_DIR"""
import json
import os
import sys

out = sys.argv[1]
R = json.load(open(f"{out}/results.json")) if os.path.exists(f"{out}/results.json") else {}
A = json.load(open(f"{out}/app-results.json")) if os.path.exists(f"{out}/app-results.json") else {}
mb = lambda kib: "—" if kib in (None, "") else f"{kib / 1024:.1f}"
g = lambda d, *ks: (lambda v: v)(__import__("functools").reduce(lambda a, k: (a or {}).get(k) if isinstance(a, dict) else None, ks, d))
L = []
P = L.append
m = R.get("meta", {})
P(f"# perf bench summary\n\nhost `{m.get('host')}` · {m.get('cpus')} cpus · {mb(m.get('mem_kib'))} MB RAM · kernel {m.get('kernel')}\n")

graphs = sorted((k for k in R if k.startswith("graph_")), key=lambda k: (len(k), k))
if graphs:
    P("## (b) subagent graph (half the children stream, the rest answer at once)\n")
    P("| run | roots | phase | host RSS MB | anon MB | wasm linear MB | native anon MB | CPU % (tree) | threads | fds | wasm mems | procs |")
    P("|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for k in graphs:
        r = R[k]
        for ph in ("idle_empty", "after_roots", "busy", "idle_live", "hibernated", "after_close"):
            s = r.get(ph) or {}
            P(f"| {k} | {r.get('roots')} | {ph} | {mb(s.get('host_rss'))} | {mb(s.get('host_anon'))} | {mb(s.get('wasm_kib'))} | {mb(s.get('native_anon_kib'))} | {s.get('cpu_pct', '—')} | "
              f"{s.get('host_threads', '—')} | {s.get('host_fds', '—')} | {s.get('wasm_mem', '—')} | {s.get('procs', '—')} |")
    P("\n| run | KiB/subagent busy-mixed | idle-live | after hibernate window | hibernated (log) | IPC frames/s (roots) | items/s | KiB/s | cache hits / misses | breaks |")
    P("|---:|---:|---:|---:|---:|---:|---:|---:|---|---|")
    for k in graphs:
        r = R[k]
        p = r.get("per_subagent_kib", {})
        f = (r.get("fake") or {}).get("stats", {})
        b = r.get("busy") or {}
        P(f"| {k} | {p.get('busy_mixed')} | {p.get('idle_live')} | {p.get('hibernated')} | {r.get('hibernated_log_lines')} | "
          f"{b.get('ipc_frames_per_s_roots')} | {b.get('ipc_items_per_s_roots')} | {b.get('ipc_kib_per_s_roots')} | "
          f"{f.get('hits', 0)} / {f.get('misses', 0)} | {json.dumps({x: y for x, y in (r.get('fake') or {}).get('breaks', {}).items() if not x.startswith('detail')})} |")
    P("")

for k in (k for k in R if k.startswith("long_")):
    r = R[k]
    P(f"## (a) long session — {k}\n")
    P("| metric | value |\n|---|---:|")
    rows = [("file MB", r.get("file_mib")), ("host startup s", r.get("startup_s")), ("sessions_list s", r.get("sessions_list_s")),
            ("session_open s", r.get("open_s")), ("open snapshot KiB / entries", f"{r.get('open_snapshot_kib')} / {r.get('open_entries')}"),
            ("history total entries", g(r, "page", "total")),
            ("scroll pages / p50 ms / max ms", f"{r.get('scroll_pages')} / {r.get('scroll_page_ms_p50')} / {r.get('scroll_page_ms_max')}"),
            ("RSS idle empty MB", mb(g(r, "idle_empty", "host_rss"))), ("RSS after open MB", mb(g(r, "after_open", "host_rss"))),
            ("RSS after scroll MB", mb(g(r, "after_scroll", "host_rss"))), ("RSS streaming MB", mb(g(r, "busy", "host_rss"))),
            ("CPU % streaming", g(r, "busy", "cpu_pct")), ("VmHWM MB", mb(r.get("hwm_kib"))),
            ("first delta s / total s", f"{r.get('stream_first_delta_s')} / {r.get('stream_total_s')}"),
            ("request bytes (2 turns)", g(r, "fake", "stats", "bytes")),
            ("cache hits / misses / first", f"{g(r, 'fake', 'stats', 'hits')} / {g(r, 'fake', 'stats', 'misses')} / {g(r, 'fake', 'stats', 'first_turns')}"),
            ("RSS idle (stream open) MB / CPU %", f"{mb(g(r, 'idle_open', 'host_rss'))} / {g(r, 'idle_open', 'cpu_pct')}"),
            ("RSS after close + eviction MB", mb(g(r, "after_close", "host_rss")))]
    for a, b in rows:
        P(f"| {a} | {b} |")
    P("")

s = R.get("soak")
if s:
    P(f"## (c) soak — {s.get('hours')} h\n")
    P("| t (h) | sessions | host RSS MB | anon MB | threads | fds | wasm mems | procs | sessions dir MB | hostdir MB |")
    P("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for p in s.get("points", []):
        d = p.get("disk", {})
        P(f"| {p['t_h']} | {p['sessions']} | {mb(p['host_rss'])} | {mb(p['host_anon'])} | {p['host_threads']} | {p['host_fds']} | "
          f"{p.get('wasm_mem')} | {p['procs']} | {mb(d.get('sessions_kib'))} | {mb(d.get('hostdir_kib'))} |")
    P(f"\nerrors: {s.get('errors', 0)} · cron runs: {s.get('cron_runs')} · end idle CPU: {g(s, 'idle_end', 'cpu_pct')} % · fake: {g(s, 'fake', 'stats')}\n")

if A:
    P("## (d) desktop app (WebKitGTK webview)\n")
    P("| phase | app RSS MB (all procs) | webview RSS MB | app CPU % | DOM nodes |")
    P("|---|---:|---:|---:|---:|")
    for ph in ("idle_home", "idle_home_paused", "idle_home_blurred", "small_open", "long_open", "long_after_scroll", "graph_busy", "graph_idle", "graph_idle_paused", "long_again"):
        x = A.get(ph) or {}
        if not x:
            continue
        P(f"| {ph} | {x.get('app_rss_mb')} | {x.get('webview_rss_mb')} | {x.get('app_cpu_pct')} | {x.get('dom', '—')} |")
    P("\n| frame meter | fps | p50 ms | p95 ms | max ms | frames > 50 ms | > 100 ms |")
    P("|---|---:|---:|---:|---:|---:|---:|")
    for ph in ("long_scroll", "graph_fps"):
        x = A.get(ph) or {}
        P(f"| {ph} | {x.get('fps')} | {x.get('p50')} | {x.get('p95')} | {x.get('max')} | {x.get('long50')} | {x.get('long100')} |")
    P(f"\nstartup: host {A.get('startup_host_s')} s, UI {A.get('startup_ui_s')} s · long chat open {A.get('long_open_s')} s · "
      f"host IPC to roots while 50 stream: {g(A, 'graph_busy', 'host_ipc_frames_per_s_roots')} frames/s\n")
print("\n".join(L))
