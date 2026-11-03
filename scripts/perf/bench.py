#!/usr/bin/env python3
"""Perf bench driver (`mise run perf`, see scripts/perf/README.md). Runs ON THE BENCH VM.

    bench.py --host BIN --out DIR [--scenarios a,b,c,d] [--sizes 10,50,200] [--soak-hours 8]
             [--long-file JSONL] [--owner-file JSONL] [--profile heaptrack|perf] [--app BIN]

Each scenario starts a FRESH host in its own isolated root (HOME / OINO_AGENT_DIR /
OINO_HOST_DIR under DIR/run-*), talks to it over app.sock like the app does, drives the
fake model (fakellm.py), samples /proc and writes DIR/results.json + DIR/summary.md.
"""
import argparse
import json
import math
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = {}


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def save(out):
    with open(f"{out}/results.json", "w") as f:
        json.dump(RESULTS, f, indent=1)


class Fake:
    def __init__(self, out, port):
        self.port = port
        self.p = subprocess.Popen([sys.executable, f"{HERE}/fakellm.py", "--port", str(port),
                                   "--log", f"{out}/fakellm-{port}.jsonl"])
        for _ in range(100):
            try:
                self.stats(reset=True)
                return
            except OSError:
                time.sleep(0.1)
        raise RuntimeError("fakellm did not start")

    def stats(self, reset=False):
        url = f"http://127.0.0.1:{self.port}/stats" + ("?reset=1" if reset else "")
        return json.load(urllib.request.urlopen(url, timeout=5))

    def stop(self):
        self.p.terminate()
        self.p.wait(5)


