"""Minimal MCP-over-HTTP (JSON-RPC) client for the srv daemon."""
from __future__ import annotations

import json
import urllib.request

MCP_URL = "http://127.0.0.1:7070/mcp"


class McpError(Exception):
    pass


class McpClient:
    def __init__(self, url: str = MCP_URL):
        self.url = url
        self._id = 0

    def _call(self, method: str, params: dict):
        self._id += 1
        body = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": self._id,
                "method": method,
                "params": params,
            }
        ).encode()
        req = urllib.request.Request(
            self.url,
            data=body,
            headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
            method="POST",
        )
        with urllib.request.urlopen(req) as resp:
            payload = json.loads(resp.read().decode())
        if "error" in payload:
            raise McpError(payload["error"].get("message", str(payload["error"])))
        return payload.get("result", {})

    def tools_list(self) -> list[dict]:
        res = self._call("tools/list", {})
        return res.get("tools", [])

    def tool_call(self, name: str, arguments: dict) -> dict:
        res = self._call("tools/call", {"name": name, "arguments": arguments})
        # Tool results come back as content[] with text JSON.
        texts = [c.get("text", "") for c in res.get("content", []) if c.get("type") == "text"]
        for t in texts:
            try:
                return json.loads(t).get("result", json.loads(t))
            except json.JSONDecodeError:
                continue
        return res


def parse_result_text(text: str) -> dict:
    """The daemon wraps tool output as text containing {"result":{...},"status":"ok"}."""
    data = json.loads(text)
    return data.get("result", data)
