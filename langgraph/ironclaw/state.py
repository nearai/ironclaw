"""Agent state — the single source of truth flowing through the LangGraph graph.

Maps to the Rust side's `ReasoningContext` + session/thread state.
"""

from __future__ import annotations

from typing import Annotated, Any, Literal
from uuid import uuid4

from langchain_core.messages import AnyMessage
from langgraph.graph.message import add_messages
from pydantic import BaseModel, Field


# ---------------------------------------------------------------------------
# Supporting types
# ---------------------------------------------------------------------------


class ToolDefinition(BaseModel):
    """A tool the LLM may call."""

    name: str
    description: str
    parameters: dict[str, Any]


class PendingApproval(BaseModel):
    """A tool call awaiting user approval."""

    tool_name: str
    tool_call_id: str
    parameters: dict[str, Any]
    requires_always: bool = False


class JobStatus(BaseModel):
    """Status of a background job."""

    job_id: str
    title: str
    description: str
    state: Literal["pending", "in_progress", "completed", "failed", "cancelled", "stuck"]
    created_at: str = ""
    updated_at: str = ""
    error: str | None = None


# ---------------------------------------------------------------------------
# Signals (mirror LoopSignal from Rust)
# ---------------------------------------------------------------------------

LoopSignal = Literal["continue", "stop", "interrupt"]


# ---------------------------------------------------------------------------
# Main agent state
# ---------------------------------------------------------------------------


class AgentState(BaseModel):
    """
    Mutable state passed through every node of the LangGraph agent graph.

    Design notes
    ------------
    * ``messages`` uses LangGraph's ``add_messages`` reducer so appends are
      safe even when multiple nodes run concurrently.
    * All other fields are replaced wholesale on each update.
    * This mirrors ``ReasoningContext`` (messages, available_tools, force_text)
      plus the broader session/thread envelope.
    """

    # --- conversation history (LangGraph managed) ---
    messages: Annotated[list[AnyMessage], add_messages] = Field(default_factory=list)

    # --- identity ---
    thread_id: str = Field(default_factory=lambda: str(uuid4()))
    session_id: str = Field(default_factory=lambda: str(uuid4()))
    user_id: str = "default"
    channel: str = "repl"

    # --- loop control ---
    signal: LoopSignal = "continue"
    iteration: int = 0
    consecutive_tool_intent_nudges: int = 0
    force_text: bool = False  # force a text-only response on next LLM call

    # --- tools ---
    available_tools: list[ToolDefinition] = Field(default_factory=list)

    # --- approval ---
    pending_approval: PendingApproval | None = None

    # --- background jobs ---
    active_jobs: list[JobStatus] = Field(default_factory=list)

    # --- last LLM response (for routing) ---
    last_response_type: Literal["text", "tool_calls", "none"] = "none"
    last_text_response: str = ""

    # --- context compaction ---
    needs_compaction: bool = False
    system_prompt: str = ""

    # --- cost tracking ---
    total_input_tokens: int = 0
    total_output_tokens: int = 0

    class Config:
        arbitrary_types_allowed = True
