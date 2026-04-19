#!/usr/bin/env python3
"""
TensorZero Proxy - DEBUG VERSION
Production-ready version for LAN deployment with enhanced Codex debugging.

Features:
- Detects /me commands and routes to slash_me function
- Handles both literal "/me" and IRC ACTION format
- Strips WeeChat metadata preamble from all messages (saves context window)
- Removes tool definitions for roleplay models (they don't support tools)
- Normal messages route to openclaw function
- Translates OpenAI Responses API (/responses) → chat/completions for Codex CLI
  - Full tool call translation in both directions (shell function calls)
  - Handles function_call / function_call_output input items
  - Streaming: translates delta.tool_calls → Responses API function_call events
  - ENHANCED DEBUG LOGGING for tool call troubleshooting
- Passes through embedding requests unchanged
- Binds to all interfaces (0.0.0.0) for LAN access
- Streams responses to avoid buffering large payloads
- Graceful BrokenPipeError handling on client disconnect
- 5-minute timeout for slow model fallback chains

Usage:
    python3 tensorzero-proxy.py --port 3001 --tensorzero http://192.168.1.XXX:3000

codex.toml:
    [model_providers.tensorzero]
    base_url = "http://127.0.0.1:3001/openai/v1"
    wire_api = "responses"
"""

import re
import json
import time
import uuid
import socketserver
import http.server
import urllib.request
import urllib.error
import argparse
import sys
import os
import traceback

# Default configuration
TENSORZERO_URL = "http://192.168.1.157:3000"
EMBEDDINGS_URL = "http://192.168.1.213:5556"
PROXY_PORT = 3001

# Load .env file from same directory as script (if it exists)
_env_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), '.env')
if os.path.exists(_env_path):
    with open(_env_path) as _f:
        for _line in _f:
            _line = _line.strip()
            if _line and not _line.startswith('#') and '=' in _line:
                _key, _val = _line.split('=', 1)
                os.environ.setdefault(_key.strip(), _val.strip().strip('"').strip("'"))

# CTCP ACTION character (ASCII 0x01)
CTCP_CHAR = '\x01'

# Pattern to match WeeChat metadata preamble blocks
METADATA_PATTERN = re.compile(
    r'(?:Conversation info|Sender)\s*\(untrusted metadata\):\s*```json\s*\{[^}]*\}\s*```\s*',
    re.DOTALL
)


# ── OpenClaw helpers ───────────────────────────────────────────────────────────

def strip_metadata(msg: str) -> str:
    if not isinstance(msg, str):
        return msg
    stripped = METADATA_PATTERN.sub('', msg).strip()
    return stripped if stripped else msg


def strip_metadata_from_content(content):
    if isinstance(content, str):
        return strip_metadata(content)
    elif isinstance(content, list):
        for i, part in enumerate(content):
            if isinstance(part, dict) and part.get('type') == 'text':
                part['text'] = strip_metadata(part.get('text', ''))
            elif isinstance(part, str):
                content[i] = strip_metadata(part)
        return content
    return content


def sanitize_roles_for_roleplay(messages: list) -> tuple[list, list]:
    VALID_ROLES = {"user", "system", "assistant"}
    ROLE_MAP = {"developer": "system"}
    DROP_ROLES = {"tool", "function"}
    sanitized = []
    skipped_roles = []
    for msg in messages:
        role = msg.get("role", "")
        if role in DROP_ROLES:
            continue
        mapped_role = ROLE_MAP.get(role, role)
        if mapped_role not in VALID_ROLES:
            skipped_roles.append(role or "<missing>")
            continue
        cleaned = msg.copy()
        cleaned["role"] = mapped_role
        if cleaned.get("tool_calls"):
            cleaned.pop("tool_calls", None)
        if "tool_call_id" in cleaned:
            cleaned.pop("tool_call_id", None)
        sanitized.append(cleaned)
    return sanitized, skipped_roles


def route_function(message: str) -> str:
    if re.search(r'(?:^|\s)/me\b', message):
        return "slash_me"
    if f'{CTCP_CHAR}ACTION ' in message:
        return "slash_me"
    return "openclaw"


def clean_message(msg: str) -> str:
    match = re.search(rf'{CTCP_CHAR}ACTION\s+(.*?){CTCP_CHAR}', msg)
    if match:
        return match.group(1).strip()
    cleaned = re.sub(r'(?:^|\n)\s*/me\s+', '', msg).strip()
    if cleaned != msg.strip():
        return cleaned
    return msg.strip()


