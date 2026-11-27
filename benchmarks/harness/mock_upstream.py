"""A fake Anthropic Messages endpoint for dry runs.

It answers every ``/v1/messages`` call with a one-line text reply (streamed or
not, as asked), so Claude Code ends after one turn and nothing is billed. Each
request is summarised so the dry run can show which arm sent what: whether the
terse-mode instruction was injected, whether the ``retoken_retrieve`` tool was
offered, and how many prompt-cache breakpoints the request carried.

Request headers are never recorded.
"""

from __future__ import annotations

import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, List, Optional

REPLY_TEXT = "Dry run: this reply came from the benchmark's mock upstream, not a model."
TERSE_MARKER = "[ReToken terse mode]"


def _text_of(value: Any) -> str:
    """Flatten Anthropic content (str or list of blocks) into plain text."""
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "\n".join(_text_of(item) for item in value)
    if isinstance(value, dict):
        parts = [_text_of(value.get(key)) for key in ("text", "content") if key in value]
        return "\n".join(parts)
    return ""


def _count_cache_control(value: Any) -> int:
    if isinstance(value, dict):
        own = 1 if "cache_control" in value else 0
        return own + sum(_count_cache_control(v) for v in value.values())
    if isinstance(value, list):
        return sum(_count_cache_control(v) for v in value)
    return 0


def summarize_request(path: str, body: Dict[str, Any]) -> Dict[str, Any]:
    tools = [t.get("name", "") for t in body.get("tools", []) if isinstance(t, dict)]
    system = _text_of(body.get("system", ""))
    messages = body.get("messages", [])
    everything = system + "\n" + _text_of(messages)
    return {
        "path": path,
        "model": body.get("model"),
        "stream": bool(body.get("stream")),
        "messages": len(messages) if isinstance(messages, list) else 0,
        "tools": len(tools),
        "retoken_retrieve_tool": any(n.endswith("retoken_retrieve") for n in tools)
        or "retoken_retrieve" in everything,
        "terse_instruction": TERSE_MARKER in everything,
        "cache_breakpoints": _count_cache_control(body),
        "request_bytes": len(json.dumps(body)),
    }


class MockUpstream:
    """Run ``with MockUpstream() as mock:`` and point a client at ``mock.url``."""

    def __init__(self) -> None:
        self.requests: List[Dict[str, Any]] = []
        self._lock = threading.Lock()
        self._server: Optional[ThreadingHTTPServer] = None
        self._thread: Optional[threading.Thread] = None

    @property
    def url(self) -> str:
        assert self._server is not None, "mock upstream is not running"
        return f"http://127.0.0.1:{self._server.server_address[1]}"

    def drain(self) -> List[Dict[str, Any]]:
        """Return and forget the requests recorded since the last drain."""
        with self._lock:
            taken, self.requests = self.requests, []
        return taken

    def _record(self, entry: Dict[str, Any]) -> None:
        with self._lock:
            self.requests.append(entry)

    def __enter__(self) -> "MockUpstream":
        mock = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args: Any) -> None:  # keep the console quiet
                pass

            def _json(self, status: int, payload: Dict[str, Any]) -> None:
                data = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self) -> None:
                mock._record({"path": self.path.split("?")[0], "method": "GET"})
                self._json(404, {"type": "error", "error": {"type": "not_found_error",
                                                            "message": "mock upstream"}})

            def do_POST(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(length) if length else b""
                path = self.path.split("?")[0]
                try:
                    body = json.loads(raw or b"{}")
                except ValueError:
                    body = {}
                if path.endswith("/messages/count_tokens"):
                    mock._record({"path": path, "method": "POST"})
                    self._json(200, {"input_tokens": max(1, len(raw) // 4)})
                    return
                if not path.endswith("/messages"):
                    mock._record({"path": path, "method": "POST"})
                    self._json(404, {"type": "error", "error": {"type": "not_found_error",
                                                                "message": "mock upstream"}})
                    return
                mock._record(summarize_request(path, body))
                self._reply(body, input_tokens=max(1, len(raw) // 4))

            def _reply(self, body: Dict[str, Any], input_tokens: int) -> None:
                model = body.get("model", "mock-model")
                usage = {"input_tokens": input_tokens, "output_tokens": 16,
                         "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}
                message = {"id": "msg_dry_run", "type": "message", "role": "assistant",
                           "model": model, "stop_reason": "end_turn", "stop_sequence": None,
                           "content": [{"type": "text", "text": REPLY_TEXT}], "usage": usage}
                if not body.get("stream"):
                    self._json(200, message)
                    return
                start = dict(message, content=[], stop_reason=None,
                             usage=dict(usage, output_tokens=1))
                events = [
                    ("message_start", {"type": "message_start", "message": start}),
                    ("content_block_start", {"type": "content_block_start", "index": 0,
                                             "content_block": {"type": "text", "text": ""}}),
                    ("content_block_delta", {"type": "content_block_delta", "index": 0,
                                             "delta": {"type": "text_delta", "text": REPLY_TEXT}}),
                    ("content_block_stop", {"type": "content_block_stop", "index": 0}),
                    ("message_delta", {"type": "message_delta",
                                       "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                                       "usage": {"output_tokens": 16}}),
                    ("message_stop", {"type": "message_stop"}),
                ]
                payload = "".join(
                    f"event: {name}\ndata: {json.dumps(data)}\n\n" for name, data in events
                ).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *exc: Any) -> None:
        if self._server is not None:
            self._server.shutdown()
            self._server.server_close()
            self._server = None
