"""
Tests for context compaction — mirrors the Rust test suite in
src/agent/compaction.rs and src/agent/context_monitor.rs.

Test numbering follows the Rust file so it's easy to cross-reference.
"""

from __future__ import annotations

import pytest
from langchain_core.messages import AIMessage, HumanMessage, SystemMessage, ToolMessage

from ironclaw.memory.workspace import Workspace
from ironclaw.nodes.compaction import (
    CompactionResult,
    ContextCompactor,
    _extract_turns,
    _format_turns_for_storage,
)
from ironclaw.nodes.context_monitor import (
    ContextBreakdown,
    ContextMonitor,
    MoveToWorkspaceStrategy,
    SummarizeStrategy,
    TruncateStrategy,
    estimate_tokens,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def make_turns(n: int) -> list[list[HumanMessage | AIMessage]]:
    """Build n turns as [[HumanMessage, AIMessage], ...].  Mirrors make_thread()."""
    return [
        [HumanMessage(content=f"msg-{i}"), AIMessage(content=f"resp-{i}")]
        for i in range(n)
    ]


def turns_to_messages(
    turns: list[list[HumanMessage | AIMessage]],
) -> list[HumanMessage | AIMessage]:
    return [msg for turn in turns for msg in turn]


def make_messages(n: int) -> list[HumanMessage | AIMessage]:
    return turns_to_messages(make_turns(n))


class StubLlm:
    """LLM stub for tests — returns a fixed response string."""

    def __init__(self, response: str = "summary text") -> None:
        self._response = response
        self.calls = 0

    async def ainvoke(self, messages) -> AIMessage:
        self.calls += 1
        return AIMessage(content=self._response)


class FailingLlm:
    """LLM stub that always raises."""

    async def ainvoke(self, messages) -> AIMessage:
        raise RuntimeError("LLM is broken")


class FailingWorkspace:
    """Workspace stub whose append() always raises."""

    async def append(self, path: str, content: str) -> None:
        raise OSError("Disk is full")


# ---------------------------------------------------------------------------
# ContextMonitor tests  (mirrors context_monitor.rs tests)
# ---------------------------------------------------------------------------


def test_token_estimation_basic():
    """Mirrors test_token_estimation."""
    msg = HumanMessage(content="Hello, how are you today?")
    tokens = estimate_tokens([msg])
    # 5 words * 1.3 + 4 = ~10-11 tokens
    assert 0 < tokens < 20


def test_needs_compaction_small_context():
    """Small context — no compaction needed."""
    monitor = ContextMonitor(context_limit=100)
    msgs = [HumanMessage(content="Hello")]
    assert not monitor.needs_compaction(msgs)


def test_needs_compaction_large_context():
    """Large context — compaction needed."""
    monitor = ContextMonitor(context_limit=100)
    large = HumanMessage(content="word " * 1000)
    assert monitor.needs_compaction([large])


def test_suggest_compaction_none_below_threshold():
    """Mirrors test_suggest_compaction — no suggestion below threshold."""
    monitor = ContextMonitor(context_limit=100_000)
    msgs = [HumanMessage(content="Hello")]
    assert monitor.suggest_compaction(msgs) is None


def test_suggest_compaction_truncate_critical():
    """Usage > 95% → TruncateStrategy."""
    monitor = ContextMonitor(context_limit=100)
    msgs = [HumanMessage(content="word " * 10_000)]
    strategy = monitor.suggest_compaction(msgs)
    assert isinstance(strategy, TruncateStrategy)
    assert strategy.keep_recent == 3


def test_suggest_compaction_summarize_high():
    """Usage 85–95% → SummarizeStrategy."""
    # Use a tiny limit so moderate content hits the 85% band
    monitor = ContextMonitor(context_limit=15)
    # ~13 tokens → ~87 % of 15
    msgs = [HumanMessage(content="word " * 7)]
    strategy = monitor.suggest_compaction(msgs)
    assert isinstance(strategy, SummarizeStrategy)
    assert strategy.keep_recent == 5


def test_suggest_compaction_move_to_workspace_moderate():
    """Usage 80–85% → MoveToWorkspaceStrategy."""
    monitor = ContextMonitor(context_limit=20)
    # ~14 tokens → ~82% of 20   (just past 80 % but below 85 %)
    msgs = [HumanMessage(content="word " * 8)]
    strategy = monitor.suggest_compaction(msgs)
    assert isinstance(strategy, MoveToWorkspaceStrategy)


def test_context_breakdown():
    """Mirrors test_context_breakdown."""
    msgs = [
        SystemMessage(content="You are a helpful assistant."),
        HumanMessage(content="Hello"),
        AIMessage(content="Hi there!"),
    ]
    bd = ContextBreakdown.analyze(msgs)
    assert bd.message_count == 3
    assert bd.system_tokens > 0
    assert bd.user_tokens > 0
    assert bd.assistant_tokens > 0
    assert bd.total_tokens == bd.system_tokens + bd.user_tokens + bd.assistant_tokens


# ---------------------------------------------------------------------------
# Compaction — Truncate  (mirrors tests 1–3 from compaction.rs)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_compact_truncate_keeps_last_n():
    """Test 1: compact_truncate keeps last N turns."""
    compactor = ContextCompactor()
    messages = make_messages(10)
    strategy = TruncateStrategy(keep_recent=3)

    new_msgs, result = await compactor.compact(messages, strategy)

    turns = _extract_turns(new_msgs)
    assert len(turns) == 3

    # The 3 remaining turns should be msg-7, msg-8, msg-9
    assert turns[0][0].content == "msg-7"
    assert turns[1][0].content == "msg-8"
    assert turns[2][0].content == "msg-9"

    assert result.turns_removed == 7
    assert not result.summary_written
    assert result.summary is None
    assert result.tokens_before > result.tokens_after


@pytest.mark.asyncio
async def test_compact_truncate_fewer_turns_than_limit():
    """Test 2: no-op when turns < keep_recent."""
    compactor = ContextCompactor()
    messages = make_messages(2)
    strategy = TruncateStrategy(keep_recent=5)

    new_msgs, result = await compactor.compact(messages, strategy)

    turns = _extract_turns(new_msgs)
    assert len(turns) == 2
    assert turns[0][0].content == "msg-0"
    assert turns[1][0].content == "msg-1"
    assert result.turns_removed == 0


@pytest.mark.asyncio
async def test_compact_truncate_empty_messages():
    """Test 3: compact on empty list is a no-op."""
    compactor = ContextCompactor()
    new_msgs, result = await compactor.compact([], TruncateStrategy(keep_recent=3))

    assert new_msgs == []
    assert result.turns_removed == 0
    assert result.tokens_before == 0
    assert result.tokens_after == 0


# ---------------------------------------------------------------------------
# Compaction — Summarize  (mirrors tests 4–6)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_compact_summarize_produces_summary():
    """Test 4: LLM is called and summary is returned."""
    stub = StubLlm("- User greeted\n- Agent responded\n- Five exchanges")
    compactor = ContextCompactor(llm=stub)
    messages = make_messages(5)
    strategy = SummarizeStrategy(keep_recent=2)

    new_msgs, result = await compactor.compact(messages, strategy)

    turns = _extract_turns(new_msgs)
    assert len(turns) == 2
    assert turns[0][0].content == "msg-3"
    assert turns[1][0].content == "msg-4"

    assert result.turns_removed == 3
    assert result.summary is not None
    assert "User greeted" in result.summary
    assert not result.summary_written   # no workspace provided
    assert stub.calls == 1


@pytest.mark.asyncio
async def test_compact_summarize_llm_failure_does_not_truncate():
    """Test 5: LLM failure propagates; messages are NOT modified."""
    compactor = ContextCompactor(llm=FailingLlm())
    messages = make_messages(8)
    strategy = SummarizeStrategy(keep_recent=3)

    with pytest.raises(RuntimeError):
        await compactor.compact(messages, strategy)

    # Original messages must be unchanged (compactor never mutates in-place)
    assert len(messages) == 16  # 8 turns × 2 messages each


@pytest.mark.asyncio
async def test_compact_summarize_fewer_turns_noop():
    """Test 6: no LLM call when turns <= keep_recent."""
    stub = StubLlm("should not be called")
    compactor = ContextCompactor(llm=stub)
    messages = make_messages(3)
    strategy = SummarizeStrategy(keep_recent=5)

    new_msgs, result = await compactor.compact(messages, strategy)

    assert len(_extract_turns(new_msgs)) == 3
    assert result.turns_removed == 0
    assert result.summary is None
    assert stub.calls == 0


@pytest.mark.asyncio
async def test_compact_summarize_workspace_write_fails_turns_still_removed():
    """
    Mirrors the workspace-write-fails behaviour:
    Rust: if workspace write fails → preserve turns, summary_written=False.
    """
    stub = StubLlm("summary")
    compactor = ContextCompactor(llm=stub, workspace=FailingWorkspace())
    messages = make_messages(8)
    strategy = SummarizeStrategy(keep_recent=3)

    new_msgs, result = await compactor.compact(messages, strategy)

    # Summary WAS generated
    assert result.summary == "summary"
    # But write failed
    assert not result.summary_written
    # Turns are still removed (summary was generated successfully)
    assert result.turns_removed == 5


@pytest.mark.asyncio
async def test_compact_summarize_writes_to_workspace():
    """Summary is written to workspace daily log when write succeeds."""
    stub = StubLlm("the summary")
    ws = Workspace()
    compactor = ContextCompactor(llm=stub, workspace=ws)
    messages = make_messages(5)
    strategy = SummarizeStrategy(keep_recent=2)

    _, result = await compactor.compact(messages, strategy)

    assert result.summary_written
    # Workspace should contain a daily log entry
    from datetime import datetime, timezone
    date = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    content = await ws.read(f"daily/{date}.md")
    assert content is not None
    assert "Context Summary" in content
    assert "the summary" in content


# ---------------------------------------------------------------------------
# Compaction — MoveToWorkspace  (mirrors tests 7–8)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_compact_move_to_workspace_without_workspace_falls_back():
    """Test 7: no workspace → falls back to Truncate(keep_recent=5)."""
    compactor = ContextCompactor()  # no workspace
    messages = make_messages(20)
    strategy = MoveToWorkspaceStrategy()

    new_msgs, result = await compactor.compact(messages, strategy)

    turns = _extract_turns(new_msgs)
    assert len(turns) == 5
    assert result.turns_removed == 15
    assert turns[0][0].content == "msg-15"
    assert turns[4][0].content == "msg-19"


@pytest.mark.asyncio
async def test_compact_move_to_workspace_fewer_turns_noop():
    """Test 8: fewer turns than keep_recent → no-op."""
    compactor = ContextCompactor()  # no workspace → keep_recent fallback=5
    messages = make_messages(4)
    strategy = MoveToWorkspaceStrategy()

    new_msgs, result = await compactor.compact(messages, strategy)

    assert len(_extract_turns(new_msgs)) == 4
    assert result.turns_removed == 0


@pytest.mark.asyncio
async def test_compact_move_to_workspace_write_fails_preserves_turns():
    """Workspace write failure → ALL turns preserved (turns_removed=0)."""
    compactor = ContextCompactor(workspace=FailingWorkspace())
    messages = make_messages(20)
    strategy = MoveToWorkspaceStrategy(keep_recent=10)

    new_msgs, result = await compactor.compact(messages, strategy)

    assert len(_extract_turns(new_msgs)) == 20
    assert result.turns_removed == 0
    assert not result.summary_written


@pytest.mark.asyncio
async def test_compact_move_to_workspace_archives_transcript():
    """Transcript is written to workspace daily log."""
    ws = Workspace()
    compactor = ContextCompactor(workspace=ws)
    messages = make_messages(15)
    strategy = MoveToWorkspaceStrategy(keep_recent=10)

    _, result = await compactor.compact(messages, strategy)

    assert result.turns_removed == 5
    assert result.summary_written
    from datetime import datetime, timezone
    date = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    content = await ws.read(f"daily/{date}.md")
    assert content is not None
    assert "Archived Context" in content


# ---------------------------------------------------------------------------
# format_turns_for_storage  (mirrors tests 9–11)
# ---------------------------------------------------------------------------


def test_format_turns_includes_tool_calls():
    """Test 9: tool call names appear in formatted output."""
    turns = [
        [
            HumanMessage(content="Search for X"),
            AIMessage(
                content="Found X",
                tool_calls=[{"id": "c1", "name": "search", "args": {}, "type": "tool_call"}],
            ),
            ToolMessage(content="result", tool_call_id="c1", name="search"),
        ]
    ]
    formatted = _format_turns_for_storage(turns)
    assert "Turn 1" in formatted
    assert "Search for X" in formatted
    assert "Found X" in formatted
    assert "search" in formatted


def test_format_turns_incomplete_turn():
    """Test 10: incomplete turn (no AI response) — no 'Agent:' line."""
    turns = [[HumanMessage(content="In progress message")]]
    formatted = _format_turns_for_storage(turns)
    assert "In progress message" in formatted
    assert "Agent:" not in formatted


def test_format_turns_empty():
    """Test 11: empty list → empty string."""
    assert _format_turns_for_storage([]) == ""


# ---------------------------------------------------------------------------
# Token count drops after compaction  (mirrors test 12)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_tokens_decrease_after_truncation():
    """Test 12: tokens_after < tokens_before after truncation."""
    compactor = ContextCompactor()
    messages = make_messages(20)
    strategy = TruncateStrategy(keep_recent=5)

    _, result = await compactor.compact(messages, strategy)

    assert result.tokens_after < result.tokens_before


# ---------------------------------------------------------------------------
# Edge cases  (mirrors tests 13–16)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_compact_truncate_keep_zero():
    """Test 13: keep_recent=0 removes all turns."""
    compactor = ContextCompactor()
    messages = make_messages(5)
    new_msgs, result = await compactor.compact(messages, TruncateStrategy(keep_recent=0))

    assert _extract_turns(new_msgs) == []
    assert result.turns_removed == 5
    assert result.tokens_after == 0


@pytest.mark.asyncio
async def test_compact_summarize_keep_zero():
    """Test 14: keep_recent=0 summarizes all turns."""
    stub = StubLlm("Summary of all turns")
    compactor = ContextCompactor(llm=stub)
    messages = make_messages(5)
    new_msgs, result = await compactor.compact(messages, SummarizeStrategy(keep_recent=0))

    assert _extract_turns(new_msgs) == []
    assert result.turns_removed == 5
    assert result.summary == "Summary of all turns"
    assert stub.calls == 1


@pytest.mark.asyncio
async def test_system_messages_preserved_after_compaction():
    """Test 15: SystemMessages survive compaction and come first."""
    compactor = ContextCompactor()
    system = SystemMessage(content="You are helpful.")
    messages = [system] + make_messages(10)
    new_msgs, result = await compactor.compact(messages, TruncateStrategy(keep_recent=3))

    assert isinstance(new_msgs[0], SystemMessage)
    assert new_msgs[0].content == "You are helpful."
    assert result.turns_removed == 7


@pytest.mark.asyncio
async def test_sequential_compactions():
    """Test 16: two compactions in a row produce correct state."""
    compactor = ContextCompactor()
    messages = make_messages(20)

    # First: 20 → 10
    msgs1, r1 = await compactor.compact(messages, TruncateStrategy(keep_recent=10))
    assert len(_extract_turns(msgs1)) == 10
    assert r1.turns_removed == 10

    # Second: 10 → 3
    msgs2, r2 = await compactor.compact(msgs1, TruncateStrategy(keep_recent=3))
    turns2 = _extract_turns(msgs2)
    assert len(turns2) == 3
    assert r2.turns_removed == 7

    # Should be the last 3 from original 20 (msg-17, msg-18, msg-19)
    assert turns2[0][0].content == "msg-17"
    assert turns2[1][0].content == "msg-18"
    assert turns2[2][0].content == "msg-19"


# ---------------------------------------------------------------------------
# No LLM required for Summarize when no turns to archive
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_summarize_no_llm_needed_when_below_threshold():
    """ContextCompactor with no LLM is fine if Summarize is a no-op."""
    compactor = ContextCompactor(llm=None)  # no LLM
    messages = make_messages(3)
    strategy = SummarizeStrategy(keep_recent=5)  # 3 < 5 → no-op

    new_msgs, result = await compactor.compact(messages, strategy)
    assert result.turns_removed == 0  # no LLM was called


@pytest.mark.asyncio
async def test_summarize_raises_without_llm():
    """Summarize raises clearly when LLM is None but turns need archiving."""
    compactor = ContextCompactor(llm=None)
    messages = make_messages(10)
    strategy = SummarizeStrategy(keep_recent=3)

    with pytest.raises(RuntimeError, match="requires an LLM"):
        await compactor.compact(messages, strategy)
