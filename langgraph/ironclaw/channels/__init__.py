"""Multi-channel input — mirrors src/channels/."""

from ironclaw.channels.channel import Channel, IncomingMessage, OutgoingResponse
from ironclaw.channels.repl import ReplChannel

__all__ = ["Channel", "IncomingMessage", "OutgoingResponse", "ReplChannel"]