def _safe_write(wfile, data: bytes, log_func=None) -> bool:
    try:
        wfile.write(data)
        return True
    except BrokenPipeError:
        if log_func:
            log_func("⚠️ Client disconnected during write")
        return False


# ── Codex / Responses API helpers ─────────────────────────────────────────────

def _sse(event: dict) -> bytes:
    """Encode a dict as a single SSE data frame."""
    return f"data: {json.dumps(event)}\n\n".encode("utf-8")


def _make_id(prefix: str) -> str:
    return f"{prefix}_{uuid.uuid4().hex[:24]}"


# Responses API built-in tool types with no chat completions equivalent
_RESPONSES_BUILTIN_TOOLS = {"web_search", "code_interpreter", "file_search", "computer"}


def _convert_tools_to_chat(tools: list) -> list:
    """
    Responses API tools:     {type, name, description, parameters}
    Chat Completions tools:  {type, function: {name, description, parameters}}

    Built-in Responses API tools (web_search, code_interpreter, etc.) have no
    chat completions equivalent and are silently dropped — TensorZero rejects them.
    """
    chat_tools = []
    for tool in tools:
        t = tool.get("type")
        if t == "function":
            chat_tools.append({
                "type": "function",
                "function": {
                    "name":        tool.get("name", ""),
                    "description": tool.get("description", ""),
                    "parameters":  tool.get("parameters", {}),
                }
            })
        elif t in _RESPONSES_BUILTIN_TOOLS:
            # No chat completions equivalent — drop silently
            continue
        else:
            chat_tools.append(tool)
    return chat_tools


def _responses_input_to_messages(data: dict) -> list:
    """
    Convert Responses API { instructions, input[] } → Chat Completions messages[].

    Handled input item types:
      role=user/system          → regular message
      role=assistant            → assistant text message
      type=function_call        → assistant message with tool_calls[]
      type=function_call_output → tool role message (tool result)

    instructions → prepended system message
    """
    messages = []

    instructions = data.get("instructions")
    if instructions:
        messages.append({"role": "system", "content": instructions})

    inp = data.get("input", [])

    if isinstance(inp, str):
        messages.append({"role": "user", "content": inp})
        return messages

    if not isinstance(inp, list):
        return messages

    # Accumulate consecutive function_call items into one assistant tool_calls message
    pending_tool_calls = []

    def flush_tool_calls():
        if pending_tool_calls:
            messages.append({
                "role":       "assistant",
                "content":    None,
                "tool_calls": list(pending_tool_calls),
            })
            pending_tool_calls.clear()

    for item in inp:
        if not isinstance(item, dict):
            continue

        item_type = item.get("type")
        role      = item.get("role")

        if item_type == "function_call":
            # Assistant issued a tool call — accumulate
            pending_tool_calls.append({
                "id":   item.get("call_id", item.get("id", _make_id("call"))),
                "type": "function",
                "function": {
                    "name":      item.get("name", ""),
                    "arguments": item.get("arguments", ""),
                }
            })

        elif item_type == "function_call_output":
            # Tool result — flush pending tool_calls as assistant message first
            flush_tool_calls()
            output = item.get("output", "")
            if not isinstance(output, str):
                output = json.dumps(output)
            messages.append({
                "role":         "tool",
                "tool_call_id": item.get("call_id", ""),
                "content":      output,
            })

        elif role == "assistant":
            flush_tool_calls()
            content = item.get("content", "")
            if isinstance(content, list):
                parts = []
                for part in content:
                    if isinstance(part, dict):
                        parts.append(part.get("text", ""))
                    elif isinstance(part, str):
                        parts.append(part)
                content = "\n".join(p for p in parts if p)
            messages.append({"role": "assistant", "content": content})

        else:
            # user / system or unrecognised
            flush_tool_calls()
            r       = role or "user"
            content = item.get("content", "")
            if isinstance(content, list):
                parts = []
                for part in content:
                    if isinstance(part, dict):
                        parts.append(part.get("text", ""))
                    elif isinstance(part, str):
                        parts.append(part)
                content = "\n".join(p for p in parts if p)
            messages.append({"role": r, "content": content})

    flush_tool_calls()
    return messages