class Host:
    """A fresh `unipi-oino-host serve` in an isolated root (optionally under a profiler)."""

    def __init__(self, args, root, port, settings=None, profile=None, tag="host", seed=None):
        self.root = root
        shutil.rmtree(root, ignore_errors=True)
        lib.seed_agent_dir(root, port, settings)
        warm = f"{args.out}/warm-cache"
        self.warm = os.path.isdir(warm) and not getattr(args, "cold", False)
        if self.warm:  # compiled-component cache of an earlier run: measures a warm start
            shutil.copytree(warm, f"{root}/agent/cache", dirs_exist_ok=True)
        if seed:
            seed(root)
        self.env = lib.perf_env(root)
        self.sock = f"{root}/hostdir/app.sock"
        cmd = [args.host, "serve", "--mode", "manual"]
        self.profile_out = None
        if profile == "heaptrack":
            self.profile_out = f"{args.out}/heaptrack-{tag}"
            cmd = ["heaptrack", "-o", self.profile_out] + cmd
        self.log = open(f"{root}/host.out", "w")
        t0 = time.time()
        self.p = subprocess.Popen(cmd, env=self.env, stdout=self.log, stderr=subprocess.STDOUT,
                                  cwd=f"{root}/work", start_new_session=True)
        self.startup_s = lib.wait_sock(self.sock)
        self.startup_wall = time.time() - t0
        self.pid = self._real_pid()
        self.perf = None

    def _real_pid(self):
        # under heaptrack the host is a child of the heaptrack shell wrapper
        for pid in lib.descendants(self.p.pid):
            if (lib.proc(pid) or {}).get("comm", "").startswith("unipi"):
                return pid
        return self.p.pid

    def perf_record(self, out, secs):
        """`perf record -g` of the host for `secs` (call graph), folded later into a flamegraph."""
        return subprocess.Popen(["perf", "record", "-F", "199", "-g", "-p", str(self.pid), "-o", out,
                                 "--", "sleep", str(secs)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def op(self, req, timeout=60):
        return lib.op(self.sock, req, timeout)

    attached = None

    def stop(self):
        if self.attached:  # heaptrack -p writes its file when the target exits
            try:
                os.killpg(self.p.pid, signal.SIGTERM)
                self.attached.wait(300)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                pass
        warm = os.path.dirname(self.root) + "/warm-cache"
        if not os.path.isdir(warm) and os.path.isdir(f"{self.root}/agent/cache"):
            shutil.copytree(f"{self.root}/agent/cache", warm)
        try:
            os.killpg(self.p.pid, signal.SIGTERM)
            self.p.wait(30)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(self.p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.log.close()


def host_log_count(root, needle):
    n = 0
    for dirpath, _, files in os.walk(f"{root}/hostdir"):
        for fn in files:
            if fn.endswith(".log") or "log" in fn:
                try:
                    with open(os.path.join(dirpath, fn), errors="ignore") as f:
                        n += sum(1 for line in f if needle in line)
                except OSError:
                    pass
    try:
        with open(f"{root}/host.out", errors="ignore") as f:
            n += sum(1 for line in f if needle in line)
    except OSError:
        pass
    return n


def disk(root):
    return {k: lib.du_kib(f"{root}/{p}") for k, p in
            (("sessions_kib", "agent/sessions"), ("agent_kib", "agent"), ("hostdir_kib", "hostdir"),
             ("xdg_cache_kib", "xdg/cache"), ("home_kib", "home"))}


def snap(h, label, secs=10):
    s = lib.cpu_window(h.pid, secs)
    if s:
        s["label"] = label
        log(f"  [{label}] host rss {s['host_rss']/1024:.1f} MB anon {s['host_anon']/1024:.1f} MB "
            f"(wasm {s.get('wasm_kib', 0)/1024:.1f} MB in {s['wasm_mem']} memories, native {s.get('native_anon_kib', 0)/1024:.1f} MB) "
            f"cpu {s['cpu_pct']}% thr {s['host_threads']} fds {s['host_fds']} procs {s['procs']}")
    return s


# ---------------------------------------------------------------- (b) subagent graph

def scenario_graph(args, fake, n):
    tag = f"graph-{n}"
    root = f"{args.out}/run-{tag}"
    hib = args.hibernate_sec
    h = Host(args, root, fake.port, {"extensionSettings": {"subagents": {"hibernateAfterSec": hib}}},
             profile=args.profile if n == args.profile_size else None, tag=tag)
    sampler = lib.Sampler(h.pid, f"{args.out}/{tag}.csv", 1.0)
    sampler.start()
    r = {"n": n, "startup_s": round(h.startup_s, 3), "hibernate_after_s": hib}
    try:
        sampler.label = "idle0"
        time.sleep(3)
        r["idle_empty"] = snap(h, "idle-empty", 10)
        roots = math.ceil(n / 16)
        per = [n // roots + (1 if i < n % roots else 0) for i in range(roots)]
        streams = []
        for i in range(roots):
            st = lib.Stream(h.sock, {"op": "session_new", "cwd": f"{root}/work", "name": f"perf-root-{i}"})
            streams.append(st)
        r["first_session_new_s"] = round(streams[0].open_s, 2)
        r["next_session_new_s"] = round(max((x.open_s for x in streams[1:]), default=0), 2)
        r["warm_cache"] = h.warm
        r["roots"] = roots
        r["after_roots"] = snap(h, "roots", 5)
        if args.profile == "heaptrack-attach" and n == args.profile_size:
            # attach after start-up (compiles) so the record covers spawn → busy → idle → close
            h.attached = subprocess.Popen(["heaptrack", "-o", f"{args.out}/heaptrack-{tag}", "-p", str(h.pid)],
                                          stdout=open(f"{args.out}/heaptrack-{tag}.log", "w"), stderr=subprocess.STDOUT)
            time.sleep(10)
        fake.stats(reset=True)
        t_spawn = time.time()
        sampler.label = "spawn"
        pumps = []
        stream_secs = args.stream_secs
        for i, st in enumerate(streams):
            k = per[i]
            st.send({"type": "prompt", "message": f"PERF:spawn n={k} stream={k // 2} secs={stream_secs} [conv:{tag}-r{i}]"})
            pumps.append(st.pump_bg())
        time.sleep(min(25, stream_secs / 3))
        sampler.label = "busy"
        f0 = sum(s.frames for s in streams)
        i0 = sum(s.items for s in streams)
        b0 = sum(s.bytes for s in streams)
        perf_p = None
        if args.profile == "perf" and n == args.profile_size:
            perf_p = h.perf_record(f"{args.out}/perf-{tag}.data", 15)
        busy = snap(h, "busy (half streaming)", 15)
        if perf_p:
            perf_p.wait()
        fsecs = 15
        busy["ipc_frames_per_s_roots"] = round((sum(s.frames for s in streams) - f0) / fsecs, 1)
        busy["ipc_items_per_s_roots"] = round((sum(s.items for s in streams) - i0) / fsecs, 1)
        busy["ipc_kib_per_s_roots"] = round((sum(s.bytes for s in streams) - b0) / 1024 / fsecs, 1)
        r["busy"] = busy
        # all children idle (streamers finished), none hibernated yet
        wait_until = t_spawn + stream_secs + 20
        while time.time() < wait_until:
            time.sleep(1)
        sampler.label = "idle-live"
        r["idle_live"] = snap(h, "all idle, live", 10)
        r["fake_after_turns"] = fake.stats()
        # hibernation: idle children hibernate `hib` s after their last activity
        wait_until = t_spawn + stream_secs + hib + 40
        while time.time() < wait_until:
            time.sleep(1)
        sampler.label = "hibernated"
        r["hibernated"] = snap(h, "after hibernate window", 20)
        r["hibernated_log_lines"] = host_log_count(root, "subagent hibernated")
        r["woke_log_lines"] = host_log_count(root, "woke from hibernation")
        r["handler_timeouts"] = host_log_count(root, "handler timed out")
        r["child_session_files"] = sum(1 for dp, _, fs in os.walk(f"{root}/agent/state") for f in fs if f.endswith(".jsonl"))
        # close the root streams (app gone) and wait for idle eviction
        for (stop, _), st in zip(pumps, streams):
            stop.set()
            st.close()
        time.sleep(args.evict_wait)
        sampler.label = "closed"
        r["after_close"] = snap(h, f"roots closed +{args.evict_wait}s", 10)
        base = r["after_roots"]["host_rss"]
        r["per_subagent_kib"] = {
            "busy_mixed": round((r["busy"]["host_rss"] - base) / n, 1),
            "idle_live": round((r["idle_live"]["host_rss"] - base) / n, 1),
            "hibernated": round((r["hibernated"]["host_rss"] - base) / n, 1),
        }
        r["disk"] = disk(root)
        r["fake"] = fake.stats()
        r["handler_timeouts_total"] = host_log_count(root, "handler timed out")
    finally:
        sampler.stop()
        h.stop()
    return r


# ---------------------------------------------------------------- (a) one long session

def scenario_long(args, fake, label, src):
    tag = f"long-{label}"
    root = f"{args.out}/run-{tag}"
    work = f"{root}/work"

    def seed(rootdir):
        if label == "synthetic":
            subprocess.run([sys.executable, f"{HERE}/gen-long-session.py", f"{rootdir}/agent", work,
                            "--turns", str(args.long_turns)], check=True, stdout=subprocess.DEVNULL)
        else:
            esc = "--" + work.strip("/").replace("/", "-") + "--"
            d = f"{rootdir}/agent/sessions/{esc}"
            os.makedirs(d, exist_ok=True)
            dst = f"{d}/{os.path.basename(src)}"
            with open(src) as fi, open(dst, "w") as fo:
                head = json.loads(fi.readline())
                head["cwd"] = work
                fo.write(json.dumps(head) + "\n")
                shutil.copyfileobj(fi, fo, 1 << 20)

    h = Host(args, root, fake.port, {"extensionSettings": {"hub": {"sessionIdleSec": args.evict_wait // 2}}},
             tag=tag, seed=seed)
    sampler = lib.Sampler(h.pid, f"{args.out}/{tag}.csv", 0.5)
    sampler.start()
    r = {"file_mib": round(sum(os.path.getsize(os.path.join(dp, f)) for dp, _, fs in os.walk(f"{root}/agent/sessions") for f in fs) / 1048576, 1),
         "startup_s": round(h.startup_s, 3)}
    try:
        r["idle_empty"] = snap(h, "idle", 5)
        t0 = time.time()
        cards = h.op({"op": "sessions_list"})
        r["sessions_list_s"] = round(time.time() - t0, 3)
        sid = (cards.get("data") or [{}])[0].get("session_id")
        sampler.label = "open"
        st = lib.Stream(h.sock, {"op": "session_open", "session": sid}, timeout=600)
        r["open_s"] = round(st.open_s, 3)
        r["open_snapshot_kib"] = round(st.open_bytes / 1024, 1)
        r["open_entries"] = st.snap_entries
        r["page"] = st.page
        r["after_open"] = snap(h, "opened", 5)
        # scroll: page back through history like the timeline does
        sampler.label = "scroll"
        cursor = (st.page or {}).get("cursor")
        pages, t0 = 0, time.time()
        page_times = []
        while cursor and pages < args.scroll_pages:
            t1 = time.time()
            rep = st.request({"type": "get_entries", "before": cursor, "limit": 50}, timeout=120)
            page_times.append(time.time() - t1)
            data = (rep or {}).get("data") or {}
            # get_entries{before} reply: {entries, leafId, total, hasMore, cursor}
            cursor = data.get("cursor") if data.get("hasMore") else None
            pages += 1
        r["scroll_pages"] = pages
        if page_times:
            page_times.sort()
            r["scroll_page_ms_p50"] = round(page_times[len(page_times) // 2] * 1000, 1)
            r["scroll_page_ms_max"] = round(page_times[-1] * 1000, 1)
        r["after_scroll"] = snap(h, "scrolled", 5)
        # stream a reply into the long session (the whole context goes to the model)
        sampler.label = "stream"
        rep = st.request({"type": "set_model", "provider": "ollama", "modelId": "fake-1"})
        fake.stats(reset=True)
        t0 = time.time()
        st.send({"type": "prompt", "message": f"PERF:stream {args.long_stream_secs} [conv:{tag}]"})
        stop, th = st.pump_bg()
        busy = snap(h, "streaming", max(3, args.long_stream_secs - 4))
        st.settled.wait(args.long_stream_secs + 120)
        r["stream_first_delta_s"] = round(st.first_delta - t0, 2) if st.first_delta else None
        r["stream_total_s"] = round(time.time() - t0, 2)
        r["busy"] = busy
        # second turn: prompt prefix must be a cache hit
        st.settled.clear()
        st.send({"type": "prompt", "message": "PERF:md follow-up"})
        st.settled.wait(120)
        r["fake"] = fake.stats()
        r["hwm_kib"] = lib.proc(h.pid)["hwm"]
        sampler.label = "idle-open"
        r["idle_open"] = snap(h, "idle, stream open", 10)
        stop.set()
        st.close()
        sampler.label = "closed"
        time.sleep(args.evict_wait)
        r["after_close"] = snap(h, f"closed +{args.evict_wait}s (eviction)", 10)
        r["disk"] = disk(root)
    finally:
        sampler.stop()
        h.stop()
    return r


# ---------------------------------------------------------------- (c) soak

def scenario_soak(args, fake):
    tag = "soak"
    root = f"{args.out}/run-{tag}"
    h = Host(args, root, fake.port, {"extensionSettings": {"hub": {"sessionIdleSec": 60},
                                                          "subagents": {"hibernateAfterSec": 30}}}, tag=tag)
    sampler = lib.Sampler(h.pid, f"{args.out}/{tag}.csv", 30.0)
    sampler.start()
    r = {"hours": args.soak_hours, "startup_s": round(h.startup_s, 3), "points": []}
    end = time.time() + args.soak_hours * 3600
    work = f"{root}/work"
    try:
        job = h.op({"op": "cron_add", "name": "perf-soak", "prompt": "PERF:tools cron run",
                    "schedule": "every 10 minutes", "tz": "UTC", "cwd": work})
        r["cron_add"] = job.get("type")
        i = 0
        next_point = time.time()
        while time.time() < end:
            i += 1
            try:
                st = lib.Stream(h.sock, {"op": "session_new", "cwd": work, "name": f"soak-{i}"})
                st.send({"type": "prompt", "message": f"PERF:tools [conv:soak-{i}]"})
                st.pump(until=lambda f: st.settled.is_set())
                st.settled.clear()
                st.send({"type": "prompt", "message": "PERF:md second"})
                st.pump(until=lambda f: st.settled.is_set())
                if i % 20 == 0:  # every 20th session also spawns a few subagents
                    st.settled.clear()
                    st.send({"type": "prompt", "message": f"PERF:spawn n=4 stream=2 secs=5 [conv:soak-g{i}]"})
                    st.pump(until=lambda f: st.settled.is_set())
                st.close()
                if i % 50 == 0:
                    h.op({"op": "session_archive", "session": st.sid})
            except Exception as e:  # keep soaking; count failures
                r["errors"] = r.get("errors", 0) + 1
                r["last_error"] = str(e)[:300]
            if time.time() >= next_point:
                s = lib.tree(h.pid)
                s.update({"t_h": round((time.time() - (end - args.soak_hours * 3600)) / 3600, 3),
                          "sessions": i, "disk": disk(root)})
                r["points"].append({k: s[k] for k in ("t_h", "sessions", "host_rss", "host_anon", "host_threads",
                                                      "host_fds", "wasm_mem", "procs", "disk")})
                log(f"  soak t={s['t_h']}h sessions={i} rss={s['host_rss']/1024:.1f}MB fds={s['host_fds']} thr={s['host_threads']}")
                save_partial(args, r)
                next_point = time.time() + args.soak_point_s
            time.sleep(args.soak_gap_s)
        r["idle_end"] = snap(h, "soak end idle", 30)
        r["cron_runs"] = h.op({"op": "cron_runs", "id": job.get("data", {}).get("id")}).get("data") if job.get("type") == "ok" else None
        if isinstance(r["cron_runs"], dict):
            runs = r["cron_runs"].get("runs") or []
            r["cron_runs"] = {"count": len(runs), "statuses": sorted({x.get("status") for x in runs})}
        r["fake"] = fake.stats()
        r["disk"] = disk(root)
    finally:
        sampler.stop()
        h.stop()
    return r


def save_partial(args, r):
    RESULTS["soak"] = r
    save(args.out)


# ---------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--scenarios", default="a,b")
    ap.add_argument("--sizes", default="10,50,200")
    ap.add_argument("--stream-secs", type=int, default=60)
    ap.add_argument("--hibernate-sec", type=int, default=60)
    ap.add_argument("--evict-wait", type=int, default=40)
    ap.add_argument("--long-turns", type=int, default=5000)
    ap.add_argument("--long-stream-secs", type=int, default=20)
    ap.add_argument("--scroll-pages", type=int, default=40)
    ap.add_argument("--owner-file")
    ap.add_argument("--soak-hours", type=float, default=8.0)
    ap.add_argument("--soak-gap-s", type=float, default=20.0)
    ap.add_argument("--soak-point-s", type=float, default=300.0)
    ap.add_argument("--profile", choices=["heaptrack", "heaptrack-attach", "perf"])
    ap.add_argument("--profile-size", type=int, default=50)
    ap.add_argument("--port", type=int, default=11534)
    ap.add_argument("--tag", default="")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    prev = f"{args.out}/results.json"
    if os.path.exists(prev):
        RESULTS.update(json.load(open(prev)))
    RESULTS.setdefault("meta", {}).update({"host": args.host, "cpus": os.cpu_count(),
                                          "mem_kib": int(open("/proc/meminfo").readline().split()[1]),
                                          "kernel": os.uname().release})
    fake = Fake(args.out, args.port)
    sc = args.scenarios.split(",")
    sfx = args.tag
    try:
        if "a" in sc:
            log("== (a) long session: synthetic")
            RESULTS["long_synthetic" + sfx] = scenario_long(args, fake, "synthetic", None)
            save(args.out)
            if args.owner_file:
                log("== (a) long session: owner copy")
                RESULTS["long_owner" + sfx] = scenario_long(args, fake, "owner", args.owner_file)
                save(args.out)
        if "b" in sc:
            for n in [int(x) for x in args.sizes.split(",")]:
                log(f"== (b) graph n={n}")
                RESULTS[f"graph_{n}{sfx}"] = scenario_graph(args, fake, n)
                save(args.out)
        if "c" in sc:
            log(f"== (c) soak {args.soak_hours} h")
            RESULTS["soak" + sfx] = scenario_soak(args, fake)
            save(args.out)
    finally:
        fake.stop()
    log("done ->", f"{args.out}/results.json")


if __name__ == "__main__":
    main()
