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
# UUIDs (project_id, thread_id, mission_id, etc.) are dynamic per run.
# Strip them so hashing is stable across recordings.
_UUID_RE = re.compile(
    r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
    re.IGNORECASE,
)
# RFC 3339 timestamps embedded in system prompts / tool results.
_TS_RE = re.compile(
    r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?"
)


def _strip_dynamic(text: str) -> str:
    """Strip run-to-run non-determinism from a string for hashing.

    Replaces UUIDs, tool-call ids, and timestamps with placeholders
    and elides the skills section (whose order varies because the
    skill registry iterates a HashMap, and whose content varies as
    skills are added/removed from the registry) so two semantically-
    identical requests produce the same hash regardless of run-to-run
    variation.
    """
    text = _TOOL_CALL_ID_RE.sub("call_<id>", text)
    text = _UUID_RE.sub("<uuid>", text)
    text = _TS_RE.sub("<ts>", text)
    text = _normalize_skills_block(text)
    text = _normalize_mission_list_result(text)
    return text


_SKILL_MARKER_RE = re.compile(r"(?:^|\n)(?:### )?\[SKILL\] skill:([A-Za-z0-9_\-]+)")
# End-of-skills boundaries that appear in real prompts. We can't use a
# generic `## ` regex because skill bodies frequently contain markdown
# `## ` headers — those would falsely terminate the skills section.
_SKILLS_END_BOUNDARIES = (
    "\nThis is thread #",
    "\n## Instructions\n",
    "\n## Available Actions\n",
    "\n## Tools Available\n",
)


def _normalize_skills_block(text: str) -> str:
    """Drop the entire `[SKILL] skill:NAME ...` block, replacing it
    with a single `[SKILLS]` placeholder.

    Skills appear in two forms: prefixed with `### ` (system prompt
    style) or bare (rendered into the user-facing mission goal). The
    full body of each skill varies as the registry adds/edits/removes
    entries between recordings. The local skill registry is also
    machine-specific, so even the *set* of active skill names cannot
    be assumed stable across record/replay machines.

    For canonicalization we therefore drop the whole block — the
    deterministic test prompt is engineered so the LLM's response
    does not branch on which skills are present. The resulting
    placeholder is intentionally opaque (no embedded names).

    Strategy: locate the first `[SKILL] skill:NAME` marker, find the
    end of the skills section (next known top-level boundary or end
    of string), and replace the entire range with `[SKILLS]`.
    """
    matches = list(_SKILL_MARKER_RE.finditer(text))
    if not matches:
        return text
    head_end = matches[0].start()
    # Find where the skills section ends. Search after the last match
    # for a known top-level boundary; if none, the section runs to EOF.
    last_match_end = matches[-1].end()
    tail_start = len(text)
    for boundary in _SKILLS_END_BOUNDARIES:
        idx = text.find(boundary, last_match_end)
        if idx != -1 and idx < tail_start:
            tail_start = idx
    head = text[:head_end]
    tail = text[tail_start:] if tail_start < len(text) else ""
    # Drop the entire skills block from the canonical form. The skill
    # registry is local-machine state (skills can be installed/removed
    # at any time) so the *set* of active skills cannot be assumed
    # stable across record/replay machines. The deterministic test
    # prompt is engineered so the LLM's response does not branch on
    # which skills are present.
    return f"{head}\n[SKILLS]\n{tail}"


_MISSION_LIST_RE = re.compile(r"\[\{'cadence':.*?\}\](?=\n|$|]|,)", re.DOTALL)


def _normalize_mission_list_result(text: str) -> str:
    """Collapse mission_list tool results to a stable shape.

    The mission_list tool returns full mission rows with descriptions
    that contain non-deterministic content (system seed missions can
    be added/reordered between runs). For canonicalization we only
    care about the names. With the deterministic sort applied in
    `list_missions_with_shared`, a stable repr appears in the
    fixture; this helper protects against past recordings whose
    capture predates the sort.
    """
    return text  # No-op; sort in store_adapter.rs handles ordering now.