def _build_codex_chat_request(data: dict) -> dict:
    """Build a Chat Completions request body from a Responses API request."""
    messages = _responses_input_to_messages(data)
    req: dict = {
        "model":    "tensorzero::function_name::codex",
        "messages": messages,
        "stream":   data.get("stream", False),
    }
    if "max_output_tokens" in data:
        req["max_tokens"] = data["max_output_tokens"]
    for key in ("temperature", "top_p", "stop"):
        if key in data:
            req[key] = data[key]
    if "tool_choice" in data:
        req["tool_choice"] = data["tool_choice"]
    if "parallel_tool_calls" in data:
        req["parallel_tool_calls"] = data["parallel_tool_calls"]
    if data.get("tools"):
        req["tools"] = _convert_tools_to_chat(data["tools"])
    return req


def _chat_completion_to_response(completion: dict, resp_id: str, msg_id: str) -> dict:
    """
    Convert a full Chat Completions response → Responses API response object.
    Handles both text replies and tool_calls.
    """
    choice        = (completion.get("choices") or [{}])[0]
    message       = choice.get("message") or {}
    finish_reason = choice.get("finish_reason", "stop")
    usage         = completion.get("usage") or {}
    output        = []

    if finish_reason == "tool_calls" and message.get("tool_calls"):
        for tc in message["tool_calls"]:
            fn = tc.get("function") or {}
            output.append({
                "type":      "function_call",
                "id":        _make_id("fc"),
                "call_id":   tc.get("id", ""),
                "name":      fn.get("name", ""),
                "arguments": fn.get("arguments", ""),
                "status":    "completed",
            })
    else:
        text = message.get("content") or ""
        output.append({
            "id":      msg_id,
            "type":    "message",
            "role":    "assistant",
            "content": [{"type": "output_text", "text": text}],
            "status":  "completed",
        })

    return {
        "id":         resp_id,
        "object":     "response",
        "created_at": completion.get("created", int(time.time())),
        "status":     "completed",
        "model":      completion.get("model", ""),
        "output":     output,
        "usage": {
            "input_tokens":  usage.get("prompt_tokens", 0),
            "output_tokens": usage.get("completion_tokens", 0),
            "total_tokens":  usage.get("total_tokens", 0),
        },
        "error": None,
    }


# ── HTTP handler ───────────────────────────────────────────────────────────────

