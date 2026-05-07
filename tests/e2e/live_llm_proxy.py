"""Record/replay HTTP proxy for live-LLM Playwright tests.

This is the Python tier's analogue to the Rust `LiveTestHarnessBuilder`
trace-recording infrastructure (`tests/support/live_harness.rs`). It
sits between ironclaw and a real LLM (NearAI / OpenAI / Anthropic) and:

- In **record** mode, forwards each `/v1/chat/completions` request to
  the upstream LLM, captures the request + response pair, and appends
  it to a JSON fixture file. The committed fixture lets later runs
  replay the conversation deterministically without an LLM API key.

- In **replay** mode, reads the fixture and returns recorded responses
  by matching the canonical request shape (model + tools + message
  sequence). Matching is structural: it ignores non-deterministic
  fields like tool-call IDs, request IDs, and timestamps.

Usage in a test:

    # tests/e2e/conftest.py
    @pytest.fixture
    async def live_llm_proxy(request):
        from live_harness import live_proxy_for
        async for url in live_proxy_for(request.node.name):
            yield url

    # ironclaw_server fixture sets:
    #   LLM_BASE_URL = url
    # The proxy auto-detects record vs replay based on
    # IRONCLAW_LIVE_TEST and the fixture file's existence.

Environment variables:

- ``IRONCLAW_LIVE_TEST=1`` — record mode. Requires upstream LLM
  credentials (``IRONCLAW_LIVE_LLM_BASE_URL``, ``IRONCLAW_LIVE_LLM_API_KEY``,
  ``IRONCLAW_LIVE_LLM_MODEL``). Writes / overwrites the fixture file.
- (unset) — replay mode. Reads the committed fixture file. Skips
  the test (with ``pytest.skip``) when the fixture is missing so a
  fresh checkout doesn't fail before someone has recorded one.

Fixture file shape (JSON):

    {
        "model": "<recorded model id>",
        "entries": [
            {
                "request_hash": "<sha256 of canonicalized request>",
                "request_summary": {
                    "model": "...",
                    "n_messages": <int>,
                    "last_user_content": "<truncated>",
                    "tool_count": <int>
                },
                "response": { ... full /v1/chat/completions JSON ... }
            },
            ...
        ]
    }

Matching uses request_hash. Multiple identical requests produce
multiple entries (each with the same hash); replay consumes them in
order.
"""

import argparse
import asyncio
import hashlib
import json
import os
import re
import sys
import time
import uuid
from pathlib import Path
from typing import Any

import aiohttp
from aiohttp import web


# ── Canonicalization ────────────────────────────────────────────────────


_TOOL_CALL_ID_RE = re.compile(r"call_[A-Za-z0-9_-]{8,}")


def _canonicalize_request(body: dict[str, Any]) -> dict[str, Any]:
    """Strip non-deterministic fields from a chat-completions request.

    Returns a dict suitable for hashing. The goal is two semantically
    identical requests (same conversation, same tools, same goal) to
    produce the same hash regardless of run-to-run variations.
    """
    canon: dict[str, Any] = {
        "model": body.get("model"),
        "messages": [],
    }
    for msg in body.get("messages", []) or []:
        role = msg.get("role")
        content = msg.get("content")
        # Normalize tool-call ids in tool/assistant messages.
        if isinstance(content, str):
            content = _TOOL_CALL_ID_RE.sub("call_<id>", content)
        elif isinstance(content, list):
            new_parts = []
            for part in content:
                if not isinstance(part, dict):
                    new_parts.append(part)
                    continue
                p = dict(part)
                if "text" in p and isinstance(p["text"], str):
                    p["text"] = _TOOL_CALL_ID_RE.sub("call_<id>", p["text"])
                new_parts.append(p)
            content = new_parts
        norm = {"role": role, "content": content}
        if "name" in msg:
            norm["name"] = msg["name"]
        # Drop tool_call_id (non-deterministic across runs).
        if "tool_calls" in msg:
            calls = []
            for tc in msg.get("tool_calls", []) or []:
                tc_copy = {
                    "type": tc.get("type"),
                    "function": {
                        "name": (tc.get("function") or {}).get("name"),
                        "arguments": (tc.get("function") or {}).get("arguments"),
                    },
                }
                calls.append(tc_copy)
            norm["tool_calls"] = calls
        canon["messages"].append(norm)

    if body.get("tools"):
        # Tools list affects model behaviour; include their function
        # names + parameter schemas in the hash. Drop descriptions which
        # we sometimes tweak between recordings.
        tools = []
        for tool in body["tools"]:
            fn = tool.get("function", {}) or {}
            tools.append({
                "name": fn.get("name"),
                "parameters": fn.get("parameters"),
            })
        # Sort so reordering doesn't break replay.
        tools.sort(key=lambda t: t.get("name") or "")
        canon["tools"] = tools

    return canon


def _hash_request(body: dict[str, Any]) -> str:
    canon = _canonicalize_request(body)
    blob = json.dumps(canon, sort_keys=True, ensure_ascii=False).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()


