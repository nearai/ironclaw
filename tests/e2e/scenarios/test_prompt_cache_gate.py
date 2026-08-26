"""Self-tests for the mock LLM's prompt-cache prefix gate (#6985).

The gate runs on every mock-LLM test via the autouse `assert_prompt_cache_reuse`
fixture, so a bug in the gate is either a suite-wide false failure or — worse —
a silent hole that lets prefix churn back in. `.claude/rules/review-discipline.md`
("Guardrails are code") requires the check to carry its own regression tests;
these are they.

The cases are written against the real shapes: the pre-#6985 bug (a clock in the
system block), the post-fix shape (the clock riding the tail as a host
reminder), and the one legitimate cause of churn (a tool-surface change).
"""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import mock_llm  # noqa: E402
import mock_llm_trace  # noqa: E402


TOOLS = [{"function": {"name": "builtin.http"}}]
WIDER_TOOLS = TOOLS + [{"function": {"name": "slack.send_message"}}]


def observe(system: str, messages: list[dict], tools: object = None) -> None:
    mock_llm._observe_cache_prefix(
        [{"role": "system", "content": system}] + messages, tools
    )


def reminder(text: str) -> dict:
    return {"role": "user", "content": f"<system-reminder>\n{text}\n</system-reminder>"}


@pytest.fixture(autouse=True)
def _clean_observer():
    mock_llm._reset_cache_observations()
    yield
    mock_llm._reset_cache_observations()


def test_flags_clock_in_the_cached_system_prefix():
    """The exact pre-#6985 regression: a per-run clock inside the system block.

    Nothing functional breaks — the model answers fine — so only a cache-shaped
    assertion can see it. This is the case the gate exists for.
    """
    observe(
        "You are IronClaw. Current time: 10:00",
        [{"role": "user", "content": "hi"}],
        TOOLS,
    )
    observe(
        "You are IronClaw. Current time: 10:01",
        [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "hello"}],
        TOOLS,
    )

    assert len(mock_llm._cache_violations) == 1
    violation = mock_llm._cache_violations[0]
    assert violation["reason"] == "system_prompt_churn_without_tool_surface_change"
    # The diagnostic must point at the actual divergence, not just say "differs".
    assert "diverges at char" in violation["detail"]
    assert "10:00" in violation["detail"] and "10:01" in violation["detail"]


def test_accepts_the_clock_riding_the_conversation_tail():
    """The post-#6985 shape: the clock is a tail host reminder, not prefix.

    The reminder text changes every request by design, so the gate must not
    read that as churn — otherwise the fix would fail its own check.
    """
    observe(
        "You are IronClaw.",
        [{"role": "user", "content": "hi"}, reminder("Time: 10:00")],
        TOOLS,
    )
    observe(
        "You are IronClaw.",
        [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
            reminder("Time: 10:01"),
        ],
        TOOLS,
    )

    assert mock_llm._cache_violations == []
    observation = mock_llm._cache_observations[0]
    assert observation["system_reused"] is True
    # The real user turn was reused; only the ephemeral reminder was replaced.
    assert observation["history_messages_reused"] == 1
    assert observation["history_rewritten"] is False


def test_allows_churn_explained_by_a_tool_surface_change():
    """Installing an extension really does rewrite the capability list.

    That invalidation is the correct price of a real change, so gating on it
    would make the check unusable for the extension-lifecycle scenarios.
    """
    observe(
        "You are IronClaw. Capabilities: builtin.http",
        [{"role": "user", "content": "hi"}],
        TOOLS,
    )
    observe(
        "You are IronClaw. Capabilities: builtin.http, slack.send_message",
        [{"role": "user", "content": "hi"}],
        WIDER_TOOLS,
    )

    assert mock_llm._cache_violations == []
    assert mock_llm._cache_observations[0]["system_change_explained"] is True


def test_separate_conversations_do_not_cross_contaminate():
    """Two scenarios running against one mock must not flag each other.

    The mock is session-scoped, so without per-conversation grouping every test
    would blame the previous test's system prompt.
    """
    observe(
        "System for scenario A", [{"role": "user", "content": "same opening"}], TOOLS
    )
    observe(
        "System for scenario B", [{"role": "user", "content": "same opening"}], TOOLS
    )
    observe("System without a user A", [], TOOLS)
    observe("System without a user B", [], TOOLS)

    assert mock_llm._cache_violations == []