def _canonicalize_request(body: dict[str, Any]) -> dict[str, Any]:
    """Build a stable hash key for a chat-completions request.

    The full system prompt isn't hashed because it varies run-to-run
    (skills loaded in HashMap order, embedded UUIDs/timestamps, etc.)
    while the LLM's response selection is driven by a much smaller
    set of stable inputs:

    - The model id (selects the response shape).
    - The conversation tail: roles + payloads of the last few
      non-system messages, with UUIDs/timestamps/tool-call-ids
      stripped. This captures "what is the LLM being asked, given
      what it just did".
    - The set of tool names exposed (function calls fall through to
      hash-based dispatch).

    Two semantically-identical requests (same conversation tail,
    same tool surface) produce the same hash regardless of system-
    prompt drift.
    """
    canon: dict[str, Any] = {
        "model": body.get("model"),
        "tail": [],
    }
    # Walk all non-system messages; stable tail captures the
    # conversation state. System prompts vary too much to hash.
    for msg in body.get("messages", []) or []:
        role = msg.get("role")
        if role == "system":
            continue
        content = msg.get("content")
        if isinstance(content, str):
            content = _strip_dynamic(content)
        elif isinstance(content, list):
            new_parts = []
            for part in content:
                if not isinstance(part, dict):
                    new_parts.append(part)
                    continue
                p = dict(part)
                if "text" in p and isinstance(p["text"], str):
                    p["text"] = _strip_dynamic(p["text"])
                new_parts.append(p)
            content = new_parts
        norm = {"role": role, "content": content}
        if "name" in msg:
            norm["name"] = msg["name"]
        if "tool_calls" in msg:
            calls = []
            for tc in msg.get("tool_calls", []) or []:
                args = (tc.get("function") or {}).get("arguments")
                if isinstance(args, str):
                    args = _strip_dynamic(args)
                calls.append({
                    "type": tc.get("type"),
                    "function": {
                        "name": (tc.get("function") or {}).get("name"),
                        "arguments": args,
                    },
                })
            norm["tool_calls"] = calls
        canon["tail"].append(norm)

    if body.get("tools"):
        # Tool *names* drive response selection; full schemas don't.
        # Sort so reordering doesn't break replay.
        canon["tools"] = sorted(
            (tool.get("function", {}) or {}).get("name") or ""
            for tool in body["tools"]
        )

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
    print(
        f"live_llm_proxy: chat_completions mode={state['mode']} "
        f"hash={request_hash[:16]} n_msg={len(body.get('messages') or [])} "
        f"tools={len(body.get('tools') or [])}",
        file=sys.stderr,
        flush=True,
    )

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
        # On miss, dump the canonical blob to a debug file so the
        # test author can diff it against fixture entries to find
        # what's different between record and replay.
        canon = _canonicalize_request(body)
        debug_dir = state["fixture_path"].parent
        debug_path = debug_dir / f"{state['fixture_path'].stem}.miss_{state['miss_count']:03d}.json"
        debug_path.write_text(
            json.dumps({"request_hash": request_hash, "canonical": canon}, indent=2)
        )
        # Build a diagnostic so the test sees exactly which prompt
        # missed when it inevitably fails to drive the next step.
        summary = _summarize_request(body)
        return web.json_response(
            {
                "error": "live_llm_proxy: no recorded response for this request",
                "request_hash": request_hash,
                "request_summary": summary,
                "fixture_path": str(state["fixture_path"]),
                "miss_dump": str(debug_path),
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
                print(
                    f"live_llm_proxy: upstream {response.status} body={json.dumps(response_body)[:1500]}",
                    file=sys.stderr,
                    flush=True,
                )
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
        # Keep the canonical blob alongside the entry so a future
        # miss can diff against it without re-recording. The blob
        # is what the hash is computed over.
        "request_canonical": _canonicalize_request(body),
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


async def _emit_streamed_response(body: dict[str, Any]) -> web.Response:
    """Re-emit a non-streaming chat-completions JSON body as a single
    SSE chunk plus the [DONE] sentinel. Good enough for ironclaw's
    streaming consumer — every test we run here uses the chunk-or-text
    accumulator, not delta-by-delta token rendering.

    Returns a one-shot `web.Response` with `text/event-stream` content
    type rather than a true `web.StreamResponse`; the underlying
    `_send_sse` helper buffers the payload and returns a
    `web.Response` because we don't have a request-scoped `prepare()`
    handle here.
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


async def _send_sse_payload(
    response: web.StreamResponse, delta: dict[str, Any]
) -> web.Response:
    return await _send_sse_lines(response, [json.dumps(delta), "[DONE]"])


async def _send_sse_lines(
    response: web.StreamResponse, payloads: list[str]
) -> web.Response:
    return await _send_sse(response, payloads)


async def _send_sse(_response: web.StreamResponse, payloads: list[str]) -> web.Response:
    # aiohttp StreamResponse needs a request-scoped prepare. We don't
    # have direct access to the original request here; instead, use a
    # trick: build the payload as a single bytes blob and return it as
    # a regular Response with text/event-stream content type. SSE
    # consumers tolerate a complete-on-arrival event stream. The
    # `_response` argument is kept for signature symmetry with the
    # streaming variant we may swap in later.
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