def _summarize_request(body: dict[str, Any]) -> dict[str, Any]:
    last_user = ""
    for msg in body.get("messages", []) or []:
        if msg.get("role") == "user":
            content = msg.get("content")
            if isinstance(content, str):
                last_user = content
            elif isinstance(content, list):
                for part in content:
                    if isinstance(part, dict) and part.get("type") == "text":
                        last_user = part.get("text", "")
                        break
    return {
        "model": body.get("model"),
        "n_messages": len(body.get("messages") or []),
        "last_user_content": last_user[:120],
        "tool_count": len(body.get("tools") or []),
    }


# ── Fixture I/O ─────────────────────────────────────────────────────────


def _empty_fixture(model: str | None) -> dict[str, Any]:
    return {
        "model": model,
        "schema_version": 1,
        "entries": [],
    }


def _load_fixture(path: Path) -> dict[str, Any]:
    if not path.exists():
        return _empty_fixture(None)
    with path.open("r", encoding="utf-8") as fp:
        return json.load(fp)


def _save_fixture(path: Path, fixture: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as fp:
        json.dump(fixture, fp, indent=2, ensure_ascii=False)
        fp.write("\n")


# ── Proxy app ───────────────────────────────────────────────────────────


def _new_state(
    *,
    mode: str,
    fixture_path: Path,
    upstream_url: str | None,
    upstream_key: str | None,
    upstream_model: str | None,
) -> dict[str, Any]:
    fixture = _load_fixture(fixture_path)
    # Track per-hash replay cursor so multiple identical requests in a
    # single run consume distinct recorded entries (e.g. retries).
    cursors: dict[str, int] = {}
    return {
        "mode": mode,
        "fixture_path": fixture_path,
        "fixture": fixture,
        "cursors": cursors,
        "upstream_url": upstream_url,
        "upstream_key": upstream_key,
        "upstream_model": upstream_model,
        "record_count": 0,
        "replay_count": 0,
        "miss_count": 0,
    }


async def chat_completions(request: web.Request) -> web.Response:
    state = request.app["state"]
    body = await request.json()
    request_hash = _hash_request(body)

    if state["mode"] == "replay":
        return await _replay(state, body, request_hash)
    return await _record(state, body, request_hash)


async def _replay(
    state: dict[str, Any], body: dict[str, Any], request_hash: str
) -> web.Response:
    entries = state["fixture"].get("entries", []) or []
    matching = [e for e in entries if e["request_hash"] == request_hash]
    cursor = state["cursors"].setdefault(request_hash, 0)
    if cursor >= len(matching):
        state["miss_count"] += 1
        # Build a diagnostic so the test sees exactly which prompt
        # missed when it inevitably fails to drive the next step.
        summary = _summarize_request(body)
        return web.json_response(
            {
                "error": "live_llm_proxy: no recorded response for this request",
                "request_hash": request_hash,
                "request_summary": summary,
                "fixture_path": str(state["fixture_path"]),
                "available_hashes": [
                    {
                        "hash": e["request_hash"],
                        "summary": e.get("request_summary", {}),
                    }
                    for e in entries
                ],
            },
            status=500,
        )
    entry = matching[cursor]
    state["cursors"][request_hash] = cursor + 1
    state["replay_count"] += 1

    response_body = entry["response"]
    streaming = bool(body.get("stream"))
    if streaming:
        return await _emit_streamed_response(response_body)
    return web.json_response(response_body)


async def _record(
    state: dict[str, Any], body: dict[str, Any], request_hash: str
) -> web.Response:
    upstream_url = state["upstream_url"]
    upstream_key = state["upstream_key"]
    if not upstream_url:
        return web.json_response(
            {"error": "live_llm_proxy: record mode requires IRONCLAW_LIVE_LLM_BASE_URL"},
            status=500,
        )

    # Override the model with the upstream model when configured. This
    # lets ironclaw send the literal "mock-model" string while the
    # proxy sends a real model name to the upstream.
    forwarded_body = dict(body)
    if state.get("upstream_model"):
        forwarded_body["model"] = state["upstream_model"]
    # Force non-streaming upstream so we capture a deterministic JSON
    # body. We can re-emit as streaming on replay if the original
    # request asked for it.
    forwarded_body["stream"] = False

    headers = {"Content-Type": "application/json"}
    if upstream_key:
        headers["Authorization"] = f"Bearer {upstream_key}"

    timeout = aiohttp.ClientTimeout(total=120)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        async with session.post(
            f"{upstream_url.rstrip('/')}/v1/chat/completions",
            json=forwarded_body,
            headers=headers,
        ) as response:
            response_body = await response.json()
            if response.status >= 400:
                return web.json_response(
                    {
                        "error": "live_llm_proxy: upstream returned error",
                        "upstream_status": response.status,
                        "upstream_body": response_body,
                    },
                    status=response.status,
                )

    # Persist the new entry.
    entry = {
        "request_hash": request_hash,
        "request_summary": _summarize_request(body),
        "response": response_body,
    }
    state["fixture"].setdefault("entries", []).append(entry)
    if state["fixture"].get("model") is None and body.get("model"):
        state["fixture"]["model"] = body["model"]
    _save_fixture(state["fixture_path"], state["fixture"])
    state["record_count"] += 1

    streaming = bool(body.get("stream"))
    if streaming:
        return await _emit_streamed_response(response_body)
    return web.json_response(response_body)


async def _emit_streamed_response(body: dict[str, Any]) -> web.StreamResponse:
    """Re-emit a non-streaming chat-completions JSON body as a single
    SSE chunk plus the [DONE] sentinel. Good enough for ironclaw's
    streaming consumer — every test we run here uses the chunk-or-text
    accumulator, not delta-by-delta token rendering.
    """
    response = web.StreamResponse(
        status=200,
        headers={"Content-Type": "text/event-stream"},
    )
    # Build a single-chunk delta from the choice's message.
    choice = (body.get("choices") or [{}])[0]
    message = choice.get("message", {})
    delta = {
        "id": body.get("id", f"chatcmpl-{uuid.uuid4().hex[:24]}"),
        "object": "chat.completion.chunk",
        "created": int(time.time()),
        "model": body.get("model", "live-replay"),
        "choices": [
            {
                "index": 0,
                "delta": {
                    "role": message.get("role", "assistant"),
                    "content": message.get("content"),
                    "tool_calls": message.get("tool_calls"),
                },
                "finish_reason": choice.get("finish_reason", "stop"),
            }
        ],
    }
    return await _send_sse_payload(response, delta)


async def _send_sse_payload(response: web.StreamResponse, delta: dict[str, Any]) -> web.StreamResponse:
    return await _send_sse_lines(response, [json.dumps(delta), "[DONE]"])


async def _send_sse_lines(
    response: web.StreamResponse, payloads: list[str]
) -> web.StreamResponse:
    return await _send_sse(response, payloads)


async def _send_sse(
    response: web.StreamResponse, payloads: list[str]
) -> web.StreamResponse:
    started = False

    async def _start(req: web.Request) -> None:
        nonlocal started
        if not started:
            await response.prepare(req)
            started = True

    # aiohttp StreamResponse needs a request-scoped prepare. We don't
    # have direct access to the original request here; instead, use a
    # trick: build the payload as a single bytes blob and return it as
    # a regular Response with text/event-stream content type. SSE
    # consumers tolerate a complete-on-arrival event stream.
    body_bytes = b""
    for payload in payloads:
        body_bytes += b"data: " + payload.encode("utf-8") + b"\n\n"
    return web.Response(
        body=body_bytes,
        headers={"Content-Type": "text/event-stream"},
    )


async def models(request: web.Request) -> web.Response:
    state = request.app["state"]
    model_id = state["fixture"].get("model") or "live-replay"
    return web.json_response(
        {
            "object": "list",
            "data": [{"id": model_id, "object": "model", "owned_by": "ironclaw-test"}],
        }
    )


async def state_handler(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response(
        {
            "mode": state["mode"],
            "fixture_path": str(state["fixture_path"]),
            "n_entries": len(state["fixture"].get("entries", []) or []),
            "record_count": state["record_count"],
            "replay_count": state["replay_count"],
            "miss_count": state["miss_count"],
        }
    )


# ── Entry point ─────────────────────────────────────────────────────────


def _resolve_mode(args: argparse.Namespace) -> str:
    if args.mode:
        return args.mode
    if os.environ.get("IRONCLAW_LIVE_TEST", "").strip() in ("1", "true"):
        return "record"
    return "replay"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--fixture", required=True, help="Path to the JSON trace fixture.")
    parser.add_argument(
        "--mode",
        choices=("record", "replay"),
        help="Override the IRONCLAW_LIVE_TEST-derived default.",
    )
    args = parser.parse_args()

    mode = _resolve_mode(args)
    upstream_url = os.environ.get("IRONCLAW_LIVE_LLM_BASE_URL")
    upstream_key = os.environ.get("IRONCLAW_LIVE_LLM_API_KEY")
    upstream_model = os.environ.get("IRONCLAW_LIVE_LLM_MODEL")

    if mode == "record" and not upstream_url:
        print(
            "live_llm_proxy: record mode requires "
            "IRONCLAW_LIVE_LLM_BASE_URL (and usually IRONCLAW_LIVE_LLM_API_KEY).",
            file=sys.stderr,
        )
        sys.exit(2)

    fixture_path = Path(args.fixture)
    state = _new_state(
        mode=mode,
        fixture_path=fixture_path,
        upstream_url=upstream_url,
        upstream_key=upstream_key,
        upstream_model=upstream_model,
    )

    app = web.Application()
    app["state"] = state
    app.router.add_post("/v1/chat/completions", chat_completions)
    app.router.add_post("/chat/completions", chat_completions)
    app.router.add_get("/v1/models", models)
    app.router.add_get("/models", models)
    app.router.add_get("/__live/state", state_handler)

    async def start() -> None:
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        print(f"LIVE_LLM_PROXY_PORT={port}", flush=True)
        print(
            f"live_llm_proxy: mode={mode} fixture={fixture_path} "
            f"entries={len(state['fixture'].get('entries', []) or [])}",
            flush=True,
        )
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
