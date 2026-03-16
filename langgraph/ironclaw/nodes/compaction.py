"""
Context compaction — mirrors src/agent/compaction.rs.

Three strategies (in ascending cost order):

    Truncate(keep_recent=N)
        Drop the oldest messages, keep the N most recent *turns*.
        No LLM call.  Fast path used when usage > 95 %.

    Summarize(keep_recent=N)
        Call the LLM to summarize the old turns, write the summary to the
        workspace daily log, then truncate.  Used when usage > 85 %.
        If the LLM call fails the error propagates — messages are NOT
        silently truncated on failure (preserves conversation integrity).
        If the workspace write fails the summary is still returned but
        summary_written=False and the old turns are still removed.

    MoveToWorkspace(keep_recent=10)
        Write the full raw transcript of old turns to the workspace daily
        log, then truncate.  No LLM call.  Used at 80–85 % (moderate).
        Falls back to Truncate(5) when no workspace is provided.

Key safety invariant (mirrors Rust):
    * Truncation only happens AFTER a successful workspace write (for
      Summarize and MoveToWorkspace strategies).  If the write fails we
      preserve all turns and return turns_removed=0.

LangGraph integration
---------------------
Call ``compact_messages()`` inside the ``check_signals`` node or as a
dedicated ``compact_context`` node that runs before ``call_llm``.
The function returns a new message list — assign it to ``state.messages``
via a state update dict.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

from langchain_core.messages import (
    AIMessage,
    AnyMessage,
    HumanMessage,
    SystemMessage,
    ToolMessage,
)

from ironclaw.nodes.context_monitor import (
    AnyStrategy,
    MoveToWorkspaceStrategy,
    SummarizeStrategy,
    TruncateStrategy,
    estimate_tokens,
)

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Result dataclass
# ---------------------------------------------------------------------------


@dataclass
class CompactionResult:
    """
    Outcome of a compaction run.  Mirrors Rust ``CompactionResult``.
    """

    messages_before: int
    messages_after: int
    tokens_before: int
    tokens_after: int
    turns_removed: int
    summary_written: bool
    summary: str | None


# ---------------------------------------------------------------------------
# Turn extraction helpers
# ---------------------------------------------------------------------------


def _extract_turns(messages: list[AnyMessage]) -> list[list[AnyMessage]]:
    """
    Group a flat message list into *turns*.

    A turn starts with a HumanMessage and includes all subsequent non-Human
    messages up to (but not including) the next HumanMessage.  SystemMessages
    are excluded from turns (they stay in the preamble).

    Returns a list of turns, where each turn is a list of AnyMessage.
    """
    turns: list[list[AnyMessage]] = []
    current: list[AnyMessage] = []

    for msg in messages:
        if isinstance(msg, SystemMessage):
            continue  # preamble — handled separately
        if isinstance(msg, HumanMessage):
            if current:
                turns.append(current)
            current = [msg]
        else:
            current.append(msg)

    if current:
        turns.append(current)

    return turns


def _system_messages(messages: list[AnyMessage]) -> list[AnyMessage]:
    return [m for m in messages if isinstance(m, SystemMessage)]


def _format_turns_for_storage(turns: list[list[AnyMessage]]) -> str:
    """
    Render turns as human-readable markdown for workspace archival.
    Mirrors ``format_turns_for_storage()`` from Rust.
    """
    lines: list[str] = []
    for i, turn in enumerate(turns, 1):
        lines.append(f"**Turn {i}**")
        tool_names: list[str] = []
        for msg in turn:
            if isinstance(msg, HumanMessage):
                content = msg.content if isinstance(msg.content, str) else str(msg.content)
                lines.append(f"User: {content}")
            elif isinstance(msg, AIMessage):
                if msg.content:
                    content = msg.content if isinstance(msg.content, str) else str(msg.content)
                    lines.append(f"Agent: {content}")
                if msg.tool_calls:
                    tool_names.extend(tc["name"] for tc in msg.tool_calls)
            elif isinstance(msg, ToolMessage):
                tool_names.append(msg.name or "tool")
        if tool_names:
            lines.append(f"Tools: {', '.join(tool_names)}")
        lines.append("")
    return "\n".join(lines)


def _format_turns_for_summary(turns: list[list[AnyMessage]]) -> str:
    """Render turns as plain dialogue for the LLM summarization prompt."""
    parts: list[str] = []
    for turn in turns:
        for msg in turn:
            if isinstance(msg, HumanMessage):
                content = msg.content if isinstance(msg.content, str) else str(msg.content)
                parts.append(f"User: {content}")
            elif isinstance(msg, AIMessage):
                if msg.content:
                    content = msg.content if isinstance(msg.content, str) else str(msg.content)
                    parts.append(f"Assistant: {content}")
            elif isinstance(msg, ToolMessage):
                parts.append(f"Tool ({msg.name}): {msg.content}")
    return "\n\n".join(parts)


# ---------------------------------------------------------------------------
# ContextCompactor
# ---------------------------------------------------------------------------


class ContextCompactor:
    """
    Compacts a LangChain message list using one of three strategies.

    Mirrors Rust ``ContextCompactor``.

    Parameters
    ----------
    llm
        A LangChain chat model (used only by the Summarize strategy).
    workspace
        An ``ironclaw.memory.Workspace`` instance (used by Summarize and
        MoveToWorkspace for archival).  Pass ``None`` to skip archival.
    """

    _SUMMARY_SYSTEM_PROMPT = """\
