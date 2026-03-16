"""
Application startup — mirrors src/app.rs.

Wires together: config → LLM → tools → safety → graph → channels → run loop.
"""

from __future__ import annotations

import asyncio
import logging
from typing import Any

from langchain_core.messages import AIMessage, HumanMessage

from ironclaw.config import Config
from ironclaw.graph import AgentDeps, build_agent_graph
from ironclaw.safety.layer import SafetyLayer
from ironclaw.scheduler.scheduler import JobScheduler
from ironclaw.state import AgentState, ToolDefinition
from ironclaw.tools.registry import ToolRegistry
from ironclaw.tools.builtin import (
    echo_tool,
    read_file_tool,
    write_file_tool,
    list_dir_tool,
    http_get_tool,
    http_post_tool,
    memory_search_tool,
    memory_write_tool,
    memory_read_tool,
    shell_tool,
    time_tool,
)
from ironclaw.channels.channel import OutgoingResponse
from ironclaw.channels.repl import ReplChannel

logger = logging.getLogger(__name__)


def _build_llm(config: Config) -> Any:
    """Instantiate the LLM from config.  Mirrors src/llm/ provider selection."""
    backend = config.llm.backend
    model = config.llm.model
    temperature = config.llm.temperature
    max_tokens = config.llm.max_tokens

    if backend == "anthropic":
        from langchain_anthropic import ChatAnthropic
        kwargs: dict[str, Any] = {
            "model": model,
            "temperature": temperature,
            "max_tokens": max_tokens,
        }
        if config.llm.api_key:
            kwargs["anthropic_api_key"] = config.llm.api_key
        return ChatAnthropic(**kwargs)

    if backend == "openai":
        from langchain_openai import ChatOpenAI
        kwargs = {
            "model": model,
            "temperature": temperature,
            "max_tokens": max_tokens,
        }
        if config.llm.api_key:
            kwargs["openai_api_key"] = config.llm.api_key
        return ChatOpenAI(**kwargs)

    if backend in ("openai_compatible", "ollama"):
        from langchain_openai import ChatOpenAI
        kwargs = {
            "model": model,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "base_url": config.llm.base_url or "http://localhost:11434/v1",
            "api_key": config.llm.api_key or "ollama",
        }
        return ChatOpenAI(**kwargs)

    raise ValueError(f"Unknown LLM backend: {backend!r}")


def _build_tool_registry(config: Config) -> ToolRegistry:
    """Register built-in tools.  Mirrors src/tools/builtin/mod.rs."""
    registry = ToolRegistry()
    registry.register(echo_tool)
    registry.register(time_tool)
    registry.register(memory_search_tool)
    registry.register(memory_write_tool)
    registry.register(memory_read_tool)

    if config.agent.allow_local_tools:
        registry.register(read_file_tool)
        registry.register(write_file_tool)
        registry.register(list_dir_tool)
        registry.register(http_get_tool)
        registry.register(http_post_tool)
        registry.register(shell_tool)

    return registry


def _tool_defs_from_registry(registry: ToolRegistry) -> list[ToolDefinition]:
    """Convert registry tools to state ToolDefinition objects."""
    return [
        ToolDefinition(
            name=t.name,
            description=t.description,
            parameters=t.parameters_schema,
        )
        for t in registry.list_tools()
    ]


async def _run_repl(graph: Any, registry: ToolRegistry, config: Config) -> None:
    """Interactive REPL loop — mirrors src/channels/repl.rs."""
    channel = ReplChannel()
    tool_defs = _tool_defs_from_registry(registry)

    print(f"\nIronClaw (LangGraph) — type your message, /quit to exit\n")

    async for msg in channel.receive():
        if msg.content in ("/quit", "/exit"):
            print("Goodbye.")
            break

        state_input: dict[str, Any] = {
            "messages": [HumanMessage(content=msg.content)],
            "user_id": msg.user_id,
            "available_tools": tool_defs,
        }
        graph_config = {"configurable": {"thread_id": msg.thread_id}}

        try:
            result = await graph.ainvoke(state_input, config=graph_config)
        except Exception as exc:  # noqa: BLE001
            logger.exception("Graph invocation error")
            await channel.send(OutgoingResponse(
                content=f"Error: {exc}",
                thread_id=msg.thread_id,
            ))
            continue

        # Extract last AI message
        messages = result.get("messages", [])
        last_ai = next(
            (m for m in reversed(messages) if isinstance(m, AIMessage)),
            None,
        )
        response_text = last_ai.content if last_ai else "(no response)"
        if isinstance(response_text, list):
            # Handle content blocks (Anthropic format)
            response_text = " ".join(
                block.get("text", "") if isinstance(block, dict) else str(block)
                for block in response_text
            )

        await channel.send(OutgoingResponse(
            content=response_text,
            thread_id=msg.thread_id,
        ))


class IronclawApp:
    """
    Top-level application.  Mirrors the Rust ``App`` struct.

    Usage::

        app = IronclawApp(config)
        await app.run()
    """

    def __init__(self, config: Config | None = None) -> None:
        self.config = config or Config.load()
        self.llm = _build_llm(self.config)
        self.tool_registry = _build_tool_registry(self.config)
        self.safety = SafetyLayer(
            injection_check_enabled=self.config.safety.injection_check_enabled,
            max_output_length=self.config.safety.max_output_length,
        )
        self.deps = AgentDeps(
            llm=self.llm,
            tool_registry=self.tool_registry,
            safety=self.safety,
            config=self.config.agent,
        )
        self.graph = build_agent_graph(self.deps)
        self.scheduler = JobScheduler(
            max_parallel_jobs=self.config.agent.max_parallel_jobs
        )

    async def run(self) -> None:
        """Start the application.  Default mode: interactive REPL."""
        await _run_repl(self.graph, self.tool_registry, self.config)
