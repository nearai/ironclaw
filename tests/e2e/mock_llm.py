"""Mock OpenAI-compatible LLM server for E2E tests."""

import argparse
import json
import re
import time
import uuid

from aiohttp import web

CANNED_RESPONSES = [
    (re.compile(r"hello|hi|hey", re.IGNORECASE), "Hello! How can I help you today?"),
    (re.compile(r"2\s*\+\s*2|two plus two", re.IGNORECASE), "The answer is 4."),
    (re.compile(r"skill|install", re.IGNORECASE), "I can help you with skills management."),
    (re.compile(r"html.?test|injection.?test", re.IGNORECASE),
     'Here is some content: <script>alert("xss")</script> and <img src=x onerror="alert(1)"> and <iframe src="javascript:alert(2)"></iframe> end of content.'),
]
DEFAULT_RESPONSE = "I understand your request."


def match_response(messages: list[dict]) -> str:
    """Find canned response for the last user message."""
    for msg in reversed(messages):
        if msg.get("role") == "user":
            content = msg.get("content", "")
            # Handle content that may be a list (multi-modal)
            if isinstance(content, list):
                content = " ".join(
                    part.get("text", "") for part in content if part.get("type") == "text"
                )
            for pattern, response in CANNED_RESPONSES:
                if pattern.search(content):
                    return response
            return DEFAULT_RESPONSE
    return DEFAULT_RESPONSE


async def chat_completions(request: web.Request) -> web.StreamResponse:
    """Handle POST /v1/chat/completions."""
    body = await request.json()
    messages = body.get("messages", [])
    stream = body.get("stream", False)
    response_text = match_response(messages)
    completion_id = f"mock-{uuid.uuid4().hex[:8]}"

    if not stream:
        return web.json_response({
            "id": completion_id,
            "object": "chat.completion",
            "created": int(time.time()),
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": response_text},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": len(response_text.split()), "total_tokens": 15},
        })

    # Streaming response: split into word-boundary chunks
    resp = web.StreamResponse(
        status=200,
        headers={"Content-Type": "text/event-stream", "Cache-Control": "no-cache"},
    )
    await resp.prepare(request)

    # First chunk: role
    chunk = {
        "id": completion_id,
        "object": "chat.completion.chunk",
        "created": int(time.time()),
        "model": "mock-model",
        "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": None}],
    }
    await resp.write(f"data: {json.dumps(chunk)}\n\n".encode())

    # Content chunks: split on spaces
    words = response_text.split(" ")
    for i, word in enumerate(words):
        text = word if i == 0 else f" {word}"
        chunk["choices"][0]["delta"] = {"content": text}
        await resp.write(f"data: {json.dumps(chunk)}\n\n".encode())

    # Final chunk: finish_reason
    chunk["choices"][0]["delta"] = {}
    chunk["choices"][0]["finish_reason"] = "stop"
    await resp.write(f"data: {json.dumps(chunk)}\n\n".encode())
    await resp.write(b"data: [DONE]\n\n")

    return resp


async def models(_request: web.Request) -> web.Response:
    """Handle GET /v1/models."""
    return web.json_response({
        "object": "list",
        "data": [{"id": "mock-model", "object": "model", "owned_by": "test"}],
    })


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()

    app = web.Application()
    app.router.add_post("/v1/chat/completions", chat_completions)
    app.router.add_get("/v1/models", models)

    # Use aiohttp's runner to get the actual bound port
    import asyncio

    async def start():
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        # Extract the actual port from the bound socket
        port = site._server.sockets[0].getsockname()[1]
        print(f"MOCK_LLM_PORT={port}", flush=True)
        # Block forever
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
