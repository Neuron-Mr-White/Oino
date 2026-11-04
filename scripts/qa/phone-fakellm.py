#!/usr/bin/env python3
"""Fake OpenAI-compatible chat-completions server for phone UI QA (no real model, no secrets).

Listens on 127.0.0.1:11434: models.json overrides the keyless `ollama` provider to point here
(a custom provider id is dropped without credentials). Log: /root/qa/fakellm.log.
Prompt keywords pick a script:
  "tools"  -> bash + read tool calls, then a text summary
  "image"  -> read of /root/qa/home/projects/demo-app/shot.png (image tool result), then text
  "ask"    -> ask_user tool call (if the tool is offered), else text; "ask two" asks 2 questions
  "error"  -> HTTP 500
  "quiz"   -> a text reply that asks two questions (no tool)
  "long"   -> very long markdown answer (scrolling)
  "marathon" -> one text stream of ~330 s (FAKELLM_MARATHON_S), a chunk every 5 s (stream-timeout QA)
  "edit"   -> edit/write tool calls (diff rows)
  "grow"   -> one bash call whose output streams for ~12 s (chat-scroll: output inside a tool row)
  "graphwork" -> update_agent_graph add_work (2 slow scans, 1 quick probe, 1 erroring probe, 1 dependent summary)
  "graphstop <id>" -> update_agent_graph stop <id> (orphaned-worker check: the stopped worker stays visible until it ends)
  "subagents" -> run_subagent x4 in the background (2 slow, 1 quick, 1 failing) + bg_run x1
                 (running-tab QA); the children's tasks use the "slow"/"quick"/"error" scripts
  "quietbg" -> bg_run without notify/wake (the quiet background marker, [status])
  "slow"   -> one bash call that sleeps 15 min (a run that stays live)
  "quick"  -> one bash call, then text (a run that completes)
  anything else -> medium markdown answer with code block + table, streamed slowly
"""
import json, os, sys, time, re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = None  # opened in main(); importable (scripts/perf/fakellm.py reuses the SSE helpers)
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
        # The user turn = every user message after the last assistant one (extensions inject
        # their own user-role notes, e.g. a memory nudge, right after the typed prompt).
        tail = []
        for m in reversed(msgs):
            if m.get("role") == "assistant":
                break
            if m.get("role") == "user":
                tail.append(text_of(m))
        last_user = " ".join(reversed(tail))
        after_tool = bool(msgs) and msgs[-1].get("role") == "tool"
        LOG.write(json.dumps({"t": time.time(), "last_user": last_user[:200], "after_tool": after_tool, "tools": tools}) + "\n"); LOG.flush()
        low = last_user.split("## \U0001f9e0 Memory System Active")[0].lower()  # the memory nudge quotes "tools"/"error"
        if "error" in low and not after_tool and "checkpoint" not in low:  # a graph checkpoint note quotes child errors
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
        elif "checkpoint" in low and "yield_agent_graph" in tools:
            # a graph checkpoint: let the scheduled work continue
            calls = [("yield_agent_graph", {"reason": "waiting on the scans"})]
            text = "Waiting on the running work."
        elif "graphstop" in low and "update_agent_graph" in tools:
            # [owner-fix-3] stop running graph work: "graphstop <work id>" (the orphaned-worker check)
            m = re.search(r"graphstop\s+(\S+)", low)
            calls = [("update_agent_graph", {"operation": "stop", "stop": [{"target_id": m.group(1) if m else "", "reason": "obsolete (QA stop)"}]})]
            text = "Stopping that work."
        elif "graphwork" in low and "update_agent_graph" in tools:
            # [owner-fix-3] a long-horizon graph whose workers stay running (Working row, Graph/Running tabs)
            sfx = f"-{int(time.time()) % 100000}"  # graphs are per project: fresh ids each run
            def w(wid, ins, deps=()):
                return {"work_id": wid + sfx, "instruction": ins, "profile": "subagent_general", "input_ids": [d + sfx for d in deps]}
            calls = [("update_agent_graph", {"operation": "add_work", "task": "Map the sidebar data path",
                       "add_work": [w("scan-host", "slow: scan the host session list"),
                                    w("scan-app", "slow: scan the app sidebar store"),
                                    w("probe-cache", "quick: probe the list cache"),
                                    w("probe-flaky", "error: probe the flaky endpoint"),
                                    w("synth", "quick: write the summary", ("r-scan-host", "r-scan-app"))]})]
            text = "Planning the graph: two scans, a probe, then a summary."
        elif "tools" in low:
            calls = [("bash", {"command": f"cd {DEMO} && ls -la && cat Cargo.toml"}),
                     ("read", {"path": f"{DEMO}/src/main.rs"}),
                     ("bash", {"command": "for i in 1 2 3; do echo step $i; sleep 1; done"})]
            text = "Let me look around the project first."
        elif "subagents" in low and "run_subagent" in tools:
            def sub(title, task, profile="subagent_explore"):
                return ("run_subagent", {"title": title, "task": task, "profile": profile, "is_background": True})
            calls = [sub("Map the status topics", "slow: map the status topics of the core extension"),
                     sub("Audit the settings schema", "slow: audit the settings schema", "subagent_general"),
                     sub("List the tray producers", "quick: list the work-tray producers"),
                     sub("Probe the flaky endpoint", "error: probe the flaky endpoint")]
            if "bg_run" in tools:
                calls.append(("bg_run", {"name": "Watch the test suite", "command": "for i in $(seq 1 900); do echo tick $i; sleep 1; done", "isAgent": False}))
            text = "Starting four subagents and a background task."
        elif "quietbg" in low and "bg_run" in tools:
            # [status] a bg task WITHOUT wake: the session reads the quiet background marker, not Working
            calls = [("bg_run", {"name": "Dev server", "command": "for i in $(seq 1 900); do echo serve $i; sleep 1; done", "isAgent": False,
                                 "notifyOnCompletion": False, "triggerOnCompletion": False})]
            text = "Starting the dev server in the background."
        elif "slow" in low:
            calls = [("bash", {"command": "sleep 900"})]
            text = "Working on it."
        elif "quick" in low:
            calls = [("bash", {"command": "echo producers: bg subagents dream workflow"})]
            text = "Checking."
        elif "grow" in low:
            # chat-scroll QA: one bash call whose output streams for ~12 s (grows inside an expanded tool row)
            calls = [("bash", {"command": "for i in $(seq 1 80); do echo \"output line $i\"; sleep 0.15; done"})]
            text = "Running a long command."
        elif "image" in low:
            calls = [("read", {"path": f"{DEMO}/shot.png"})]
            text = "Reading the screenshot."
        elif "edit" in low:
            calls = [("edit", {"path": f"{DEMO}/src/main.rs", "edits": [{"oldText": "Hello", "newText": "Hello, phone"}]}),
                     ("write", {"path": f"{DEMO}/NOTES.md", "content": "# Notes\n\n- added by QA\n"})]
            text = "I'll make the change."
        elif re.search(r"\bask\b", low) and "ask_user" in tools:  # a word: the memory nudge says "task"
            # The tool's own shape ({questions: [...]}; [ask-user]): "ask two" asks two at once.
            db = {"question": "Which database should I use for the cache?", "header": "Database",
                  "options": [{"label": "SQLite", "description": "One file, no server"},
                              {"label": "Postgres", "description": "Shared, needs a server"},
                              {"label": "Keep files", "description": "What we have now"}]}
            extras = {"question": "Which extras should the first version have?", "header": "Extras", "multi_select": True,
                      "options": [{"label": "Auth"}, {"label": "Rate limits"}, {"label": "Metrics"}]}
            calls = [("ask_user", {"questions": [db, extras] if "two" in low else [db]})]
            text = "Before I wire the cache I need your call on the storage."
        elif "quiz" in low:
            # A plain reply that asks (the utility /answer hint's trigger; [ask-user] QA).
            text = "I can do that. Which storage should the cache use?\n\n- SQLite\n- Postgres\n\nAnd should it expire entries after a day?"
        elif "marathon" in low:
            # [owner-fix-2] stream-timeout QA: ONE provider stream longer than the old 300 s client
            # read timeout (a chunk every 5 s, never idle long enough for httpIdleTimeoutMs).
            secs = int(os.environ.get("FAKELLM_MARATHON_S", "330"))
            for i in range(secs // 5):
                sse(self, delta(content=f"tick {i * 5:>3} s. ")); time.sleep(5)
            text = "Marathon stream finished."
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

if __name__ == "__main__":
    LOG = open(os.environ.get("FAKELLM_LOG", "/root/qa/fakellm.log"), "a")
    ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("FAKELLM_PORT", "11434"))), H).serve_forever()
