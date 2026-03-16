"""REPL channel — mirrors src/channels/repl.rs.

Reads from stdin and writes to stdout.  Used for the interactive
command-line interface.
"""

from __future__ import annotations

import asyncio
import sys
from typing import AsyncIterator

from ironclaw.channels.channel import Channel, IncomingMessage, OutgoingResponse


class ReplChannel(Channel):
    """Simple REPL channel for interactive terminal usage."""

    name = "repl"

    def __init__(self, user_id: str = "user", thread_id: str | None = None) -> None:
        self._user_id = user_id
        self._thread_id = thread_id or "repl-default"

    async def receive(self) -> AsyncIterator[IncomingMessage]:  # type: ignore[override]
        loop = asyncio.get_event_loop()
        while True:
            try:
                line = await loop.run_in_executor(None, sys.stdin.readline)
            except (EOFError, KeyboardInterrupt):
                break
            content = line.rstrip("\n")
            if not content:
                continue
            yield IncomingMessage(
                content=content,
                user_id=self._user_id,
                channel=self.name,
                thread_id=self._thread_id,
            )

    async def send(self, response: OutgoingResponse) -> None:
        print(f"\n{response.content}\n", flush=True)
