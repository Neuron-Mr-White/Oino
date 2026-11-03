#!/usr/bin/env python3
"""(d) The real desktop app with everything visible: long chat + 50-subagent graph.

    app.py --deb UNIPI.deb --out DIR [--turns 5000] [--agents 50] [--stream-secs 90]

Runs the INSTALLED app file tree (.deb) under Xvfb via scripts/qa/rig.sh (tauri-driver +
WebKitWebDriver) with an isolated QA_HOME under DIR, lets the app start its own host
sidecar, seeds a long session + a subagent graph over app.sock, and measures: app process
RSS / CPU (main + WebKitWebProcess + WebKitNetworkProcess), webview frame timing (rAF meter:
fps, frames > 50 ms; WebKitGTK has no CDP, so the frame log is exported as a
Chrome-trace JSON for chrome://tracing / Perfetto), DOM node counts, startup and IPC rate.
"""
import argparse
import json
import math
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402
import lib  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
R = {}

METER = r"""
// The loop runs only while recording: a meter left looping after its window kept the webview
// at ~4 % CPU in every later "idle" phase (perf-baseline before perf-app measured that).
window.__pm = window.__pm || {on:false, t:[], max:12000, looping:false};
const m0 = window.__pm; m0.t = []; m0.on = true;
if (!m0.looping) {
  m0.looping = true;
  const loop = (ts) => { const m = window.__pm; if (!m.on) { m.looping = false; return; } if (m.t.length < m.max) m.t.push(ts); requestAnimationFrame(loop); };
  requestAnimationFrame(loop);
}
return true;
"""
READ = r"""
const m = window.__pm; m.on = false; const t = m.t; const d = [];
for (let i = 1; i < t.length; i++) d.push(t[i] - t[i-1]);
const s = d.slice().sort((a,b)=>a-b); const dur = t.length > 1 ? (t[t.length-1]-t[0]) : 0;
return {frames: t.length, dur_ms: Math.round(dur), fps: dur ? +(1000*(t.length-1)/dur).toFixed(1) : 0,
  p50: s[Math.floor(s.length*0.5)]||0, p95: s[Math.floor(s.length*0.95)]||0, max: s[s.length-1]||0,
  long50: d.filter(x=>x>50).length, long100: d.filter(x=>x>100).length,
  dom: document.getElementsByTagName('*').length, t0: t[0]||0, deltas: d.map(x=>Math.round(x*10)/10)};
"""
SCROLLER = r"""
const all = Array.from(document.querySelectorAll('*')).filter(e => {
  const cs = getComputedStyle(e); return /(auto|scroll)/.test(cs.overflowY) && e.scrollHeight > e.clientHeight + 200; });
all.sort((a,b)=> (b.clientHeight*b.clientWidth) - (a.clientHeight*a.clientWidth));
const el = all[0]; if (!el) return null; window.__sc = el;
return {cls: el.className, sh: el.scrollHeight, ch: el.clientHeight};
"""

# What still animates in a phase (idle CPU attribution): running CSS animations / transitions
# grouped by name + target class, the DOM size, and the app's own perf-app counters.
PROBE = r"""
const out = {dom: document.getElementsByTagName('*').length, hidden: document.hidden, anims: 0, top: []};
try {
  const g = {};
  for (const a of document.getAnimations()) {
    if (a.playState !== 'running') continue;
    out.anims++;
    const t = a.effect && a.effect.target;
    const cls = t ? ((t.getAttribute && (t.getAttribute('class') || t.getAttribute('data-track'))) || t.tagName) : '?';
    const k = (a.animationName || a.transitionProperty || 'anim') + ' @ ' + String(cls).slice(0, 60);
    g[k] = (g[k] || 0) + 1;
  }
  out.top = Object.entries(g).sort((a, b) => b[1] - a[1]).slice(0, 12);
} catch (e) { out.err = String(e); }
if (window.__UNIPI_IDLE__) out.idle = window.__UNIPI_IDLE__();
return out;
"""

