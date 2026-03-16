"""LLM call node — mirrors the ``call_llm`` step in the agentic loop.

Handles:
* Building the chat messages list for the provider
* Tool definitions injection
* Token usage tracking
* Tool-intent nudge detection (``llm_signals_tool_intent``)
* Force-text mode
"""

from __future__ import annotations

import re
from typing import Any

from langchain_core.messages import AIMessage, AnyMessage, SystemMessage

from ironclaw.state import AgentState

# Phrases indicating the LLM is about to call a tool but forgot to emit
# the tool_call JSON.  Mirror ``TOOL_INTENT_NUDGE`` / ``llm_signals_tool_intent``
# from the Rust implementation.
_TOOL_INTENT_PATTERNS: list[re.Pattern[str]] = [
    re.compile(p, re.IGNORECASE)
    for p in [
        r"\blet me (search|look|find|check|fetch|get|run|execute)\b",
        r"\bi('ll| will) (search|look|find|check|fetch|get|run|execute)\b",
        r"\bsearching\b",
        r"\blooking up\b",
        r"\bfetching\b",
        r"\brunning\b",
        r"\bexecuting\b",
    ]
]

TOOL_INTENT_NUDGE = (
    "It looks like you intended to call a tool but you did not include any tool calls "
    "in your response. Please respond with the appropriate tool call now."
)


def llm_signals_tool_intent(text: str) -> bool:
    """Return True if the LLM response suggests it wants to call a tool."""
    return any(p.search(text) for p in _TOOL_INTENT_PATTERNS)


async def call_llm_node(state: AgentState, llm: Any) -> dict[str, Any]:
    """
    Node: call the LLM with the current message history and available tools.

    Returns state updates:
    * ``messages``       — append the AI response
    * ``last_response_type``  — "text" or "tool_calls"
    * ``last_text_response``  — if text, the response string
    * ``iteration``      — incremented by 1
    * ``total_input_tokens`` / ``total_output_tokens`` — updated if provider exposes usage
    * ``consecutive_tool_intent_nudges`` — incremented when nudge fires

    Parameters
    ----------
    state
        Current graph state.
    llm
        A LangChain chat model instance (ChatAnthropic, ChatOpenAI, etc.).
    """
    messages: list[AnyMessage] = list(state.messages)

    # Inject system prompt if not already present
    if state.system_prompt and not any(isinstance(m, SystemMessage) for m in messages):
        messages = [SystemMessage(content=state.system_prompt)] + messages

    # Bind tools if available and not force_text
    if state.available_tools and not state.force_text:
        tools_json = [
            {
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                },
            }
            for t in state.available_tools
        ]
        bound_llm = llm.bind_tools(tools_json)
    else:
        bound_llm = llm

    # Invoke
    response: AIMessage = await bound_llm.ainvoke(messages)

    updates: dict[str, Any] = {
        "iteration": state.iteration + 1,
        "messages": [response],
    }

    # Token usage (LangChain surfaces this via response_metadata)
    meta = response.response_metadata or {}
    usage = meta.get("usage", meta.get("token_usage", {}))
    if usage:
        updates["total_input_tokens"] = state.total_input_tokens + usage.get(
            "input_tokens", usage.get("prompt_tokens", 0)
        )
        updates["total_output_tokens"] = state.total_output_tokens + usage.get(
            "output_tokens", usage.get("completion_tokens", 0)
        )

    # Determine response type
    if response.tool_calls:
        updates["last_response_type"] = "tool_calls"
        updates["consecutive_tool_intent_nudges"] = 0
    else:
        text = response.content if isinstance(response.content, str) else ""
        updates["last_text_response"] = text

        # Tool-intent nudge
        if (
            state.available_tools
            and not state.force_text
            and state.consecutive_tool_intent_nudges < 2  # max_tool_intent_nudges
            and llm_signals_tool_intent(text)
        ):
            updates["last_response_type"] = "tool_intent_nudge"
            updates["consecutive_tool_intent_nudges"] = state.consecutive_tool_intent_nudges + 1
        else:
            updates["last_response_type"] = "text"
            updates["consecutive_tool_intent_nudges"] = 0

    return updates