Summarize the following conversation concisely. Focus on:
- Key decisions made
- Important information exchanged
- Actions taken and their outcomes

Be brief but capture all important details. Use bullet points."""

    def __init__(self, llm: Any = None, workspace: Any = None) -> None:
        self._llm = llm
        self._workspace = workspace

    # ------------------------------------------------------------------
    # Public entry point
    # ------------------------------------------------------------------

    async def compact(
        self,
        messages: list[AnyMessage],
        strategy: AnyStrategy,
    ) -> tuple[list[AnyMessage], CompactionResult]:
        """
        Apply ``strategy`` to ``messages`` and return the compacted list
        plus a ``CompactionResult`` report.

        The original list is never mutated.
        """
        tokens_before = estimate_tokens(messages)
        system_msgs = _system_messages(messages)
        turns = _extract_turns(messages)

        if isinstance(strategy, TruncateStrategy):
            new_turns, result_partial = self._compact_truncate(turns, strategy.keep_recent)
        elif isinstance(strategy, SummarizeStrategy):
            new_turns, result_partial = await self._compact_summarize(
                turns, strategy.keep_recent
            )
        elif isinstance(strategy, MoveToWorkspaceStrategy):
            new_turns, result_partial = await self._compact_move_to_workspace(
                turns, strategy.keep_recent
            )
        else:
            raise ValueError(f"Unknown strategy: {strategy}")

        # Re-assemble: system messages first, then remaining turns flattened
        new_messages = system_msgs + [msg for turn in new_turns for msg in turn]
        tokens_after = estimate_tokens(new_messages)

        result = CompactionResult(
            messages_before=len(messages),
            messages_after=len(new_messages),
            tokens_before=tokens_before,
            tokens_after=tokens_after,
            turns_removed=result_partial["turns_removed"],
            summary_written=result_partial["summary_written"],
            summary=result_partial.get("summary"),
        )

        logger.info(
            "Compaction complete: %d→%d messages, %d→%d tokens, strategy=%s, "
            "turns_removed=%d, summary_written=%s",
            result.messages_before,
            result.messages_after,
            result.tokens_before,
            result.tokens_after,
            type(strategy).__name__,
            result.turns_removed,
            result.summary_written,
        )

        return new_messages, result

    # ------------------------------------------------------------------
    # Strategy: Truncate
    # ------------------------------------------------------------------

    def _compact_truncate(
        self,
        turns: list[list[AnyMessage]],
        keep_recent: int,
    ) -> tuple[list[list[AnyMessage]], dict]:
        """
        Drop oldest turns, keep the N most recent.  No LLM call.

        Mirrors ``compact_truncate()`` from Rust.
        """
        if len(turns) <= keep_recent:
            return turns, {"turns_removed": 0, "summary_written": False}

        turns_removed = len(turns) - keep_recent
        new_turns = turns[turns_removed:]  # keep the tail

        return new_turns, {
            "turns_removed": turns_removed,
            "summary_written": False,
        }

    # ------------------------------------------------------------------
    # Strategy: Summarize
    # ------------------------------------------------------------------

    async def _compact_summarize(
        self,
        turns: list[list[AnyMessage]],
        keep_recent: int,
    ) -> tuple[list[list[AnyMessage]], dict]:
        """
        Summarize old turns via LLM, archive to workspace, then truncate.

        Safety invariant:
        * LLM failure  → propagate exception, turns unchanged
        * Write failure → log warning, summary NOT written but turns still removed
          (the summary is already generated; we don't throw it away)

        Mirrors ``compact_with_summary()`` from Rust.
        """
        if len(turns) <= keep_recent:
            return turns, {"turns_removed": 0, "summary_written": False, "summary": None}

        turns_to_archive = turns[: len(turns) - keep_recent]

        # --- Generate summary (may raise — callers must handle) ---
        summary = await self._generate_summary(turns_to_archive)

        # --- Attempt workspace archival ---
        summary_written = False
        if self._workspace is not None:
            try:
                await self._write_summary_to_workspace(summary)
                summary_written = True
            except Exception as exc:  # noqa: BLE001
                logger.warning(
                    "Compaction summary workspace write failed (turns still removed): %s", exc
                )

        # --- Truncate ---
        new_turns = turns[len(turns) - keep_recent :]

        return new_turns, {
            "turns_removed": len(turns_to_archive),
            "summary_written": summary_written,
            "summary": summary,
        }

    async def _generate_summary(self, turns: list[list[AnyMessage]]) -> str:
        """
        Call the LLM to summarize old turns.

        Mirrors ``generate_summary()`` from Rust.
        Raises if ``self._llm`` is None or if the LLM call fails.
        """
        if self._llm is None:
            raise RuntimeError(
                "ContextCompactor requires an LLM for the Summarize strategy. "
                "Pass llm= when constructing ContextCompactor."
            )

        dialogue = _format_turns_for_summary(turns)
        prompt = (
            f"{self._SUMMARY_SYSTEM_PROMPT}\n\n"
            f"Please summarize this conversation:\n\n{dialogue}"
        )

        response: AIMessage = await self._llm.ainvoke([HumanMessage(content=prompt)])
        text = response.content
        if isinstance(text, list):
            text = " ".join(
                b.get("text", "") if isinstance(b, dict) else str(b) for b in text
            )
        return str(text)

    # ------------------------------------------------------------------
    # Strategy: MoveToWorkspace
    # ------------------------------------------------------------------

    async def _compact_move_to_workspace(
        self,
        turns: list[list[AnyMessage]],
        keep_recent: int,
    ) -> tuple[list[list[AnyMessage]], dict]:
        """
        Archive full turn transcript to workspace then truncate.

        Falls back to Truncate(5) when no workspace is available.

        Safety invariant:
        * Workspace write failure → preserve ALL turns, return turns_removed=0.

        Mirrors ``compact_to_workspace()`` from Rust.
        """
        # No workspace → fall back to truncation
        if self._workspace is None:
            logger.debug("MoveToWorkspace: no workspace provided, falling back to Truncate(5)")
            return self._compact_truncate(turns, keep_recent=5)

        if len(turns) <= keep_recent:
            return turns, {"turns_removed": 0, "summary_written": False}

        turns_to_archive = turns[: len(turns) - keep_recent]
        content = _format_turns_for_storage(turns_to_archive)

        try:
            await self._write_context_to_workspace(content)
        except Exception as exc:  # noqa: BLE001
            # Preserve turns on write failure (mirrors Rust behaviour)
            logger.warning(
                "Compaction context workspace write failed (turns preserved): %s", exc
            )
            return turns, {"turns_removed": 0, "summary_written": False}

        new_turns = turns[len(turns) - keep_recent :]
        return new_turns, {
            "turns_removed": len(turns_to_archive),
            "summary_written": True,
        }

    # ------------------------------------------------------------------
    # Workspace helpers
    # ------------------------------------------------------------------

    async def _write_summary_to_workspace(self, summary: str) -> None:
        """Append summary to today's daily log.  Mirrors Rust ``write_summary_to_workspace``."""
        date = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        time_str = datetime.now(timezone.utc).strftime("%H:%M UTC")
        entry = f"\n## Context Summary ({time_str})\n\n{summary}\n"
        path = f"daily/{date}.md"
        await self._workspace.append(path, entry)

    async def _write_context_to_workspace(self, content: str) -> None:
        """Append raw transcript to today's daily log.  Mirrors Rust ``write_context_to_workspace``."""
        date = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        time_str = datetime.now(timezone.utc).strftime("%H:%M UTC")
        entry = f"\n## Archived Context ({time_str})\n\n{content}\n"
        path = f"daily/{date}.md"
        await self._workspace.append(path, entry)


# ---------------------------------------------------------------------------
# LangGraph node
# ---------------------------------------------------------------------------


async def compact_context_node(
    state: Any,
    compactor: ContextCompactor,
    monitor: Any,
) -> dict[str, Any]:
    """
    LangGraph node: check if compaction is needed, apply if so.

    Inserts between ``check_signals`` and ``call_llm``.  Returns an
    update to ``state.messages`` and clears ``state.needs_compaction``.

    Parameters
    ----------
    state
        Current ``AgentState``.
    compactor
        A configured ``ContextCompactor`` instance.
    monitor
        A configured ``ContextMonitor`` instance.
    """
    strategy = monitor.suggest_compaction(state.messages)
    if strategy is None:
        return {"needs_compaction": False}

    logger.info(
        "Context compaction triggered: %.1f%% of limit, strategy=%s",
        monitor.usage_percent(state.messages),
        type(strategy).__name__,
    )

    new_messages, result = await compactor.compact(state.messages, strategy)

    return {
        "messages": new_messages,   # replace, not append
        "needs_compaction": False,
        # Persist summary as a pseudo-system message so future LLM calls
        # have the context even after truncation
        **(
            {
                "messages": [
                    SystemMessage(
                        content=f"[Context summary from earlier conversation]\n{result.summary}"
                    )
                ]
                + new_messages
            }
            if result.summary
            else {}
        ),
    }
