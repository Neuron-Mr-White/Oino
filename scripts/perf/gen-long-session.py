#!/usr/bin/env python3
"""Writes one synthetic, very long session JSONL (default 5,000 turns ≈ 200 MB) in the
session v3 layout the owner's real chats use: per turn a user message, an assistant
thinking+toolCall, a large toolResult, an assistant text answer, and a compaction entry
every 500 turns. Streams to disk (constant memory).

    gen-long-session.py <agent_dir> <cwd> [--turns 5000] [--result-kib 36]
prints the session file path.
"""
import argparse
import json
import os
import random
import uuid
from datetime import datetime, timedelta, timezone

ap = argparse.ArgumentParser()
ap.add_argument("agent_dir")
ap.add_argument("cwd")
ap.add_argument("--turns", type=int, default=5000)
ap.add_argument("--result-kib", type=int, default=36)
a = ap.parse_args()

rnd = random.Random(7)
WORDS = ("cargo build test session host engine stream render tool result agent graph wasm "
         "memory compaction token prefix cache event timeline sidebar virtual scroll").split()
t0 = datetime(2026, 10, 1, tzinfo=timezone.utc)
sid = str(uuid.UUID(int=rnd.getrandbits(128)))
esc = "--" + a.cwd.strip("/").replace("/", "-") + "--"
d = os.path.join(a.agent_dir, "sessions", esc)
os.makedirs(d, exist_ok=True)
path = os.path.join(d, f"2026-10-01T00-00-00-000Z_{sid}.jsonl")


def words(n):
    return " ".join(rnd.choice(WORDS) for _ in range(n))


def blob(kib):
    line = words(14)
    out, size = [], 0
    while size < kib * 1024:
        s = f"{len(out):05d}  {line}\n"
        out.append(s)
        size += len(s)
    return "".join(out)


usage = {"input": 1200, "output": 300, "cacheRead": 90000, "cacheWrite": 0, "reasoning": 50,
         "totalTokens": 91500, "cost": {"input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0}}
eid = 0
parent = None


def nid():
    global eid
    eid += 1
    return f"{eid:08x}"


with open(path, "w") as f:
    f.write(json.dumps({"type": "session", "version": 3, "id": sid, "timestamp": t0.isoformat(), "cwd": a.cwd}) + "\n")

    def put(entry, ts):
        global parent
        e = {"id": nid(), "parentId": parent, "timestamp": ts.isoformat().replace("+00:00", "Z")}
        e.update(entry)
        if "type" not in e:
            e["type"] = "message"
        f.write(json.dumps(e) + "\n")
        parent = e["id"]
        return e["id"]

    put({"type": "model_change", "provider": "ollama", "modelId": "fake-1"}, t0)
    first_kept = None
    for turn in range(a.turns):
        ts = t0 + timedelta(seconds=turn * 30)
        ms = int(ts.timestamp() * 1000)
        uid = put({"message": {"role": "user", "content": [{"type": "text", "text": f"Turn {turn}: " + words(40)}], "timestamp": ms}}, ts)
        if first_kept is None:
            first_kept = uid
        call = f"call_{turn}"
        put({"message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": words(60), "thinkingSignature": "sig"},
            {"type": "toolCall", "id": call, "name": "bash", "arguments": {"command": f"cat file_{turn}.rs"}}],
            "api": "openai-completions", "provider": "ollama", "model": "fake-1", "usage": usage,
            "stopReason": "toolUse", "timestamp": ms}}, ts)
        put({"message": {"role": "toolResult", "toolCallId": call, "toolName": "bash",
                         "content": [{"type": "text", "text": blob(a.result_kib)}], "isError": False, "timestamp": ms}}, ts)
        put({"message": {"role": "assistant", "content": [{"type": "text", "text": "## Result\n\n" + words(120) + "\n\n```rust\nfn main() {}\n```\n"}],
                         "api": "openai-completions", "provider": "ollama", "model": "fake-1", "usage": usage,
                         "stopReason": "stop", "timestamp": ms}}, ts)
        if turn % 500 == 499:
            put({"type": "compaction", "summary": "Summary: " + words(3000), "firstKeptEntryId": uid,
                 "tokensBefore": 150000, "details": {"compactor": "synthetic", "version": 1}}, ts)
print(path)
