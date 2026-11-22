#!/usr/bin/env python3
"""Tiny MCP stdio server used by examples/mcp.json."""
import json
import sys


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    try:
        request = json.loads(line)
    except Exception:
        continue
    method = request.get("method")
    request_id = request.get("id")
    params = request.get("params") or {}

    if method == "initialize":
        send({"jsonrpc": "2.0", "id": request_id, "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "echo", "version": "1.0"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": [{
            "name": "echo",
            "description": "Echo the provided text back.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string", "description": "Text to echo"}},
                "required": ["text"],
            },
        }]}})
    elif method == "tools/call":
        text = str((params.get("arguments") or {}).get("text", ""))
        send({"jsonrpc": "2.0", "id": request_id, "result": {
            "content": [{"type": "text", "text": text}],
        }})
    elif request_id is not None:
        send({"jsonrpc": "2.0", "id": request_id, "error": {
            "code": -32601, "message": "method not found",
        }})
