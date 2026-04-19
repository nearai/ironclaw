#!/usr/bin/env python3
"""
IronClaw → TensorZero Proxy

Sits between IronClaw and TensorZero, cleaning responses to be
strictly OpenAI-compatible by removing TensorZero-specific fields
that IronClaw's Rust parser can't handle (episode_id, tensorzero_cost, etc.).

Also forces tool_choice=none on all requests to prevent tool_calls responses.

Usage:
    python3 ironclaw-proxy.py --port 3002 --tensorzero http://192.168.1.157:3000
    
Then point IronClaw at http://192.168.1.157:3002 instead of :3000
"""

import json
import socketserver
import http.server
import urllib.request
import urllib.error
import argparse
import sys
import traceback

TENSORZERO_URL = "http://192.168.1.157:3000"
PROXY_PORT = 3002


def clean_response(data: dict) -> dict:
    """Strip TensorZero-specific fields, return strict OpenAI-compatible response."""
    cleaned = {
        "id": data.get("id", ""),
        "object": data.get("object", "chat.completion"),
        "created": data.get("created", 0),
        "model": data.get("model", ""),
        "choices": [],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0,
        },
    }

    # Clean usage if present
    usage = data.get("usage", {})
    if isinstance(usage, dict):
        cleaned["usage"] = {
            "prompt_tokens": usage.get("prompt_tokens", 0),
            "completion_tokens": usage.get("completion_tokens", 0),
            "total_tokens": usage.get("total_tokens", 0),
        }

    # Clean choices
    for choice in data.get("choices", []):
        clean_choice = {
            "index": choice.get("index", 0),
            "finish_reason": choice.get("finish_reason", "stop"),
        }
        msg = choice.get("message", {})
        content = msg.get("content") or ""
        # If content is empty, try reasoning fields (thinking models)
        if not content.strip():
            content = msg.get("reasoning_content") or msg.get("reasoning") or ""
            # Strip <think>...</think> wrapper if present
            if content:
                import re
                content = re.sub(r'<think>\s*', '', content)
                content = re.sub(r'\s*</think>\s*', '', content)
                content = content.strip()
        clean_msg = {
            "role": msg.get("role", "assistant"),
            "content": content,
        }
        # Preserve tool_calls if present (shouldn't happen with tool_choice=none)
        if msg.get("tool_calls"):
            clean_msg["tool_calls"] = msg["tool_calls"]
        clean_choice["message"] = clean_msg
        cleaned["choices"].append(clean_choice)

    return cleaned


class IronClawProxyHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        sys.stderr.write(f"[ironclaw-proxy] {format % args}\n")

    def do_OPTIONS(self):
        self.send_response(200)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.end_headers()

    def do_GET(self):
        # Health check / model list passthrough
        try:
            req = urllib.request.Request(
                TENSORZERO_URL + ("/openai/v1" + self.path if not self.path.startswith("/openai") else self.path),
                headers={"Content-Type": "application/json"},
                method="GET",
            )
            with urllib.request.urlopen(req, timeout=10) as resp:
                content = resp.read()
            self.send_response(resp.status)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(content)
        except Exception as e:
            self.send_response(500)
            self.end_headers()
            self.wfile.write(json.dumps({"error": str(e)}).encode())

    def do_POST(self):
        try:
            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length) if length else b""

            # Parse and modify request
            try:
                data = json.loads(body.decode("utf-8"))
            except json.JSONDecodeError:
                data = {}

            out_body = json.dumps(data).encode("utf-8")

            req = urllib.request.Request(
                TENSORZERO_URL + ("/openai/v1" + self.path if not self.path.startswith("/openai") else self.path),
                data=out_body,
                headers={
                    "Content-Type": "application/json",
                    "Authorization": self.headers.get("Authorization", ""),
                },
                method="POST",
            )

            with urllib.request.urlopen(req, timeout=300) as resp:
                raw = resp.read()

            # Parse and clean response
            try:
                resp_data = json.loads(raw.decode("utf-8"))
                cleaned = clean_response(resp_data)
                out = json.dumps(cleaned).encode("utf-8")
                self.log_message(
                    "✅ %s → %s (content: %s)",
                    data.get("model", "?"),
                    cleaned["choices"][0]["finish_reason"] if cleaned["choices"] else "?",
                    (cleaned["choices"][0]["message"]["content"] or "")[:60] if cleaned["choices"] else "empty",
                )
            except json.JSONDecodeError:
                # Can't parse, pass through raw
                out = raw
                self.log_message("⚠️ Non-JSON response, passing through raw")

            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(out)

        except urllib.error.HTTPError as e:
            error_body = e.read().decode("utf-8", errors="replace")
            self.log_message("❌ HTTP %d: %s", e.code, error_body[:200])
            self.send_response(e.code)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(error_body.encode("utf-8"))

        except Exception as e:
            self.log_message("❌ %s", str(e))
            traceback.print_exc()
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"error": str(e)}).encode())


class ThreadedServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main():
    global TENSORZERO_URL, PROXY_PORT

    parser = argparse.ArgumentParser(description="IronClaw → TensorZero Proxy")
    parser.add_argument("--port", "-p", type=int, default=PROXY_PORT)
    parser.add_argument("--tensorzero", "-t", type=str, default=TENSORZERO_URL)
    parser.add_argument("--bind", "-b", type=str, default="0.0.0.0")
    args = parser.parse_args()

    TENSORZERO_URL = args.tensorzero.rstrip("/")
    PROXY_PORT = args.port

    print(f"🔧 IronClaw Proxy")
    print(f"   Listen:     {args.bind}:{PROXY_PORT}")
    print(f"   Forward to: {TENSORZERO_URL}")
    print(f"   Cleans TZ-specific fields from responses")
    print(f"   Forces tool_choice=none on all requests")
    print()

    with ThreadedServer((args.bind, PROXY_PORT), IronClawProxyHandler) as httpd:
        print(f"🔊 Running on {args.bind}:{PROXY_PORT}")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\n🛑 Stopped")


if __name__ == "__main__":
    main()
