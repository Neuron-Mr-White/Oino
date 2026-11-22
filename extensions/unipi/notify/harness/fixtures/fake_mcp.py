#!/usr/bin/env python3
"""Minimal stdio MCP server for the unipi.mcp harness: echo + add tools, one server request."""
import json
import sys

TOOLS = [
    {"name": "echo", "description": "Echo text back", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}, "annotations": {"readOnlyHint": True}},
    {"name": "add", "inputSchema": {"type": "object", "properties": {"b": {"type": "number"}, "a": {"type": "number"}}, "required": ["b", "a", "a"]}},
    {"name": "fail", "description": "Always fails", "inputSchema": {"type": "object", "properties": {}}},
]


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        sys.stdout.write("not json noise\n")
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": msg["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}, "instructions": "Fake server."}})
    elif method == "tools/list":
        # A server→client request before the answer exercises the reply path.
        send({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"})
        send({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "hi"}})
        send({"jsonrpc": "2.0", "id": mid, "result": {"tools": TOOLS}})
    elif method == "tools/call":
        name = msg["params"]["name"]
        args = msg["params"].get("arguments", {})
        if name == "echo":
            send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": "echo: " + args.get("text", "")}]}})
        elif name == "add":
            send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": str(args.get("a", 0) + args.get("b", 0))}], "structuredContent": {"sum": args.get("a", 0) + args.get("b", 0)}}})
        else:
            send({"jsonrpc": "2.0", "id": mid, "result": {"isError": True, "content": [{"type": "text", "text": "it broke"}]}})
    elif mid is not None and method is None:
        pass  # our ping reply
    elif mid is not None:
        send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "unknown"}})
