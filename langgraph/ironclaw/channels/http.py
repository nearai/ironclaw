"""HTTP webhook channel — mirrors src/channels/http.rs.

Exposes a FastAPI application that accepts POST /webhook requests and
streams responses via Server-Sent Events.
"""

from __future__ import annotations

import asyncio
import hmac
import hashlib
import json
import logging
from typing import AsyncIterator

from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

from ironclaw.channels.channel import Channel, IncomingMessage, OutgoingResponse

logger = logging.getLogger(__name__)


class WebhookPayload(BaseModel):
    content: str
    user_id: str = "webhook-user"
    thread_id: str | None = None


class HttpChannel(Channel):
    """
    HTTP webhook channel.

    Incoming messages arrive via POST /webhook.
    Responses are streamed back via SSE on the same request.
    """

    name = "http"

    def __init__(self, secret: str | None = None) -> None:
        self._secret = secret
        self._queue: asyncio.Queue[IncomingMessage] = asyncio.Queue()
        self._response_queues: dict[str, asyncio.Queue[str | None]] = {}
        self.app = self._build_app()

    def _verify_signature(self, body: bytes, signature: str) -> bool:
        if not self._secret:
            return True
        expected = hmac.new(
            self._secret.encode(),
            body,
            hashlib.sha256,
        ).hexdigest()
        return hmac.compare_digest(f"sha256={expected}", signature)

    def _build_app(self) -> FastAPI:
        app = FastAPI(title="IronClaw HTTP Channel")

        @app.post("/webhook")
        async def webhook(request: Request, payload: WebhookPayload):
            # Signature verification
            if self._secret:
                sig = request.headers.get("X-Hub-Signature-256", "")
                body = await request.body()
                if not self._verify_signature(body, sig):
                    raise HTTPException(status_code=401, detail="Invalid signature")

            thread_id = payload.thread_id or f"http-{id(request)}"
            msg = IncomingMessage(
                content=payload.content,
                user_id=payload.user_id,
                channel=self.name,
                thread_id=thread_id,
            )

            resp_queue: asyncio.Queue[str | None] = asyncio.Queue()
            self._response_queues[thread_id] = resp_queue

            await self._queue.put(msg)

            async def sse_stream():
                while True:
                    chunk = await resp_queue.get()
                    if chunk is None:
                        break
                    data = json.dumps({"text": chunk})
                    yield f"data: {data}\n\n"

            return StreamingResponse(sse_stream(), media_type="text/event-stream")

        @app.get("/health")
        async def health():
            return {"status": "ok"}

        return app

    async def receive(self) -> AsyncIterator[IncomingMessage]:  # type: ignore[override]
        while True:
            msg = await self._queue.get()
            yield msg

    async def send(self, response: OutgoingResponse) -> None:
        queue = self._response_queues.get(response.thread_id)
        if queue:
            await queue.put(response.content)
            await queue.put(None)  # Signal end of stream
            del self._response_queues[response.thread_id]
        else:
            logger.warning("No response queue for thread %s", response.thread_id)