def test_history_rewrite_is_measured_but_not_gated():
    """A rewritten earlier message is recorded, not failed.

    History rewriting kills cache reuse just as thoroughly as prefix churn, but
    it has legitimate causes (summarization, tool-result truncation), so the
    signal is a statistic for a ratchet to watch rather than a per-test gate.
    """
    observe(
        "You are IronClaw.",
        [
            {"role": "user", "content": "turn one"},
            {"role": "assistant", "content": "reply one"},
            {"role": "user", "content": "turn two"},
        ],
        TOOLS,
    )
    observe(
        "You are IronClaw.",
        [
            {"role": "user", "content": "turn one"},
            {"role": "assistant", "content": "reply one REWRITTEN"},
            {"role": "user", "content": "turn two"},
        ],
        TOOLS,
    )

    assert mock_llm._cache_violations == []
    observation = mock_llm._cache_observations[0]
    assert observation["history_rewritten"] is True
    # Only the leading user turn survived byte-identical.
    assert observation["history_messages_reused"] == 1


def test_churn_plus_history_rewrite_starts_a_new_chain_and_is_not_gated():
    """Pin the conservative `_conversation_key` blind spot.

    A request that rewrites history and churns the prefix matches no
    continuation branch, so it starts a new chain without recording churn.
    """
    observe(
        "You are IronClaw. Current time: 10:00",
        [
            {"role": "user", "content": "turn one"},
            {"role": "assistant", "content": "reply one"},
        ],
        TOOLS,
    )
    observe(
        "You are IronClaw. Current time: 10:01",
        [
            {"role": "user", "content": "turn one"},
            {"role": "assistant", "content": "reply one REWRITTEN"},
        ],
        TOOLS,
    )

    assert mock_llm._cache_violations == []
    assert len(mock_llm._cache_chains) == 2


def test_compaction_starts_a_new_chain_rather_than_reporting_churn():
    """Documents conservative history-continuation behavior after compaction.

    Compaction replaces the head of the transcript, so the mock cannot tell the
    compacted conversation from a brand-new one and starts a fresh chain. That
    is the safe direction — it under-reports rather than blaming a legitimate
    compaction for churn — but it does mean cache accounting restarts there.
    """
    observe("You are IronClaw.", [{"role": "user", "content": "turn one"}], TOOLS)
    observe(
        "You are IronClaw.",
        [{"role": "user", "content": "[summary of earlier conversation]"}],
        TOOLS,
    )

    assert mock_llm._cache_violations == []
    assert mock_llm._cache_observations == []
    assert len(mock_llm._cache_chains) == 2


def test_first_request_of_a_conversation_is_not_scored():
    """A conversation's first request can neither hit nor miss the cache."""
    observe("You are IronClaw.", [{"role": "user", "content": "only turn"}], TOOLS)

    assert mock_llm._cache_observations == []
    assert mock_llm._cache_violations == []


# ---------------------------------------------------------------------------
# Host-reminder recognition — shared by both mock paths.
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "predicate", [mock_llm._is_host_reminder, mock_llm_trace._is_host_reminder]
)
def test_both_mock_paths_skip_host_reminders(predicate):
    """The canned mock and the trace mock must agree on what a reminder is.

    They anchor response matching on "the last user message"; if only one of
    them skips reminders, a trace-backed fixture matches the runtime clock
    instead of the user's ask and replays the wrong step.
    """
    assert predicate(
        {
            "role": "user",
            "content": "<system-reminder>\nTime: 10:00\n</system-reminder>",
        }
    )
    assert not predicate({"role": "user", "content": "summarize the report"})


@pytest.mark.parametrize(
    "predicate", [mock_llm._is_host_reminder, mock_llm_trace._is_host_reminder]
)
def test_opening_tag_alone_is_not_a_reminder(predicate):
    """A user message that merely starts with the literal tag is still a user ask.

    Matching the opening delimiter alone would swallow it, leaving trace replay
    to anchor on an earlier turn or an empty ask. The emitter escapes payload
    delimiters, so requiring a complete frame is unambiguous.
    """
    assert not predicate(
        {"role": "user", "content": "<system-reminder> explain this to me"}
    )
    assert not predicate(
        {"role": "user", "content": "why does </system-reminder> appear here?"}
    )


def test_reminder_is_skipped_when_selecting_the_last_user_message():
    """Drive the real selectors, not just the predicate (test through the caller)."""
    messages = [
        {"role": "system", "content": "prefix"},
        {"role": "user", "content": "what is the weather"},
        {
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {
                    "id": "call-1",
                    "function": {"name": "weather", "arguments": "{}"},
                }
            ],
        },
        {
            "role": "tool",
            "tool_call_id": "call-1",
            "name": "weather",
            "content": "sunny",
        },
        {
            "role": "user",
            "content": "<system-reminder>\nTime: 10:00\n</system-reminder>",
        },
    ]

    assert mock_llm._last_user_content(messages) == "what is the weather"
    assert mock_llm._last_user_message(messages)["content"] == "what is the weather"
    assert mock_llm_trace._last_user_content(messages) == "what is the weather"
    for finder in (mock_llm._find_tool_results, mock_llm_trace._find_tool_results):
        results = finder(messages, after_latest_user=True)
        assert len(results) == 1
        assert results[0]["content"] == "sunny"
