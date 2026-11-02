"""Shared pieces of the perf harness: app.sock client, /proc sampler, isolated env.

Stdlib only (runs on a bare bench VM). Every buffer here is bounded: event streams are
counted, not stored; samples are written to CSV as they are taken.
"""
import json
import os
import socket
import struct
import subprocess
import threading
import time

PROTO = "2.1.0"
CLK = os.sysconf("SC_CLK_TCK")
PAGE = os.sysconf("SC_PAGE_SIZE")
WASM_RESERVATION = 1 << 30  # anonymous mappings >= 1 GiB = wasmtime linear-memory reservations


# ---------------------------------------------------------------- isolated environment

def perf_env(root):
    """HOME / OINO_AGENT_DIR / OINO_HOST_DIR / XDG under `root` (never the owner's dirs)."""
    env = dict(os.environ)
    env.update({
        "HOME": f"{root}/home", "OINO_AGENT_DIR": f"{root}/agent", "OINO_HOST_DIR": f"{root}/hostdir",
        "XDG_DATA_HOME": f"{root}/xdg/data", "XDG_CONFIG_HOME": f"{root}/xdg/config",
        "XDG_CACHE_HOME": f"{root}/xdg/cache", "XDG_STATE_HOME": f"{root}/xdg/state",
        "RUST_LOG": env.get("PERF_RUST_LOG", "info"),
    })
    return env


def seed_agent_dir(root, port, settings_extra=None):
    """models.json (fake model on the keyless `ollama` provider), settings, auth."""
    for d in ("home", "agent", "hostdir", "work", "xdg/data", "xdg/config", "xdg/cache", "xdg/state"):
        os.makedirs(f"{root}/{d}", exist_ok=True)
    models = {"providers": {"ollama": {
        "baseUrl": f"http://127.0.0.1:{port}/v1", "api": "openai-completions", "apiKey": "ollama",
        "models": [{"id": "fake-1", "name": "Fake perf model", "contextWindow": 200000,
                    "maxTokens": 8192, "input": ["text"]}]}}}
    settings = {"defaultProvider": "ollama", "defaultModel": "fake-1",
                "extensionSettings": {"subagents": {"maxConcurrent": 16}}}
    for k, v in (settings_extra or {}).items():
        if isinstance(v, dict) and isinstance(settings.get(k), dict):
            for k2, v2 in v.items():
                if isinstance(v2, dict) and isinstance(settings[k].get(k2), dict):
                    settings[k][k2].update(v2)
                else:
                    settings[k][k2] = v2
        else:
            settings[k] = v
    with open(f"{root}/agent/models.json", "w") as f:
        json.dump(models, f)
    with open(f"{root}/agent/settings.json", "w") as f:
        json.dump(settings, f)
    with open(f"{root}/agent/auth.json", "w") as f:
        json.dump({}, f)


# ---------------------------------------------------------------- app.sock client

def frame(kind, payload):
    return bytes([kind]) + struct.pack(">I", len(payload)) + payload


def hello(req):
    return frame(0, json.dumps({"type": "hello", "proto": PROTO, "req": req}).encode())


def read_frame(s):
    h = b""
    while len(h) < 5:
        c = s.recv(5 - len(h))
        if not c:
            raise EOFError
        h += c
    n = struct.unpack(">I", h[1:])[0]
    b = bytearray()
    while len(b) < n:
        c = s.recv(min(1 << 20, n - len(b)))
        if not c:
            raise EOFError
        b += c
    return h[0], bytes(b)


def op(sock_path, req, timeout=60):
    """One request/response op (`sessions_list`, `host_info`, `cron_*` …)."""
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(timeout)
    s.connect(sock_path)
    s.sendall(hello(req))
    while True:
        k, b = read_frame(s)
        if k == 0:
            r = json.loads(b)
            if r.get("type") in ("ok", "err", "exit"):
                s.close()
                return r


