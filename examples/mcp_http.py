#!/usr/bin/env python3
"""Tiny Streamable-HTTP MCP server used by examples/mcp-http.json."""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        try:
            request = json.loads(self.rfile.read(length) or b"{}")
        except Exception:
            request = {}
        method = request.get("method")
        request_id = request.get("id")
        params = request.get("params") or {}

        if method == "notifications/initialized":
            self.send_response(202)
            self.end_headers()
            return

        if method == "initialize":
            result = {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "http-echo", "version": "1.0"},
            }
        elif method == "tools/list":
            result = {"tools": [{
                "name": "echo",
                "description": "Echo the provided text back over HTTP.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"text": {"type": "string", "description": "Text to echo"}},
                    "required": ["text"],
                },
            }]}
        elif method == "tools/call":
            text = str((params.get("arguments") or {}).get("text", ""))
            result = {"content": [{"type": "text", "text": text}]}
        else:
            result = None

        payload = {"jsonrpc": "2.0", "id": request_id}
        if result is None:
            payload["error"] = {"code": -32601, "message": "method not found"}
        else:
            payload["result"] = result
        data = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 8799), Handler).serve_forever()