# --diag: counts timer / frame / IPC callbacks per call site during a phase (WebKitGTK has no
# profiler over WebDriver). Sites are the scheduling stack's first app frame (bundle file:line:col).
TIMERS = r"""
if (!window.__pt) {
  const pt = window.__pt = {c: {}, on: false};
  const site = () => { const s = (new Error().stack || '').split('\n'); for (const l of s.slice(2)) { if (!/__pt|TIMERS/.test(l)) return l.trim().replace(/^.*\//, '').slice(0, 90); } return '?'; };
  const hit = (k) => { if (pt.on) pt.c[k] = (pt.c[k] || 0) + 1; };
  const st = window.setTimeout, si = window.setInterval, raf = window.requestAnimationFrame;
  window.setTimeout = function (f, ms, ...a) { const k = 'T' + (ms|0) + ' ' + site(); return st.call(window, typeof f === 'function' ? function () { hit(k); return f.apply(this, arguments); } : f, ms, ...a); };
  window.setInterval = function (f, ms, ...a) { const k = 'I' + (ms|0) + ' ' + site(); return si.call(window, typeof f === 'function' ? function () { hit(k); return f.apply(this, arguments); } : f, ms, ...a); };
  window.requestAnimationFrame = function (f) { const k = 'R ' + site(); return raf.call(window, function (t) { hit(k); return f(t); }); };
  const ti = window.__TAURI_INTERNALS__;
  if (ti && ti.invoke) { const inv = ti.invoke.bind(ti); ti.invoke = function (cmd, args, o) { let d = ''; try { const r = args && (args.request || args.req || args.frame || args); d = r && (r.op || r.type || (r.cmd && r.cmd.type)) || ''; } catch (e) {} hit('IPC ' + cmd + ' ' + String(d).slice(0, 40)); return inv(cmd, args, o); }; }
}
return true;
"""
TIMERS_START = "if (window.__pt) { window.__pt.c = {}; window.__pt.on = true; } return !!window.__pt;"
TIMERS_READ = "const p = window.__pt; if (!p) return null; p.on = false; return Object.entries(p.c).sort((a, b) => b[1] - a[1]).slice(0, 25);"
DIAG = {"on": False}


def rig(env, *a, check=True):
    return subprocess.run(["bash", f"{REPO}/scripts/qa/rig.sh", *a], env=env, check=check,
                          capture_output=True, text=True)


def mjs(env, *a):
    p = subprocess.run(["node", f"{REPO}/scripts/qa/rig.mjs", *a], env=env, capture_output=True, text=True, timeout=180)
    if p.returncode:
        raise RuntimeError(p.stderr[-500:])
    return p.stdout.strip()


def ev(env, js):
    out = mjs(env, "eval", js)
    try:
        return json.loads(out) if out else None
    except json.JSONDecodeError:
        return out


def app_pids():
    out = []
    for pid in (int(d) for d in os.listdir("/proc") if d.isdigit()):
        c = (lib.proc(pid) or {}).get("comm", "")
        if c in ("unipi-oino", "WebKitWebProces", "WebKitNetworkPr", "WebKitWebProcess", "WebKitNetworkProcess", "WebKitGPUProces"):
            out.append(pid)
    return out


def thread_ticks(pids):
    """{(pid comm, thread comm): ticks} for every thread of `pids` (attribution of idle CPU)."""
    out = {}
    for pid in pids:
        pc = (lib.proc(pid) or {}).get("comm", "?")
        try:
            tids = os.listdir(f"/proc/{pid}/task")
        except OSError:
            continue
        for tid in tids:
            try:
                raw = open(f"/proc/{pid}/task/{tid}/stat").read()
            except OSError:
                continue
            comm = raw[raw.index("(") + 1:raw.rindex(")")]
            f = raw.rsplit(")", 1)[1].split()
            k = f"{pc}/{comm}"
            out[k] = out.get(k, 0) + int(f[11]) + int(f[12])
    return out


def app_sample(secs=10, env=None):
    probe = None
    if env is not None:
        try:
            probe = ev(env, PROBE)
        except Exception as e:  # the probe never fails a phase
            probe = {"err": str(e)[:200]}
    if env is not None and DIAG["on"]:
        try:
            ev(env, TIMERS_START)
        except Exception:
            pass
    pids = app_pids()
    a = {p: lib.proc(p) for p in pids}
    th0 = thread_ticks(pids) if DIAG["on"] else None
    time.sleep(secs)
    if th0 is not None:
        th1 = thread_ticks(pids)
        busy = {k: round(100.0 * (v - th0.get(k, 0)) / lib.CLK / secs, 2) for k, v in th1.items()}
        probe = dict(probe or {}, threads=sorted(((k, v) for k, v in busy.items() if v > 0), key=lambda kv: -kv[1])[:10])
    if env is not None and DIAG["on"]:
        try:
            probe = dict(probe or {}, timers=ev(env, TIMERS_READ))
        except Exception as e:
            probe = dict(probe or {}, timers_err=str(e)[:200])
    out = {"procs": {}}
    tot_rss = tot_cpu = 0
    for p in pids:
        b = lib.proc(p)
        if not b or not a.get(p):
            continue
        cpu = round(100.0 * (b["ticks"] - a[p]["ticks"]) / lib.CLK / secs, 1)
        key = f"{b['comm']}:{p}"
        out["procs"][key] = {"rss_mb": round(b["rss"] / 1024, 1), "cpu_pct": cpu, "threads": b["threads"]}
        tot_rss += b["rss"]
        tot_cpu += cpu
    out["app_rss_mb"] = round(tot_rss / 1024, 1)
    out["app_cpu_pct"] = round(tot_cpu, 1)
    out["webview_rss_mb"] = round(sum(v["rss_mb"] for k, v in out["procs"].items() if k.startswith("WebKitWeb")), 1)
    if probe is not None:
        out["probe"] = probe
    return out


