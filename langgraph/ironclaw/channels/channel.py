"""Channel abstractions — mirrors src/channels/channel.rs."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import AsyncIterator
from uuid import uuid4


@dataclass
class IncomingMessage:
    """A message arriving on any channel."""

    content: str
    user_id: str = "default"
    channel: str = "repl"
    thread_id: str = field(default_factory=lambda: str(uuid4()))
    timestamp: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    metadata: dict = field(default_factory=dict)


@dataclass
class OutgoingResponse:
    """A response to send back on a channel."""

    content: str
    thread_id: str
    channel: str = "repl"
    metadata: dict = field(default_factory=dict)


class Channel(ABC):
    """
    Abstract channel interface.

    Mirrors the Rust ``Channel`` trait.  Channels produce ``IncomingMessage``
    streams and accept ``OutgoingResponse`` sends.
    """

    @property
    @abstractmethod
    def name(self) -> str:
        ...

    @abstractmethod
    async def receive(self) -> AsyncIterator[IncomingMessage]:
        """Yield incoming messages from this channel."""
        ...

    @abstractmethod
    async def send(self, response: OutgoingResponse) -> None:
        """Send a response back to the originating channel."""
        ...
