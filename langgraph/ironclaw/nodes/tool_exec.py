"""Tool execution node — mirrors ``execute_tool_calls`` in dispatcher.rs.

Processes all tool_calls emitted by the LLM:
1. Checks approval requirement
2. Runs safety layer on output
3. Appends ToolMessage results back into messages
4. Returns ``pending_approval`` if a tool requires user approval
"""

from __future__ import annotations

import json
from typing import Any

from langchain_core.messages import AIMessage, ToolMessage

from ironclaw.safety.layer import SafetyLayer
from ironclaw.state import AgentState, PendingApproval
from ironclaw.tools.registry import ToolRegistry


async def execute_tools_node(
    state: AgentState,
    tool_registry: ToolRegistry,
    safety: SafetyLayer,
    auto_approve: bool = False,
) -> dict[str, Any]:
    """
    Node: execute tool calls returned by the LLM.

    Returns state updates:
    * ``messages``         — append ToolMessage results
    * ``pending_approval`` — set if a tool requires approval
    * ``last_response_type`` — reset to "none" so the loop continues
    """
    # Find the most recent AIMessage with tool_calls
    last_ai: AIMessage | None = None
    for msg in reversed(state.messages):
        if isinstance(msg, AIMessage) and msg.tool_calls:
            last_ai = msg
            break

    if last_ai is None:
        return {"last_response_type": "none"}

    tool_messages: list[ToolMessage] = []

    for call in last_ai.tool_calls:
        tool_name = call["name"]
        tool_call_id = call["id"]
        raw_args = call.get("args", {})
        params = raw_args if isinstance(raw_args, dict) else {}

        tool_def = tool_registry.get(tool_name)

        # --- Approval check ---
        if tool_def is not None and tool_def.requires_approval and not auto_approve:
            # Pause loop for user approval
            pending = PendingApproval(
                tool_name=tool_name,
                tool_call_id=tool_call_id,
                parameters=params,
                requires_always=True,
            )
            return {
                "pending_approval": pending,
                "last_response_type": "need_approval",
            }

        # --- Execute ---
        result = await tool_registry.execute(tool_name, params)

        # --- Safety: sanitize output ---
        if tool_def is not None and tool_def.requires_sanitization:
            sanitized = safety.sanitize_tool_output(result.output)
        else:
            sanitized = result.output

        # --- Build ToolMessage ---
        tool_msg = ToolMessage(
            content=sanitized,
            tool_call_id=tool_call_id,
            name=tool_name,
        )
        if result.error:
            tool_msg.status = "error"  # type: ignore[attr-defined]

        tool_messages.append(tool_msg)

    updates: dict[str, Any] = {
        "messages": tool_messages,
        "last_response_type": "none",
        "pending_approval": None,
    }
    return updates