def meter(env, secs, action=None, name="trace"):
    ev(env, METER)
    t_end = time.time() + secs
    while time.time() < t_end:
        if action:
            action()
        time.sleep(0.25)
    m = ev(env, READ)
    if isinstance(m, dict):
        # Chrome trace (chrome://tracing / Perfetto): one complete event per frame interval
        ts, events = m.get("t0", 0) * 1000, []
        for d in m.pop("deltas", []):
            events.append({"name": "frame", "ph": "X", "ts": round(ts), "dur": round(d * 1000), "pid": 1, "tid": 1,
                           "cat": "long" if d > 50 else "frame"})
            ts += d * 1000
        with open(f"{R['_out']}/app-{name}.trace.json", "w") as f:
            json.dump({"traceEvents": events, "displayTimeUnit": "ms"}, f)
    return m


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--deb", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--turns", type=int, default=5000)
    ap.add_argument("--agents", type=int, default=50)
    ap.add_argument("--stream-secs", type=int, default=90)
    ap.add_argument("--port", type=int, default=11634)
    ap.add_argument("--diag", action="store_true", help="also sample idle phases with every CSS animation paused (attribution)")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    R["_out"] = out
    qa = f"{out}/run-app"
    subprocess.run(["rm", "-rf", qa])
    lib.seed_agent_dir(qa, a.port, {"extensionSettings": {"subagents": {"hibernateAfterSec": 600}}})
    work = f"{qa}/work"
    subprocess.run([sys.executable, f"{HERE}/gen-long-session.py", f"{qa}/agent", work, "--turns", str(a.turns)],
                   check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, QA_HOME=qa, QA_DISPLAY=":151", QA_PORT="4451", QA_SIZE="1440x900", QA_MAX_RIGS="3")
    fake = bench.Fake(out, a.port)
    sock = f"{qa}/agent/host/app.sock"
    try:
        t0 = time.time()
        rig(env, "up", a.deb)
        R["startup_host_s"] = round(lib.wait_sock(sock, 120) + (time.time() - t0), 2)
        for _ in range(120):
            if ev(env, "return document.querySelectorAll('[data-testid], nav, aside').length > 5"):
                break
            time.sleep(0.5)
        R["startup_ui_s"] = round(time.time() - t0, 2)
        # "Idle" means nothing loads any more: wait for the sidebar's session list (its skeleton
        # shimmers while sessions_list runs) before the idle sample.
        for _ in range(120):
            if ev(env, "return !document.querySelector('.sb-skeleton')"):
                break
            time.sleep(0.5)
        R["sessions_loaded_s"] = round(time.time() - t0, 2)
        if a.diag:
            DIAG["on"] = bool(ev(env, TIMERS))
        time.sleep(5)
        R["idle_home"] = app_sample(15, env)
        bench.log("idle home", R["idle_home"]["app_rss_mb"], "MB", R["idle_home"]["app_cpu_pct"], "%")

        def paused(name, secs=15):
            ev(env, "const s=document.createElement('style');s.id='__perf_pause';s.textContent='*,*::before,*::after{animation-play-state:paused!important}';document.head.append(s);return true")
            time.sleep(1)
            R[name] = app_sample(secs, env)
            ev(env, "document.getElementById('__perf_pause')?.remove();return true")
        if a.diag:
            paused("idle_home_paused")
            # no focused field either (a blinking caret repaints twice a second)
            ev(env, "document.activeElement && document.activeElement.blur && document.activeElement.blur(); return true")
            time.sleep(1)
            R["idle_home_blurred"] = app_sample(15, env)
        cards = lib.op(sock, {"op": "sessions_list"}).get("data") or []
        long_sid = cards[0]["session_id"] if cards else None
        host_id = ev(env, "const a=document.querySelector('a[href*=\"#/s/\"]');return a?a.getAttribute('href'):location.hash")
        R["href_sample"] = host_id
        # small chat first, then the 5k-turn chat
        small = lib.Stream(sock, {"op": "session_new", "cwd": work, "name": "perf-small"})
        small.request({"type": "prompt", "message": "PERF:md small"}, timeout=60)
        small.settled.wait(30)
        hid = "local"
        if isinstance(host_id, str) and "#/s/" in host_id:
            hid = host_id.split("#/s/")[1].split("/")[0]
        R["host_route_id"] = hid

        def go(sid):
            ev(env, f"location.hash = '#/s/{hid}/{sid}'; return location.hash")
            time.sleep(1)

        go(small.sid)
        time.sleep(4)
        R["small_open"] = app_sample(10, env)
        R["small_open"]["dom"] = ev(env, "return document.getElementsByTagName('*').length")
        t1 = time.time()
        go(long_sid)
        for _ in range(240):
            n = ev(env, "return document.getElementsByTagName('*').length")
            if isinstance(n, int) and n > (R["small_open"]["dom"] or 0) * 0.8:
                break
            time.sleep(0.25)
        R["long_open_s"] = round(time.time() - t1, 2)
        time.sleep(5)
        R["long_open"] = app_sample(10, env)
        R["long_open"]["dom"] = ev(env, "return document.getElementsByTagName('*').length")
        R["scroller"] = ev(env, SCROLLER)

        def scroll():
            ev(env, "const e=window.__sc; if(e){e.scrollTop=Math.max(0,e.scrollTop-900);} return e?e.scrollTop:null")
        R["long_scroll"] = meter(env, 15, scroll, "long-scroll")
        R["long_after_scroll"] = app_sample(5, env)
        bench.log("long chat", R["long_open"]["app_rss_mb"], "MB; scroll", R["long_scroll"])
        # subagent graph: 50 children all streaming, root chat open, tree visible
        roots = math.ceil(a.agents / 13)
        per = [a.agents // roots + (1 if i < a.agents % roots else 0) for i in range(roots)]
        streams = []
        for i in range(roots):
            st = lib.Stream(sock, {"op": "session_new", "cwd": work, "name": f"perf-graph-{i}"})
            st.send({"type": "prompt", "message": f"PERF:spawn n={per[i]} stream={per[i]} secs={a.stream_secs} [conv:app-r{i}]"})
            streams.append((st, st.pump_bg()))
        go(streams[0][0].sid)
        time.sleep(20)
        f0 = sum(s.frames for s, _ in streams)
        i0 = sum(s.items for s, _ in streams)
        R["graph_busy"] = app_sample(10, env)
        R["graph_busy"]["dom"] = ev(env, "return document.getElementsByTagName('*').length")
        R["graph_busy"]["host_ipc_frames_per_s_roots"] = round((sum(s.frames for s, _ in streams) - f0) / 10, 1)
        R["graph_busy"]["host_ipc_items_per_s_roots"] = round((sum(s.items for s, _ in streams) - i0) / 10, 1)
        R["graph_fps"] = meter(env, 15, None, "graph-50-streaming")
        mjs(env, "shot", "perf-graph-50")
        bench.log("graph busy", R["graph_busy"]["app_rss_mb"], "MB", R["graph_busy"]["app_cpu_pct"], "%", R["graph_fps"])
        time.sleep(max(0, a.stream_secs - 40))
        time.sleep(30)
        R["graph_idle"] = app_sample(15, env)
        R["graph_idle"]["dom"] = ev(env, "return document.getElementsByTagName('*').length")
        if a.diag:
            paused("graph_idle_paused")
        go(long_sid)
        time.sleep(5)
        R["long_again"] = app_sample(10, env)
        for st, (stop, _) in streams:
            stop.set()
            st.close()
        R["fake"] = fake.stats()
    finally:
        with open(f"{out}/app-results.json", "w") as f:
            json.dump({k: v for k, v in R.items() if not k.startswith("_")}, f, indent=1)
        try:
            mjs(env, "shot", "perf-final")
        except Exception:
            pass
        rig(env, "down", check=False)
        fake.stop()
    print(json.dumps({k: v for k, v in R.items() if not k.startswith("_")}, indent=1)[:4000])


if __name__ == "__main__":
    main()