class ProxyHandler(http.server.BaseHTTPRequestHandler):

    def log_message(self, format, *args):
        from datetime import datetime
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        msg = format % args if args else format
        print(f"[{timestamp}] {self.address_string()} - {msg}")

    def do_POST(self):
        # Codex CLI: Responses API → codex function
        if '/responses' in self.path:
            self.handle_codex_responses()

        # OpenClaw: chat completions with /me routing
        elif '/chat/completions' in self.path:
            self.handle_chat_completions()

        # Embeddings → KoboldCpp
        elif '/embeddings' in self.path:
            self.handle_embeddings()

        # Everything else → TensorZero passthrough
        elif '/completions' in self.path or '/models' in self.path:
            self.handle_passthrough()

        else:
            self.send_response(404)
            self.end_headers()
            _safe_write(self.wfile, b'{"error": "Not found"}', self.log_message)

    def do_GET(self):
        if '/models' in self.path:
            self.handle_passthrough()
        else:
            self.send_response(404)
            self.end_headers()
            _safe_write(self.wfile, b'{"error": "Not found"}', self.log_message)

    def _read_body(self):
        content_length = int(self.headers.get('Content-Length', 0))
        if content_length == 0:
            return b''
        chunks = []
        remaining = content_length
        while remaining > 0:
            chunk = self.rfile.read(min(remaining, 65536))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        return b''.join(chunks)

    # ── Codex: /responses ──────────────────────────────────────────────────────

    def handle_codex_responses(self):
        try:
            body      = self._read_body()
            data      = json.loads(body.decode("utf-8"))
            resp_id   = _make_id("resp")
            msg_id    = _make_id("msg")
            streaming = data.get("stream", False)

            chat_req  = _build_codex_chat_request(data)
            has_tools = bool(chat_req.get("tools"))

            # DEBUG: Log full request details
            self.log_message(f"🖥️  Codex /responses → codex function "
                            f"{'[stream]' if streaming else '[sync]'} "
                            f"msgs={len(chat_req['messages'])} tools={'yes' if has_tools else 'no'}")

            if has_tools:
                self.log_message(f"🔧 Tools in request: {json.dumps(chat_req['tools'], indent=2)}")

            self.log_message(f"📝 Messages: {json.dumps(chat_req['messages'][:2], indent=2)}...")  # First 2 msgs

            upstream = urllib.request.Request(
                TENSORZERO_URL + "/openai/v1/chat/completions",
                data=json.dumps(chat_req).encode("utf-8"),
                headers={
                    "Content-Type":  "application/json",
                    "Authorization": "Bearer tensorzero-proxy",
                },
                method="POST"
            )

            if streaming:
                self._codex_stream(upstream, resp_id, msg_id)
            else:
                self._codex_sync(upstream, resp_id, msg_id)

        except json.JSONDecodeError as e:
            self.log_message(f"❌ Bad JSON in /responses: {e}")
            self.send_response(400)
            self.end_headers()
            _safe_write(self.wfile,
                        json.dumps({"error": f"Bad JSON: {e}"}).encode(),
                        self.log_message)
        except Exception as e:
            self.log_message(f"❌ /responses error: {e}")
            traceback.print_exc()
            self.send_response(500)
            self.end_headers()
            _safe_write(self.wfile,
                        json.dumps({"error": str(e)}).encode(),
                        self.log_message)

    def _codex_sync(self, upstream_req, resp_id: str, msg_id: str):
        try:
            with urllib.request.urlopen(upstream_req, timeout=300) as resp:
                raw = resp.read()
            completion   = json.loads(raw.decode("utf-8"))
            response_obj = _chat_completion_to_response(completion, resp_id, msg_id)
            out          = json.dumps(response_obj).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(out)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            _safe_write(self.wfile, out, self.log_message)

            # DEBUG: Log response details
            choice = (completion.get("choices") or [{}])[0]
            message = choice.get("message") or {}
            finish_reason = choice.get("finish_reason", "stop")
            self.log_message(f"✅ Codex sync done - finish_reason={finish_reason}")

            if message.get("tool_calls"):
                self.log_message(f"🔧 Tool calls in response: {len(message['tool_calls'])}")
                for tc in message["tool_calls"]:
                    fn = tc.get("function") or {}
                    self.log_message(f"   - {fn.get('name', 'unknown')}: {fn.get('arguments', '')[:100]}...")
            else:
                content = message.get("content", "")
                self.log_message(f"   Text response: {content[:100]}...")

        except urllib.error.HTTPError as e:
            body = e.read()
            self.log_message(f"❌ Upstream {e.code}: {body[:200]}")
            self.send_response(e.code)
            self.end_headers()
            _safe_write(self.wfile, body, self.log_message)

    def _codex_stream(self, upstream_req, resp_id: str, msg_id: str):
        """
        Consume chat.completion.chunk SSE from TensorZero.
        Emit Responses API events to Codex — including function_call items.

        Text output events:
          response.created
          response.output_item.added        (message)
          response.content_part.added
          response.output_text.delta ×N
          response.output_text.done
          response.content_part.done
          response.output_item.done         (message)

        Per tool call events:
          response.output_item.added        (function_call)
          response.function_call_arguments.delta ×N
          response.function_call_arguments.done
          response.output_item.done         (function_call)

          response.completed
          [DONE]
        """
        try:
            with urllib.request.urlopen(upstream_req, timeout=300) as resp:

                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Access-Control-Allow-Origin", "*")
                self.end_headers()

                created_at = int(time.time())

                if not _safe_write(self.wfile, _sse({
                    "type": "response.created",
                    "response": {
                        "id": resp_id, "object": "response",
                        "created_at": created_at, "status": "in_progress",
                        "model": "", "output": [], "usage": None, "error": None,
                    }
                }), self.log_message):
                    return

                # ── Streaming state ───────────────────────────────────────────
                next_output_index = 0

                # Text output
                text_started = False
                text_out_idx = None
                full_text    = ""

                # Tool calls keyed by delta index
                # {delta_idx: {fc_id, call_id, name, arguments, output_index}}
                tool_call_state = {}

                final_model = ""
                usage       = {}
                buf         = b""
                done        = False
                chunks_received = 0

                while not done:
                    chunk = resp.read(8192)
                    if not chunk:
                        break
                    buf += chunk
                    chunks_received += 1

                    while b"\n\n" in buf:
                        frame, buf = buf.split(b"\n\n", 1)
                        frame = frame.strip()
                        if not frame:
                            continue
                        if frame.startswith(b"data: "):
                            frame = frame[6:]
                        if frame == b"[DONE]":
                            done = True
                            break
                        try:
                            evt = json.loads(frame.decode("utf-8"))
                        except json.JSONDecodeError:
                            continue

                        if not final_model:
                            final_model = evt.get("model", "")
                        if evt.get("usage"):
                            usage = evt["usage"]

                        choices = evt.get("choices") or []
                        if not choices:
                            continue
                        delta = choices[0].get("delta") or {}

                        # DEBUG: Log chunk details
                        self.log_message(f"📦 Chunk #{chunks_received}: delta={json.dumps(delta)[:200]}...")

                        # ── Text content ──────────────────────────────────────
                        content = delta.get("content")
                        if content:
                            if not text_started:
                                text_out_idx       = next_output_index
                                next_output_index += 1
                                text_started       = True

                                if not _safe_write(self.wfile, _sse({
                                    "type": "response.output_item.added",
                                    "output_index": text_out_idx,
                                    "item": {
                                        "id": msg_id, "type": "message",
                                        "role": "assistant", "content": [],
                                        "status": "in_progress",
                                    }
                                }), self.log_message):
                                    return

                                if not _safe_write(self.wfile, _sse({
                                    "type":          "response.content_part.added",
                                    "item_id":       msg_id,
                                    "output_index":  text_out_idx,
                                    "content_index": 0,
                                    "part":          {"type": "output_text", "text": ""},
                                }), self.log_message):
                                    return

                            full_text += content
                            try:
                                self.wfile.write(_sse({
                                    "type":          "response.output_text.delta",
                                    "item_id":       msg_id,
                                    "output_index":  text_out_idx,
                                    "content_index": 0,
                                    "delta":         content,
                                }))
                                self.wfile.flush()
                            except BrokenPipeError:
                                self.log_message("⚠️ Codex disconnected during text stream")
                                return

                        # ── Tool call deltas ──────────────────────────────────
                        for tc_delta in (delta.get("tool_calls") or []):
                            didx = tc_delta.get("index", 0)

                            # DEBUG: Log incoming tool call delta
                            self.log_message(f"🔍 Tool delta idx={didx}: {json.dumps(tc_delta)}")

                            if didx not in tool_call_state:
                                fc_id   = _make_id("fc")
                                out_idx = next_output_index
                                next_output_index += 1
                                call_id = tc_delta.get("id", "")
                                name    = (tc_delta.get("function") or {}).get("name", "")
                                tool_call_state[didx] = {
                                    "fc_id":        fc_id,
                                    "call_id":      call_id,
                                    "name":         name,
                                    "arguments":    "",
                                    "output_index": out_idx,
                                }
                                self.log_message(
                                    f"🔧 NEW tool call: idx={didx} fc_id={fc_id} "
                                    f"call_id={call_id} name={name} out_idx={out_idx}"
                                )
                                if not _safe_write(self.wfile, _sse({
                                    "type": "response.output_item.added",
                                    "output_index": out_idx,
                                    "item": {
                                        "id":        fc_id,
                                        "type":      "function_call",
                                        "call_id":   call_id,
                                        "name":      name,
                                        "arguments": "",
                                        "status":    "in_progress",
                                    }
                                }), self.log_message):
                                    return

                            tc = tool_call_state[didx]

                            # Patch in id/name if they arrive in later deltas
                            if tc_delta.get("id") and not tc["call_id"]:
                                tc["call_id"] = tc_delta["id"]
                                self.log_message(f"🔧 Patched call_id: {tc['call_id']}")
                            fn = tc_delta.get("function") or {}
                            if fn.get("name") and not tc["name"]:
                                tc["name"] = fn["name"]
                                self.log_message(f"🔧 Patched name: {tc['name']}")

                            args_delta = fn.get("arguments", "")
                            if args_delta:
                                tc["arguments"] += args_delta
                                self.log_message(f"📝 Args delta ({len(args_delta)} chars): {args_delta[:50]}...")
                                try:
                                    self.wfile.write(_sse({
                                        "type":         "response.function_call_arguments.delta",
                                        "item_id":      tc["fc_id"],
                                        "output_index": tc["output_index"],
                                        "delta":        args_delta,
                                    }))
                                    self.wfile.flush()
                                except BrokenPipeError:
                                    self.log_message("⚠️ Codex disconnected during tool stream")
                                    return

                # ── Close text item ───────────────────────────────────────────
                if text_started:
                    if not _safe_write(self.wfile, _sse({
                        "type": "response.output_text.done",
                        "item_id": msg_id, "output_index": text_out_idx,
                        "content_index": 0, "text": full_text,
                    }), self.log_message):
                        return
                    if not _safe_write(self.wfile, _sse({
                        "type": "response.content_part.done",
                        "item_id": msg_id, "output_index": text_out_idx,
                        "content_index": 0,
                        "part": {"type": "output_text", "text": full_text},
                    }), self.log_message):
                        return
                    if not _safe_write(self.wfile, _sse({
                        "type": "response.output_item.done",
                        "output_index": text_out_idx,
                        "item": {
                            "id": msg_id, "type": "message", "role": "assistant",
                            "content": [{"type": "output_text", "text": full_text}],
                            "status": "completed",
                        }
                    }), self.log_message):
                        return

                # ── Close tool call items ─────────────────────────────────────
                for tc in tool_call_state.values():
                    self.log_message(f"✅ Closing tool call: {tc['name']} args={tc['arguments'][:100]}...")

                    if not _safe_write(self.wfile, _sse({
                        "type":         "response.function_call_arguments.done",
                        "item_id":      tc["fc_id"],
                        "output_index": tc["output_index"],
                        "arguments":    tc["arguments"],
                    }), self.log_message):
                        return
                    if not _safe_write(self.wfile, _sse({
                        "type": "response.output_item.done",
                        "output_index": tc["output_index"],
                        "item": {
                            "id":        tc["fc_id"],
                            "type":      "function_call",
                            "call_id":   tc["call_id"],
                            "name":      tc["name"],
                            "arguments": tc["arguments"],
                            "status":    "completed",
                        }
                    }), self.log_message):
                        return

                # ── Build completed output list ────────────────────────────────
                completed_output = []
                if text_started:
                    completed_output.append({
                        "id": msg_id, "type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": full_text}],
                        "status": "completed",
                    })
                for tc in tool_call_state.values():
                    completed_output.append({
                        "id":        tc["fc_id"],
                        "type":      "function_call",
                        "call_id":   tc["call_id"],
                        "name":      tc["name"],
                        "arguments": tc["arguments"],
                        "status":    "completed",
                    })

                # DEBUG: Log final state
                self.log_message(f"📊 Final tool_call_state: {json.dumps(tool_call_state, indent=2)}")
                self.log_message(f"📊 Final completed_output: {json.dumps(completed_output, indent=2)}")

                if not _safe_write(self.wfile, _sse({
                    "type": "response.completed",
                    "response": {
                        "id": resp_id, "object": "response",
                        "created_at": created_at, "status": "completed",
                        "model": final_model,
                        "output": completed_output,
                        "usage": {
                            "input_tokens":  usage.get("prompt_tokens", 0),
                            "output_tokens": usage.get("completion_tokens", 0),
                            "total_tokens":  usage.get("total_tokens", 0),
                        },
                        "error": None,
                    }
                }), self.log_message):
                    return

                _safe_write(self.wfile, b"data: [DONE]\n\n", self.log_message)
                self.log_message(
                    f"✅ Codex stream done: {len(full_text)} chars text, "
                    f"{len(tool_call_state)} tool call(s), {chunks_received} chunks"
                )

        except urllib.error.HTTPError as e:
            err_body = e.read()
            self.log_message(f"❌ Upstream {e.code}: {err_body[:200]}")
            try:
                _safe_write(self.wfile, _sse({
                    "type": "response.failed",
                    "response": {
                        "id": resp_id, "object": "response", "status": "failed",
                        "error": {
                            "code":    str(e.code),
                            "message": err_body.decode(errors="replace"),
                        },
                        "output": [],
                    }
                }), self.log_message)
                _safe_write(self.wfile, b"data: [DONE]\n\n", self.log_message)
            except Exception:
                pass

        except BrokenPipeError:
            self.log_message("⚠️ Codex client disconnected mid-stream")

    # ── OpenClaw: /chat/completions with /me routing ───────────────────────────

    def handle_chat_completions(self):
        """Handle chat completion requests with /me routing."""
        try:
            post_data = self._read_body()
            data = json.loads(post_data.decode('utf-8'))

            messages = data.get('messages', [])
            if messages:
                last_msg = messages[-1]
                if last_msg.get('role') == 'user':
                    content = last_msg.get('content', '')

                    if isinstance(content, list):
                        text_parts = []
                        for part in content:
                            if isinstance(part, dict) and part.get('type') == 'text':
                                text_parts.append(part.get('text', ''))
                            elif isinstance(part, str):
                                text_parts.append(part)
                        content = ' '.join(text_parts)
                    elif not isinstance(content, str):
                        content = str(content)

                    func = route_function(content)
                    data['model'] = f"tensorzero::function_name::{func}"

                    if func == 'slash_me':
                        cleaned = clean_message(content)

                        for msg in messages:
                            msg['content'] = strip_metadata_from_content(msg.get('content', ''))

                        MAX_ROLEPLAY_MSGS = 25
                        system_msgs = [m for m in messages if m.get('role') == 'system']
                        non_system  = [m for m in messages if m.get('role') != 'system']
                        if len(non_system) > MAX_ROLEPLAY_MSGS:
                            non_system = non_system[-MAX_ROLEPLAY_MSGS:]
                        messages = system_msgs + non_system

                        messages, skipped_roles = sanitize_roles_for_roleplay(messages)
                        data['messages'] = messages

                        last_msg = messages[-1] if messages else last_msg

                        orig_content = last_msg.get('content', '')
                        if isinstance(orig_content, list):
                            for part in orig_content:
                                if isinstance(part, dict) and part.get('type') == 'text':
                                    part['text'] = clean_message(part.get('text', ''))
                        else:
                            last_msg['content'] = cleaned

                        data.pop('tools', None)
                        data.pop('tool_choice', None)

                        if data.get('max_tokens', 0) > 1024:
                            data['max_tokens'] = 1024

                        skipped_note = ""
                        if skipped_roles:
                            skipped_note = f", skipped roles: {sorted(set(skipped_roles))}"
                        self.log_message(
                            f"🎭 Routed to {func} → {len(messages)} msgs, cleaned: {cleaned!r}{skipped_note}"
                        )
                    else:
                        self.log_message(f"💬 Routed to {func}")

            req = urllib.request.Request(
                TENSORZERO_URL + '/openai/v1/chat/completions',
                data=json.dumps(data).encode('utf-8'),
                headers={
                    'Content-Type': 'application/json',
                    'Authorization': 'Bearer tensorzero-proxy'
                },
                method='POST'
            )

            try:
                with urllib.request.urlopen(req, timeout=300) as response:
                    self.send_response(response.status)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Access-Control-Allow-Origin', '*')
                    self.end_headers()

                    while True:
                        chunk = response.read(8192)
                        if not chunk:
                            break
                        try:
                            self.wfile.write(chunk)
                            self.wfile.flush()
                        except BrokenPipeError:
                            self.log_message("⚠️ Client disconnected during streaming")
                            return

            except BrokenPipeError:
                self.log_message("⚠️ Client disconnected before response")
                return

        except urllib.error.HTTPError as e:
            error_msg = f"HTTP Error {e.code}: {e.reason}"
            self.log_message(f"❌ {error_msg}")
            self.send_response(e.code)
            self.end_headers()
            _safe_write(self.wfile, json.dumps({"error": error_msg}).encode(), self.log_message)

        except Exception as e:
            error_msg = f"Error: {str(e)}"
            self.log_message(f"❌ {error_msg}")
            traceback.print_exc()
            self.send_response(500)
            self.end_headers()
            _safe_write(self.wfile, json.dumps({"error": error_msg}).encode(), self.log_message)

    # ── Embeddings ─────────────────────────────────────────────────────────────

    def handle_embeddings(self):
        try:
            post_data = self._read_body()

            try:
                embed_data = json.loads(post_data.decode('utf-8'))
                inp = embed_data.get('input', '')
                batch_size = len(inp) if isinstance(inp, list) else 1
                self.log_message(f"🔢 Embedding request → {EMBEDDINGS_URL} (batch={batch_size})")
            except Exception:
                self.log_message(f"🔢 Embedding request → {EMBEDDINGS_URL}")

            req = urllib.request.Request(
                EMBEDDINGS_URL + '/v1/embeddings',
                data=post_data,
                headers={'Content-Type': 'application/json'},
                method='POST'
            )

            with urllib.request.urlopen(req, timeout=600) as response:
                content = response.read()

            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Access-Control-Allow-Origin', '*')
            self.end_headers()
            _safe_write(self.wfile, content, self.log_message)

        except urllib.error.HTTPError as e:
            error_msg = f"HTTP Error {e.code}: {e.reason}"
            self.log_message(f"❌ {error_msg}")
            self.send_response(e.code)
            self.end_headers()
            _safe_write(self.wfile, json.dumps({"error": error_msg}).encode(), self.log_message)

        except Exception as e:
            error_msg = f"Error: {str(e)}"
            self.log_message(f"❌ {error_msg}")
            traceback.print_exc()
            self.send_response(500)
            self.end_headers()
            _safe_write(self.wfile, json.dumps({"error": error_msg}).encode(), self.log_message)

    def handle_passthrough(self):
        try:
            post_data = self._read_body() or None

            req = urllib.request.Request(
                TENSORZERO_URL + self.path,
                data=post_data,
                headers={
                    'Content-Type': 'application/json',
                    'Authorization': 'Bearer tensorzero-proxy'
                },
                method=self.command
            )

            self.log_message(f"📬 Passthrough request: {self.path}")

            with urllib.request.urlopen(req, timeout=300) as response:
                content = response.read()

            self.send_response(response.status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Access-Control-Allow-Origin', '*')
            for header, value in response.getheaders():
                if header.lower() not in ['transfer-encoding', 'connection']:
                    self.send_header(header, value)
            self.end_headers()
            _safe_write(self.wfile, content, self.log_message)

        except urllib.error.HTTPError as e:
            error_msg = f"HTTP Error {e.code}: {e.reason}"
            self.log_message(f"❌ {error_msg}")
            self.send_response(e.code)
            self.end_headers()
            _safe_write(self.wfile, json.dumps({"error": error_msg}).encode(), self.log_message)

        except Exception as e:
            error_msg = f"Error: {str(e)}"
            self.log_message(f"❌ {error_msg}")
            traceback.print_exc()
            self.send_response(500)
            self.end_headers()
            _safe_write(self.wfile, json.dumps({"error": error_msg}).encode(), self.log_message)


class ThreadedTCPServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main():
    global TENSORZERO_URL, EMBEDDINGS_URL, PROXY_PORT

    parser = argparse.ArgumentParser(
        description='TensorZero Proxy with /me routing, Codex shim, and embeddings',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
    python3 tensorzero-proxy.py
    python3 tensorzero-proxy.py --tensorzero http://192.168.1.100:3000
    python3 tensorzero-proxy.py --port 3001
        """
    )

    parser.add_argument('--port', '-p', type=int, default=PROXY_PORT,
                        help=f'Port to listen on (default: {PROXY_PORT})')
    parser.add_argument('--tensorzero', '-t', type=str, default=TENSORZERO_URL,
                        help=f'TensorZero URL (default: {TENSORZERO_URL})')
    parser.add_argument('--embeddings', '-e', type=str, default=EMBEDDINGS_URL,
                        help=f'Embeddings server URL (default: {EMBEDDINGS_URL})')
    parser.add_argument('--bind', '-b', type=str, default='0.0.0.0',
                        help='Address to bind to (default: 0.0.0.0 for LAN access)')

    args = parser.parse_args()

    TENSORZERO_URL = args.tensorzero.rstrip('/')
    EMBEDDINGS_URL = args.embeddings.rstrip('/')
    PROXY_PORT = args.port

    print("=" * 70)
    print("🔄 TensorZero Proxy - /me Routing + Codex Shim + Embeddings")
    print("=" * 70)
    print(f"📍 Listening on:   {args.bind}:{PROXY_PORT}")
    print(f"🔗 Chat →          {TENSORZERO_URL}")
    print(f"🔢 Embeddings →    {EMBEDDINGS_URL}")
    print()
    print("🎭 Routing Rules:")
    print("   /me commands    → slash_me function (metadata stripped, tools removed)")
    print("   IRC ACTION      → slash_me function (metadata stripped, tools removed)")
    print("   Normal chat     → openclaw function")
    print("   /responses      → codex function  (Responses API ↔ chat/completions)")
    print("                     text + tool_calls fully translated both directions")
    print()
    print("📝 Client Configuration:")
    print(f"   OpenClaw:   OPENAI_BASE_URL=http://<lan-ip>:{PROXY_PORT}/openai")
    print(f"   codex.toml: base_url = \"http://127.0.0.1:{PROXY_PORT}/openai/v1\"")
    print("=" * 70)
    print()

    try:
        with ThreadedTCPServer((args.bind, PROXY_PORT), ProxyHandler) as httpd:
            print(f"🔊 Server running on {args.bind}:{PROXY_PORT}")
            print("   Press Ctrl+C to stop")
            print()
            httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n\n🛑 Proxy stopped by user")
    except Exception as e:
        print(f"\n❌ Error starting proxy: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()
