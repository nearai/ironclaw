"""
Context window monitor — mirrors src/agent/context_monitor.rs.

Estimates token usage from the current message list and recommends a
``CompactionStrategy`` when usage approaches the model's context limit.

Token estimation
----------------
We use the same heuristic as the Rust implementation:
    tokens ≈ word_count × 1.3 + 4 (overhead per message)

This avoids a tiktoken dependency while staying within ~15% of the
true count for typical English/code conversations.

Strategy selection thresholds (mirrors Rust exactly):
    usage > 95%  → Truncate(keep_recent=3)   [critical — fast path]
    usage > 85%  → Summarize(keep_recent=5)  [high     — LLM summary]
    usage > 80%  → MoveToWorkspace           [moderate — archive + keep 10]
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Union

from langchain_core.messages import AIMessage, AnyMessage, HumanMessage, SystemMessage, ToolMessage

# ---------------------------------------------------------------------------
# Constants (mirror Rust defaults)
# ---------------------------------------------------------------------------

DEFAULT_CONTEXT_LIMIT: int = 100_000
COMPACTION_THRESHOLD: float = 0.80   # trigger compaction at 80 %
TOKENS_PER_WORD: float = 1.3
MESSAGE_OVERHEAD: int = 4            # tokens for role + structure markers


# ---------------------------------------------------------------------------
# CompactionStrategy
# ---------------------------------------------------------------------------


class CompactionStrategy(Enum):
    """
    Which compaction algorithm to apply.

    Mirrors Rust ``CompactionStrategy`` enum.
    """
    Truncate = "truncate"           # fast: drop old messages, no LLM call
    Summarize = "summarize"         # medium: LLM generates summary of old turns
    MoveToWorkspace = "move_to_workspace"  # slow: archive full transcript


@dataclass(frozen=True)
class TruncateStrategy:
    kind: CompactionStrategy = CompactionStrategy.Truncate
    keep_recent: int = 3


@dataclass(frozen=True)
class SummarizeStrategy:
    kind: CompactionStrategy = CompactionStrategy.Summarize
    keep_recent: int = 5


@dataclass(frozen=True)
class MoveToWorkspaceStrategy:
    kind: CompactionStrategy = CompactionStrategy.MoveToWorkspace
    keep_recent: int = 10  # keep more when archiving to workspace


AnyStrategy = Union[TruncateStrategy, SummarizeStrategy, MoveToWorkspaceStrategy]


# ---------------------------------------------------------------------------
# Token estimation helpers
# ---------------------------------------------------------------------------


def estimate_message_tokens(msg: AnyMessage) -> int:
    """
    Estimate the token cost of a single LangChain message.

    Mirrors ``estimate_message_tokens()`` from Rust.
    """
    content = msg.content
    if isinstance(content, list):
        # Anthropic-style content blocks: [{type: "text", text: "..."}, ...]
        text = " ".join(
            block.get("text", "") if isinstance(block, dict) else str(block)
            for block in content
        )
    else:
        text = str(content)

    word_count = len(text.split())
    return int(word_count * TOKENS_PER_WORD) + MESSAGE_OVERHEAD


def estimate_tokens(messages: list[AnyMessage]) -> int:
    """Total estimated token count for a message list."""
    return sum(estimate_message_tokens(m) for m in messages)


# ---------------------------------------------------------------------------
# ContextBreakdown
# ---------------------------------------------------------------------------


@dataclass
class ContextBreakdown:
    """Per-role token breakdown.  Mirrors Rust ``ContextBreakdown``."""

    total_tokens: int = 0
    system_tokens: int = 0
    user_tokens: int = 0
    assistant_tokens: int = 0
    tool_tokens: int = 0
    message_count: int = 0

    @classmethod
    def analyze(cls, messages: list[AnyMessage]) -> "ContextBreakdown":
        bd = cls(message_count=len(messages))
        for msg in messages:
            t = estimate_message_tokens(msg)
            bd.total_tokens += t
            if isinstance(msg, SystemMessage):
                bd.system_tokens += t
            elif isinstance(msg, HumanMessage):
                bd.user_tokens += t
            elif isinstance(msg, AIMessage):
                bd.assistant_tokens += t
            elif isinstance(msg, ToolMessage):
                bd.tool_tokens += t
        return bd


# ---------------------------------------------------------------------------
# ContextMonitor
# ---------------------------------------------------------------------------


class ContextMonitor:
    """
    Watches the running token total and recommends a compaction strategy.

    Mirrors Rust ``ContextMonitor``.

    Usage::

        monitor = ContextMonitor()
        strategy = monitor.suggest_compaction(state.messages)
        if strategy:
            # run compaction node
    """

    def __init__(
        self,
        context_limit: int = DEFAULT_CONTEXT_LIMIT,
        threshold_ratio: float = COMPACTION_THRESHOLD,
    ) -> None:
        self.context_limit = context_limit
        self.threshold_ratio = max(0.5, min(0.95, threshold_ratio))

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def estimate_tokens(self, messages: list[AnyMessage]) -> int:
        return estimate_tokens(messages)

    def usage_ratio(self, messages: list[AnyMessage]) -> float:
        return self.estimate_tokens(messages) / self.context_limit

    def usage_percent(self, messages: list[AnyMessage]) -> float:
        return self.usage_ratio(messages) * 100.0

    def needs_compaction(self, messages: list[AnyMessage]) -> bool:
        threshold = int(self.context_limit * self.threshold_ratio)
        return self.estimate_tokens(messages) >= threshold

    def suggest_compaction(self, messages: list[AnyMessage]) -> AnyStrategy | None:
        """
        Return the recommended strategy, or ``None`` if not needed.

        Thresholds mirror Rust exactly:
        * >95 % → Truncate(keep_recent=3)
        * >85 % → Summarize(keep_recent=5)
        * >80 % → MoveToWorkspace
        """
        if not self.needs_compaction(messages):
            return None

        ratio = self.usage_ratio(messages)

        if ratio > 0.95:
            return TruncateStrategy(keep_recent=3)
        if ratio > 0.85:
            return SummarizeStrategy(keep_recent=5)
        return MoveToWorkspaceStrategy()

    def threshold_tokens(self) -> int:
        return int(self.context_limit * self.threshold_ratio)
