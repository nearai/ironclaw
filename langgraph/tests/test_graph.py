"""Tests for the LangGraph agent graph (uses a mock LLM)."""

from __future__ import annotations

import pytest
from langchain_core.messages import AIMessage, HumanMessage

from ironclaw.config import AgentConfig
from ironclaw.graph import AgentDeps, build_agent_graph
from ironclaw.safety.layer import SafetyLayer
from ironclaw.state import AgentState, ToolDefinition
from ironclaw.tools.registry import ToolRegistry
from ironclaw.tools.builtin.echo import echo_tool


class _MockLLM:
    """Minimal LLM stub that returns a fixed text response."""

    def __init__(self, response: str = "Hello from stub LLM") -> None:
        self._response = response

    def bind_tools(self, tools):
        return self  # ignore tools for mock

    async def ainvoke(self, messages):
        return AIMessage(content=self._response)


def _make_deps(llm=None, auto_approve=False) -> AgentDeps:
    registry = ToolRegistry()
    registry.register(echo_tool)
    safety = SafetyLayer()
    config = AgentConfig(auto_approve_tools=auto_approve, max_iterations=10)
    return AgentDeps(
        llm=llm or _MockLLM(),
        tool_registry=registry,
        safety=safety,
        config=config,
    )


@pytest.mark.asyncio
async def test_simple_chat_response():
    deps = _make_deps(_MockLLM("Hi there!"))
    graph = build_agent_graph(deps)

    result = await graph.ainvoke(
        {"messages": [HumanMessage(content="Hello")]},
        config={"configurable": {"thread_id": "test-1"}},
    )
    messages = result["messages"]
    assert any(isinstance(m, AIMessage) and "Hi there!" in m.content for m in messages)


@pytest.mark.asyncio
async def test_quit_command_stops_loop():
    deps = _make_deps(_MockLLM("should not appear"))
    graph = build_agent_graph(deps)

    result = await graph.ainvoke(
        {"messages": [HumanMessage(content="/quit")]},
        config={"configurable": {"thread_id": "test-quit"}},
    )
    # Graph should exit without calling LLM (signal=stop from router)
    messages = result["messages"]
    ai_messages = [m for m in messages if isinstance(m, AIMessage)]
    # No AI response expected because the signal=stop before LLM call
    assert len(ai_messages) == 0


@pytest.mark.asyncio
async def test_interrupt_command_stops_loop():
    deps = _make_deps()
    graph = build_agent_graph(deps)

    result = await graph.ainvoke(
        {"messages": [HumanMessage(content="/interrupt")]},
        config={"configurable": {"thread_id": "test-interrupt"}},
    )
    # signal=stop should prevent LLM call
    ai_messages = [m for m in result["messages"] if isinstance(m, AIMessage)]
    assert len(ai_messages) == 0


@pytest.mark.asyncio
async def test_tool_call_executes_and_loops():
    """LLM returns a tool call → execute_tools runs → loop returns to LLM → text response."""

    call_count = [0]

    class _ToolCallThenTextLLM:
        """First invocation returns echo tool call; second returns text."""

        def bind_tools(self, tools):
            return self

        async def ainvoke(self, messages):
            call_count[0] += 1
            if call_count[0] == 1:
                # First call: emit a tool call
                return AIMessage(
                    content="",
                    tool_calls=[{
                        "id": "call_1",
                        "name": "echo",
                        "args": {"text": "ping"},
                        "type": "tool_call",
                    }],
                )
            # Second call: return text
            return AIMessage(content="Echo returned: ping")

    deps = _make_deps(_ToolCallThenTextLLM(), auto_approve=True)
    graph = build_agent_graph(deps)

    result = await graph.ainvoke(
        {
            "messages": [HumanMessage(content="echo ping")],
            "available_tools": [
                ToolDefinition(
                    name="echo",
                    description="Echo text",
                    parameters={"type": "object", "properties": {}},
                )
            ],
        },
        config={"configurable": {"thread_id": "test-tool-call"}},
    )
    assert call_count[0] == 2
    messages = result["messages"]
    final_ai = next(
        (m for m in reversed(messages) if isinstance(m, AIMessage) and m.content),
        None,
    )
    assert final_ai is not None
    assert "Echo returned" in final_ai.content
