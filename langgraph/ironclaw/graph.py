"""
IronClaw LangGraph agent graph.

Maps the Rust ``run_agentic_loop`` + ``Agent::run`` into a LangGraph
``StateGraph``.  The three execution modes from the Rust implementation
(chat, job, container) collapse into a single, configurable graph here —
mode-specific behaviour is expressed through ``AgentDeps`` rather than
separate ``LoopDelegate`` implementations.

Graph topology
--------------

                ┌──────────────┐
   START ──────►│ route_input  │
                └──────┬───────┘
                       │ signal==stop → END
                       ▼
                ┌──────────────┐
           ┌───►│check_signals │
           │    └──────┬───────┘
           │           │ stop → END
           │           ▼
           │    ┌──────────────┐
           │    │   call_llm   │
           │    └──────┬───────┘
           │           │
           │    ┌──────▼───────────────────┐
           │    │    route_llm_response    │
           │    └──┬───────────┬────────── ┘
           │  text │     tools │   nudge │  max_iter
           │       │           │         │
           │       ▼           ▼         ▼
           │      END   ┌────────────┐ loop back
           │            │exec_tools  │
           │            └──────┬─────┘
           │                   │ need_approval → END (wait)
           └───────────────────┘  otherwise loop back
"""

from __future__ import annotations

import functools
from typing import Any, Literal

from langgraph.graph import END, START, StateGraph
from langgraph.checkpoint.memory import MemorySaver

from ironclaw.config import AgentConfig
from ironclaw.nodes.router import route_input_node
from ironclaw.nodes.signals import check_signals_node
from ironclaw.nodes.llm_call import call_llm_node
from ironclaw.nodes.tool_exec import execute_tools_node
from ironclaw.safety.layer import SafetyLayer
from ironclaw.state import AgentState
from ironclaw.tools.registry import ToolRegistry


# ---------------------------------------------------------------------------
# Routing helpers (conditional edges)
# ---------------------------------------------------------------------------


def _after_route_input(state: AgentState) -> Literal["check_signals", "__end__"]:
    """After parsing input: stop if a control command set signal=stop."""
    if state.signal == "stop":
        return END
    return "check_signals"


def _after_check_signals(state: AgentState) -> Literal["call_llm", "__end__"]:
    if state.signal == "stop":
        return END
    return "call_llm"


def _after_call_llm(
    state: AgentState,
    max_iterations: int = 50,
) -> Literal["check_signals", "execute_tools", "__end__"]:
    """Route after the LLM call based on response type."""
    if state.iteration >= max_iterations:
        return END
    if state.last_response_type == "text":
        return END
    if state.last_response_type == "tool_calls":
        return "execute_tools"
    # nudge or none — go back to signal check which will inject nudge msg
    return "check_signals"


def _after_execute_tools(
    state: AgentState,
) -> Literal["check_signals", "__end__"]:
    """After tool execution: loop back unless approval is needed."""
    if state.last_response_type == "need_approval":
        return END
    if state.signal == "stop":
        return END
    return "check_signals"


# ---------------------------------------------------------------------------
# Graph builder
# ---------------------------------------------------------------------------


class AgentDeps:
    """
    Runtime dependencies injected into graph nodes.

    Mirrors ``AgentDeps`` from the Rust implementation.  All three
    execution modes (chat, job, container) share these same deps.
    """

    def __init__(
        self,
        llm: Any,
        tool_registry: ToolRegistry,
        safety: SafetyLayer,
        config: AgentConfig,
    ) -> None:
        self.llm = llm
        self.tool_registry = tool_registry
        self.safety = safety
        self.config = config


def build_agent_graph(deps: AgentDeps) -> Any:
    """
    Build and compile the IronClaw agent StateGraph.

    Returns a compiled LangGraph graph ready to be invoked with::

        graph.ainvoke({"messages": [HumanMessage(content="hello")]},
                      config={"configurable": {"thread_id": "abc"}})

    The ``MemorySaver`` checkpointer provides per-thread state persistence
    analogous to the Rust ``SessionManager`` + ``UndoManager``.
    """
    builder = StateGraph(AgentState)

    # ── Nodes ────────────────────────────────────────────────────────────────

    builder.add_node("route_input", route_input_node)
    builder.add_node("check_signals", check_signals_node)

    # LLM node — partial-apply deps
    async def _call_llm(state: AgentState) -> dict[str, Any]:
        return await call_llm_node(state, deps.llm)

    builder.add_node("call_llm", _call_llm)

    # Tool execution node — partial-apply deps
    async def _execute_tools(state: AgentState) -> dict[str, Any]:
        return await execute_tools_node(
            state,
            deps.tool_registry,
            deps.safety,
            auto_approve=deps.config.auto_approve_tools,
        )

    builder.add_node("execute_tools", _execute_tools)

    # ── Edges ─────────────────────────────────────────────────────────────────

    builder.add_edge(START, "route_input")

    builder.add_conditional_edges(
        "route_input",
        _after_route_input,
        {"check_signals": "check_signals", END: END},
    )

    builder.add_conditional_edges(
        "check_signals",
        _after_check_signals,
        {"call_llm": "call_llm", END: END},
    )

    after_llm = functools.partial(
        _after_call_llm,
        max_iterations=deps.config.max_iterations,
    )
    builder.add_conditional_edges(
        "call_llm",
        after_llm,
        {
            "check_signals": "check_signals",
            "execute_tools": "execute_tools",
            END: END,
        },
    )

    builder.add_conditional_edges(
        "execute_tools",
        _after_execute_tools,
        {"check_signals": "check_signals", END: END},
    )

    # ── Compile ───────────────────────────────────────────────────────────────

    checkpointer = MemorySaver()
    return builder.compile(checkpointer=checkpointer)