def wait_sock(sock_path, timeout=120):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            r = op(sock_path, {"op": "host_info"}, timeout=5)
            if r.get("type") == "ok":
                return time.time() - t0
        except OSError:
            pass
        time.sleep(0.05)
    raise TimeoutError(f"host never answered on {sock_path}")


class Stream:
    """One session stream. Events are counted (frames, items, bytes) — never stored."""

    def __init__(self, sock_path, req, timeout=300):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.settimeout(timeout)
        self.s.connect(sock_path)
        t0 = time.time()
        self.s.sendall(hello(req))
        k, b = read_frame(self.s)
        self.open_s = time.time() - t0
        self.open_bytes = len(b)
        r = json.loads(b)
        if r.get("type") != "ok":
            raise RuntimeError(f"open failed: {str(r)[:300]}")
        self.snap = r["data"]
        self.snap_entries = len(self.snap.get("entries") or [])
        self.page = self.snap.get("page")
        self.sid = self.snap.get("session_id")
        self.frames = self.items = self.bytes = self.deltas = 0
        self.settled = threading.Event()
        self.first_delta = None
        self.replies = {}
        self.lock = threading.Lock()
        self.closed = False
        self._seq = 0

    def send(self, cmd, cid=None):
        self._seq += 1
        cid = cid or f"c{self._seq}"
        self.s.sendall(frame(0, json.dumps({"type": "session_cmd", "id": cid, "cmd": cmd}).encode()))
        return cid

    def pump(self, until=None, stop=None):
        """Reads frames until `until(frame)` is true, `stop` is set, or EOF."""
        while not (stop and stop.is_set()):
            try:
                k, b = read_frame(self.s)
            except (EOFError, OSError):
                self.closed = True
                return None
            if k != 0:
                continue
            with self.lock:
                self.frames += 1
                self.bytes += len(b)
            f = json.loads(b)
            t = f.get("type")
            if t == "events":
                for it in f.get("items", []):
                    self.items += 1
                    et = (it.get("event") or {}).get("type")
                    if et == "message_update":
                        self.deltas += 1
                        if self.first_delta is None:
                            self.first_delta = time.time()
                    elif et == "agent_settled":
                        self.settled.set()
            elif t in ("ok", "err") and f.get("id"):
                self.replies[f["id"]] = f
            if until and until(f):
                return f
        return None

    def request(self, cmd, timeout=120):
        cid = self.send(cmd)
        self.s.settimeout(timeout)
        return self.pump(until=lambda f: f.get("id") == cid and f.get("type") in ("ok", "err"))

    def pump_bg(self):
        stop = threading.Event()
        th = threading.Thread(target=self.pump, kwargs={"stop": stop}, daemon=True)
        self.s.settimeout(None)
        th.start()
        return stop, th

    def close(self):
        try:
            self.s.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.s.close()


# ---------------------------------------------------------------- /proc sampler

def _read(p):
    try:
        with open(p) as f:
            return f.read()
    except OSError:
        return ""


def descendants(pid):
    """pid plus every descendant (from /proc/*/stat ppid)."""
    kids = {}
    for d in os.listdir("/proc"):
        if d.isdigit():
            st = _read(f"/proc/{d}/stat")
            if st:
                ppid = int(st.rsplit(")", 1)[1].split()[1])
                kids.setdefault(ppid, []).append(int(d))
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        out.append(p)
        todo.extend(kids.get(p, []))
    return out


