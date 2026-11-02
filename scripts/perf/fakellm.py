#!/usr/bin/env python3
"""Deterministic fake OpenAI-compatible model for `mise run perf` (no network, no cost).

Reuses the SSE helpers of scripts/qa/phone-fakellm.py (same wire shape the bundled
`oino.openai-compat` provider parses) and adds:

* perf scripts picked by markers in the last user text:
    PERF:spawn n=K stream=S secs=T  -> K `run_subagent` background calls; the first S
                                       children stream for T s, the rest answer at once
    PERF-CHILD stream T             -> stream ~20 chunks/s for T s (a busy subagent)
    PERF-CHILD idle                 -> "ok" at once (an idle subagent)
    PERF:stream T                   -> the session itself streams for T s
    PERF:tools                      -> one `bash` call (echo), then a short answer
    anything else                   -> a short markdown answer (no sleep)
* prompt-prefix (provider cache) accounting: every request is compared with the
  previous request of the same conversation. A turn is a cache hit when the system
  prompt and the tool list are byte-identical and the earlier message list is an exact
  prefix of the new one; otherwise the break reason (system / tools +-names / history@i)
  is logged. `GET /stats` returns the counters (`?reset=1` zeroes them).

Memory is bounded: per-conversation state is an LRU of at most MAX_CONVS entries of
hashes only; the log is a JSONL file the harness rotates per run.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import re
import threading
import time
from collections import Counter, OrderedDict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

_spec = importlib.util.spec_from_file_location(
    "phone_fakellm", os.path.join(os.path.dirname(__file__), "..", "qa", "phone-fakellm.py"))
_pf = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_pf)
chunks, delta, text_of = _pf.chunks, _pf.delta, _pf.text_of

MAX_CONVS = 4096
CHUNK = "Streaming perf tokens, deterministic text. "
LOCK = threading.Lock()
CONVS = OrderedDict()          # conv key -> (sys_hash, tools_hash, tool_names, [msg hashes])
STATS = Counter()
BREAKS = Counter()
LOG = None


def h(obj):
    return hashlib.sha256(json.dumps(obj, sort_keys=True, separators=(",", ":")).encode()).hexdigest()[:16]


def conv_key(msgs):
    """The conversation a request belongs to: the `[conv:…]` marker of the first user
    message if any (the harness tags every prompt), else a hash of that message."""
    for m in msgs:
        if m.get("role") == "user":
            t = text_of(m)
            mk = re.search(r"\[conv:([\w.-]+)\]", t)
            return mk.group(1) if mk else "h" + h(t)
    return "none"


def account(req, body_len):
    msgs = req.get("messages", []) or []
    system = [m for m in msgs if m.get("role") in ("system", "developer")]
    rest = [m for m in msgs if m.get("role") not in ("system", "developer")]
    tools = req.get("tools", []) or []
    names = sorted(t.get("function", {}).get("name", "?") for t in tools)
    sys_h, tools_h = h(system), h(tools)
    hashes = [h(m) for m in rest]
    key = conv_key(msgs)
    rec = {"conv": key, "n_msgs": len(msgs), "bytes": body_len, "sys": sys_h, "tools": tools_h,
           "n_tools": len(tools)}
    with LOCK:
        prev = CONVS.pop(key, None)
        CONVS[key] = (sys_h, tools_h, names, hashes)
        while len(CONVS) > MAX_CONVS:
            CONVS.popitem(last=False)
        STATS["requests"] += 1
        STATS["bytes"] += body_len
        if prev is None:
            STATS["first_turns"] += 1
            rec["cache"] = "first"
            return rec
        reasons = []
        if prev[0] != sys_h:
            reasons.append("system")
        if prev[1] != tools_h:
            added = sorted(set(names) - set(prev[2]))
            removed = sorted(set(prev[2]) - set(names))
            reasons.append("tools" + (" +" + ",".join(added) if added else "")
                           + (" -" + ",".join(removed) if removed else "")
                           + ("" if added or removed else " (order/schema)"))
        old = prev[3]
        if hashes[:len(old)] != old:
            i = next((i for i, (a, b) in enumerate(zip(old, hashes)) if a != b), min(len(old), len(hashes)))
            reasons.append(f"history@{i}/{len(old)}")
        if reasons:
            STATS["misses"] += 1
            for r in reasons:
                BREAKS[r.split(" ")[0].split("@")[0]] += 1
                BREAKS["detail:" + r[:120]] += 1
            rec["cache"] = "miss"
            rec["why"] = reasons
        else:
            STATS["hits"] += 1
            rec["cache"] = "hit"
    return rec


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.startswith("/stats"):
            with LOCK:
                out = {"stats": dict(STATS), "breaks": dict(BREAKS), "convs": len(CONVS)}
                if "reset=1" in self.path:
                    STATS.clear(); BREAKS.clear(); CONVS.clear()
            return self._json(200, out)
        self._json(200, {"data": [{"id": "fake-1", "object": "model"}]})

    def sse(self, obj):
        data = b"data: " + json.dumps(obj).encode() + b"\n\n"
        self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data))
        self.wfile.flush()

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        raw = self.rfile.read(n) or b"{}"
        req = json.loads(raw)
        msgs = req.get("messages", []) or []
        tools = [t.get("function", {}).get("name") for t in req.get("tools", []) or []]
        last_user = next((text_of(m) for m in reversed(msgs) if m.get("role") == "user"), "")
        after_tool = bool(msgs) and msgs[-1].get("role") == "tool"
        rec = account(req, len(raw))
        rec.update({"t": round(time.time(), 3), "after_tool": after_tool, "last_user": last_user[:80]})
        if LOG:
            with LOCK:
                LOG.write(json.dumps(rec) + "\n"); LOG.flush()

        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("transfer-encoding", "chunked")
        self.end_headers()
        calls, text, secs = [], "Done.", 0.0
        m_spawn = re.search(r"PERF:spawn n=(\d+) stream=(\d+) secs=(\d+)", last_user)
        m_child = re.search(r"PERF-CHILD (stream|idle)(?: (\d+))?", last_user)
        m_stream = re.search(r"PERF:stream (\d+)", last_user)
        tag = re.search(r"\[conv:([\w.-]+)\]", last_user)
        tag = tag.group(1) if tag else "x"
        if after_tool:
            text = "Started. I will wait for the completion notifications."
        elif m_spawn and "run_subagent" in tools and msgs[-1].get("role") == "user" \
                and "PERF:spawn" in text_of(msgs[-1]):
            # Only on the spawn prompt itself: a later completion notification must not respawn.
            k, s, t = int(m_spawn.group(1)), int(m_spawn.group(2)), int(m_spawn.group(3))
            for i in range(k):
                mode = f"stream {t}" if i < s else "idle"
                calls.append(("run_subagent", {
                    "title": f"perf {tag}-{i}", "profile": "subagent_explore", "is_background": True,
                    "task": f"PERF-CHILD {mode} [conv:{tag}-c{i}]"}))
            text = f"Spawning {k} subagents."
        elif m_child and m_child.group(1) == "stream":
            secs = float(m_child.group(2) or 30)
        elif m_child:
            text = "ok"
        elif m_stream:
            secs = float(m_stream.group(1))
        elif "PERF:tools" in last_user and "bash" in tools:
            calls = [("bash", {"command": "echo perf-tool-ok"})]
            text = "Running a tool."
        else:
            text = _pf.MD if "PERF:md" in last_user else "Short answer for the soak."
        try:
            if secs:
                end = time.time() + secs
                while time.time() < end:
                    self.sse(delta(content=CHUNK)); time.sleep(0.05)
            else:
                for c in chunks(text, 64):
                    self.sse(delta(content=c))
            for i, (name, args) in enumerate(calls):
                self.sse(delta(tool_calls=[{"index": i, "id": f"call_{tag}_{i}_{int(time.time()*1000)}",
                                            "type": "function",
                                            "function": {"name": name, "arguments": json.dumps(args)}}]))
            self.sse(delta(finish="tool_calls" if calls else "stop"))
            ptok = len(raw) // 4
            cached = ptok if rec.get("cache") == "hit" else 0
            self.sse({"id": "c1", "object": "chat.completion.chunk", "choices": [],
                      "usage": {"prompt_tokens": ptok, "completion_tokens": 50, "total_tokens": ptok + 50,
                                "prompt_tokens_details": {"cached_tokens": cached}}})
            data = b"data: [DONE]\n\n"
            self.wfile.write(b"%x\r\n%s\r\n0\r\n\r\n" % (len(data), data)); self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    global LOG
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=int(os.environ.get("FAKELLM_PORT", 11434)))
    ap.add_argument("--log", default=os.environ.get("FAKELLM_LOG", "fakellm.jsonl"))
    a = ap.parse_args()
    LOG = open(a.log, "a")
    srv = ThreadingHTTPServer(("127.0.0.1", a.port), H)
    srv.daemon_threads = True
    srv.serve_forever()


if __name__ == "__main__":
    main()
