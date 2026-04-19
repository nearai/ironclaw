#!/usr/bin/env python3
"""Host-side connectivity probe for the IronClaw worker."""

import argparse
import asyncio
import json
import os
import sys
import urllib.error
import urllib.request

try:
    import websockets
except ImportError:  # pragma: no cover - handled at runtime
    websockets = None


def check_http(url: str, timeout: float) -> tuple[bool, str]:
    request = urllib.request.Request(url, method="GET")
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8", errors="replace")
            return True, f"HTTP {response.status} {url} -> {body}"
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        return False, f"HTTP {exc.code} {url} -> {body}"
    except Exception as exc:  # pragma: no cover - network/runtime dependent
        return False, f"HTTP error {url} -> {exc}"


async def check_websocket(ws_url: str, token: str, timeout: float) -> tuple[bool, str]:
    if websockets is None:
        return False, "websockets package is not installed; cannot run websocket probe"

    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"

    try:
        async with websockets.connect(
            ws_url,
            additional_headers=headers or None,
            subprotocols=["ironclaw-agent-v1"],
            open_timeout=timeout,
            close_timeout=timeout,
        ) as websocket:
            ready_raw = await asyncio.wait_for(websocket.recv(), timeout=timeout)
            ready = json.loads(ready_raw)

            await websocket.send(
                json.dumps(
                    {
                        "id": "connectivity-ping-001",
                        "type": "ping",
                        "timestamp": "2026-04-13T00:00:00Z",
                        "payload": {},
                    }
                )
            )
            pong_raw = await asyncio.wait_for(websocket.recv(), timeout=timeout)
            pong = json.loads(pong_raw)

            if ready.get("type") != "ready":
                return False, f"Unexpected first websocket message: {ready}"

            if pong.get("type") != "pong":
                return False, f"Unexpected ping response: {pong}"

            return True, f"WS ready/pong ok {ws_url} -> ready={ready} pong={pong}"
    except Exception as exc:  # pragma: no cover - network/runtime dependent
        return False, f"WS error {ws_url} -> {exc}"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Check IronClaw worker host-side connectivity")
    parser.add_argument("--host", default="127.0.0.1", help="Host to probe (default: 127.0.0.1)")
    parser.add_argument("--health-port", type=int, default=8443, help="HTTP health port (default: 8443)")
    parser.add_argument("--ws-port", type=int, default=9090, help="WebSocket port (default: 9090)")
    parser.add_argument("--ws-path", default="/ws/agent", help="WebSocket path (default: /ws/agent)")
    parser.add_argument("--ws-url", help="Full websocket URL override")
    parser.add_argument("--token", default=os.environ.get("AGENT_AUTH_TOKEN", ""), help="Bearer token for websocket auth")
    parser.add_argument("--timeout", type=float, default=5.0, help="Per-check timeout in seconds (default: 5)")
    parser.add_argument("--skip-http", action="store_true", help="Skip HTTP /health and /ready checks")
    parser.add_argument("--skip-ws", action="store_true", help="Skip websocket ready/pong check")
    return parser.parse_args()


async def async_main() -> int:
    args = parse_args()
    ok = True

    if not args.skip_http:
        health_ok, health_msg = check_http(f"http://{args.host}:{args.health_port}/health", args.timeout)
        ready_ok, ready_msg = check_http(f"http://{args.host}:{args.health_port}/ready", args.timeout)
        print(health_msg)
        print(ready_msg)
        ok = ok and health_ok and ready_ok

    if not args.skip_ws:
        ws_url = args.ws_url or f"ws://{args.host}:{args.ws_port}{args.ws_path}"
        ws_ok, ws_msg = await check_websocket(ws_url, args.token, args.timeout)
        print(ws_msg)
        ok = ok and ws_ok

    return 0 if ok else 1


def main() -> None:
    raise SystemExit(asyncio.run(async_main()))


if __name__ == "__main__":
    main()