def proc(pid, deep=False):
    """One process: rss/anon/file KiB, threads, fds, cpu ticks, wasm reservations, comm."""
    st = _read(f"/proc/{pid}/status")
    if not st:
        return None
    kv = {}
    for line in st.splitlines():
        k, _, v = line.partition(":")
        kv[k] = v.strip()
    kib = lambda k: int(kv.get(k, "0 kB").split()[0])
    stat = _read(f"/proc/{pid}/stat").rsplit(")", 1)[1].split()
    r = {"pid": pid, "comm": kv.get("Name", "?"), "rss": kib("VmRSS"), "anon": kib("RssAnon"),
         "file": kib("RssFile"), "hwm": kib("VmHWM"), "vsz": kib("VmSize"),
         "threads": int(kv.get("Threads", "0")), "ticks": int(stat[11]) + int(stat[12])}
    try:
        r["fds"] = len(os.listdir(f"/proc/{pid}/fd"))
    except OSError:
        r["fds"] = -1
    if deep:
        big = 0
        maps = _read(f"/proc/{pid}/maps")
        for line in maps.splitlines():
            parts = line.split()
            a, b = parts[0].split("-")
            # anonymous (no path column) and >= 1 GiB: a wasm linear-memory reservation
            if len(parts) < 6 and int(b, 16) - int(a, 16) >= WASM_RESERVATION:
                big += 1
        r["wasm_mem"] = big
        r["maps"] = len(maps.splitlines())
        roll = _read(f"/proc/{pid}/smaps_rollup")
        for line in roll.splitlines():
            if line.startswith("Pss:"):
                r["pss"] = int(line.split()[1])
    return r


def tree(pid, deep=True):
    """Aggregate of the host process tree (host + cron `oino` runs + PTYs …)."""
    ps = [p for p in (proc(x, deep and x == pid) for x in descendants(pid)) if p]
    if not ps:
        return None
    top = ps[0]
    agg = {"host_rss": top["rss"], "host_anon": top["anon"], "host_hwm": top["hwm"],
           "host_threads": top["threads"], "host_fds": top["fds"], "host_ticks": top["ticks"],
           "wasm_mem": top.get("wasm_mem"), "maps": top.get("maps"), "pss": top.get("pss"),
           "procs": len(ps), "tree_rss": sum(p["rss"] for p in ps),
           "tree_threads": sum(p["threads"] for p in ps), "tree_fds": sum(max(p["fds"], 0) for p in ps),
           "tree_ticks": sum(p["ticks"] for p in ps),
           "children": sorted({p["comm"] for p in ps[1:]})}
    return agg


def cpu_window(pid, secs):
    """Host-tree CPU % (of one core) over `secs`, plus the end sample."""
    a = tree(pid, deep=False)
    t0 = time.time()
    time.sleep(secs)
    b = tree(pid)
    dt = time.time() - t0
    if not a or not b:
        return None
    b["cpu_pct"] = round(100.0 * (b["tree_ticks"] - a["tree_ticks"]) / CLK / dt, 2)
    b["host_cpu_pct"] = round(100.0 * (b["host_ticks"] - a["host_ticks"]) / CLK / dt, 2)
    return b


class Sampler(threading.Thread):
    """Samples `tree(pid)` every `every` s into a CSV (one line per sample; no in-RAM list)."""

    COLS = ["t", "host_rss", "host_anon", "pss", "host_threads", "host_fds", "wasm_mem", "maps",
            "procs", "tree_rss", "tree_threads", "tree_fds", "tree_ticks", "label"]

    def __init__(self, pid, path, every=1.0, extra_pids=None):
        super().__init__(daemon=True)
        self.pid, self.every, self.label = pid, every, ""
        self.extra = extra_pids or (lambda: [])
        self.f = open(path, "w")
        self.f.write(",".join(self.COLS + ["app_rss"]) + "\n")
        self.stop_ev = threading.Event()

    def run(self):
        t0 = time.time()
        while not self.stop_ev.is_set():
            s = tree(self.pid)
            if s:
                s["t"] = round(time.time() - t0, 1)
                s["label"] = self.label
                app = sum((proc(p) or {"rss": 0})["rss"] for p in self.extra())
                self.f.write(",".join(str(s.get(c, "")) for c in self.COLS) + f",{app}\n")
                self.f.flush()
            self.stop_ev.wait(self.every)

    def stop(self):
        self.stop_ev.set()
        self.join(5)
        self.f.close()


def du_kib(path):
    try:
        out = subprocess.run(["du", "-sk", path], capture_output=True, text=True).stdout
        return int(out.split()[0])
    except (ValueError, IndexError):
        return 0
