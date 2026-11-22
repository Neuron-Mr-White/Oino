"""Synthetic OpenAI chat-completions SSE helpers (no model or credentials)."""
import json

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
