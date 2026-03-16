"""Tests for builtin tools and the tool registry."""

import pytest
from ironclaw.tools.registry import ToolRegistry
from ironclaw.tools.builtin.echo import echo_tool
from ironclaw.tools.builtin.time_tool import time_tool


@pytest.mark.asyncio
async def test_echo_tool():
    result = await echo_tool.execute({"text": "hello"}, None)
    assert result.output == "hello"
    assert not result.error


@pytest.mark.asyncio
async def test_echo_tool_empty():
    result = await echo_tool.execute({}, None)
    assert result.output == ""
    assert not result.error


@pytest.mark.asyncio
async def test_time_tool_utc():
    result = await time_tool.execute({"timezone": "UTC"}, None)
    assert not result.error
    assert "T" in result.output  # ISO 8601 format


@pytest.mark.asyncio
async def test_time_tool_invalid_timezone():
    result = await time_tool.execute({"timezone": "Invalid/Zone"}, None)
    assert result.error


@pytest.mark.asyncio
async def test_registry_execute_unknown_tool():
    registry = ToolRegistry()
    result = await registry.execute("nonexistent", {})
    assert result.error
    assert "not found" in result.output.lower()


@pytest.mark.asyncio
async def test_registry_register_and_execute():
    registry = ToolRegistry()
    registry.register(echo_tool)
    result = await registry.execute("echo", {"text": "world"})
    assert result.output == "world"
    assert not result.error


def test_registry_to_llm_format():
    registry = ToolRegistry()
    registry.register(echo_tool)
    tools = registry.to_llm_format()
    assert len(tools) == 1
    assert tools[0]["function"]["name"] == "echo"
