#!/usr/bin/env python3
"""Fake OpenAI-compatible chat-completions server for phone UI QA (no real model, no secrets).

Listens on 127.0.0.1:11434: models.json overrides the keyless `ollama` provider to point here
(a custom provider id is dropped without credentials). Log: /root/qa/fakellm.log.
Prompt keywords pick a script:
  "tools"  -> bash + read tool calls, then a text summary
  "image"  -> read of /root/qa/home/projects/demo-app/shot.png (image tool result), then text
  "ask"    -> ask_user tool call (if the tool is offered), else text
  "error"  -> HTTP 500
  "long"   -> very long markdown answer (scrolling)
  "edit"   -> edit/write tool calls (diff rows)
  anything else -> medium markdown answer with code block + table, streamed slowly
"""
import json, sys, time, re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = open("/root/qa/fakellm.log", "a")
DEMO = "/root/qa/home/projects/demo-app"

MD = """Here is a **quick overview** of the project.

## Structure
- `src/main.rs` — entry point, parses CLI flags
- `src/server.rs` — HTTP server with *graceful shutdown*
- `tests/` — integration tests

```rust
fn main() {
    let cfg = Config::from_env().expect("config");
    server::run(cfg).unwrap();
}
```

| File | Lines | Status |
|------|------:|--------|
| main.rs | 42 | ok |
| server.rs | 318 | needs review |
| a_really_long_file_name_that_should_truncate_somewhere.rs | 9001 | generated |

> Note: the server binds `0.0.0.0:8080` by default; consider `127.0.0.1` for dev.

Next I can run the tests or open `server.rs` — say **tools** to see tool rows.
"""

def chunks(text, n=12):
    for i in range(0, len(text), n):
        yield text[i:i + n]

def sse(handler, obj):
    handler.wfile.write(b"data: " + json.dumps(obj).encode() + b"\n\n")
    handler.wfile.flush()

def delta(content=None, tool_calls=None, finish=None, model="fake-1"):
    d = {}
    if content is not None:
        d["content"] = content
    if tool_calls is not None:
        d["tool_calls"] = tool_calls
    return {"id": "c1", "object": "chat.completion.chunk", "model": model,
            "choices": [{"index": 0, "delta": d, "finish_reason": finish}]}

def text_of(msg):
    c = msg.get("content")
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return " ".join(p.get("text", "") for p in c if isinstance(p, dict))
    return ""

class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        body = json.dumps({"data": [{"id": "fake-1", "object": "model"}]}).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        req = json.loads(self.rfile.read(n) or b"{}")
        msgs = req.get("messages", [])
        tools = [t.get("function", {}).get("name") for t in req.get("tools", []) or []]
        last_user = next((text_of(m) for m in reversed(msgs) if m.get("role") == "user"), "")
        after_tool = bool(msgs) and msgs[-1].get("role") == "tool"
        LOG.write(json.dumps({"t": time.time(), "last_user": last_user[:200], "after_tool": after_tool, "tools": tools}) + "\n"); LOG.flush()
        low = last_user.lower()
        if "error" in low and not after_tool:
            body = b'{"error":{"message":"fake upstream overloaded (QA error case)","type":"server_error"}}'
            self.send_response(500); self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body); return
        self.send_response(200)
        self.send_header("content-type", "text/event-stream"); self.send_header("cache-control", "no-cache"); self.end_headers()
        calls = []
        text = MD
        slow = 0.05
        if after_tool:
            text = "Done. The tool calls above finished; `cargo test` reported **12 passed, 0 failed**. Anything else?"
        elif "tools" in low:
            calls = [("bash", {"command": f"cd {DEMO} && ls -la && cat Cargo.toml"}),
                     ("read", {"path": f"{DEMO}/src/main.rs"}),
                     ("bash", {"command": "for i in 1 2 3; do echo step $i; sleep 1; done"})]
            text = "Let me look around the project first."
        elif "image" in low:
            calls = [("read", {"path": f"{DEMO}/shot.png"})]
            text = "Reading the screenshot."
        elif "edit" in low:
            calls = [("edit", {"path": f"{DEMO}/src/main.rs", "edits": [{"oldText": "Hello", "newText": "Hello, phone"}]}),
                     ("write", {"path": f"{DEMO}/NOTES.md", "content": "# Notes\n\n- added by QA\n"})]
            text = "I'll make the change."
        elif "ask" in low and "ask_user" in tools:
            calls = [("ask_user", {"question": "Which database should I use?", "options": ["SQLite", "Postgres", "Keep files"]})]
            text = "One question first."
        elif "long" in low:
            text = "\n\n".join(f"### Section {i}\n" + ("Lorem ipsum dolor sit amet, consectetur adipiscing elit. " * 6) + f"\n\n```sh\ncargo run --example s{i}\n```" for i in range(1, 25))
            slow = 0.005
        for c in chunks(text):
            sse(self, delta(content=c)); time.sleep(slow)
        if calls:
            for i, (name, args) in enumerate(calls):
                sse(self, delta(tool_calls=[{"index": i, "id": f"call_{int(time.time()*1000)}_{i}", "type": "function",
                                             "function": {"name": name, "arguments": json.dumps(args)}}]))
            sse(self, delta(finish="tool_calls"))
        else:
            sse(self, delta(finish="stop"))
        sse(self, {"id": "c1", "object": "chat.completion.chunk", "choices": [], "usage": {"prompt_tokens": 1200, "completion_tokens": 300, "total_tokens": 1500}})
        self.wfile.write(b"data: [DONE]\n\n"); self.wfile.flush()

ThreadingHTTPServer(("127.0.0.1", 11434), H).serve_forever()
