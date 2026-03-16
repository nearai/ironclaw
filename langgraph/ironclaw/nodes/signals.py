"""Signal check node — mirrors ``check_signals`` in the agentic loop.

In the Rust implementation this checks for cancellation, user interrupt
messages injected via the scheduler channel, and stop requests.  In the
Python LangGraph version we read the ``signal`` field that was set by
the router node or by external callers (e.g. a REST endpoint).
"""

from __future__ import annotations

from typing import Any

from langchain_core.messages import HumanMessage

from ironclaw.nodes.llm_call import TOOL_INTENT_NUDGE
from ironclaw.state import AgentState


def check_signals_node(state: AgentState) -> dict[str, Any]:
    """
    Node: inspect ``state.signal`` and decide whether to continue or stop.

    Also handles the tool-intent nudge: when the LLM said "let me search…"
    without emitting a tool call, inject a TOOL_INTENT_NUDGE message and
    keep the loop going.
    """
    if state.signal == "stop":
        # Signal propagates; the router edge will direct to END
        return {}

    if state.last_response_type == "tool_intent_nudge":
        # Inject nudge message so the LLM tries again with a tool call
        return {
            "messages": [HumanMessage(content=TOOL_INTENT_NUDGE)],
            "last_response_type": "none",
        }

    return {}
