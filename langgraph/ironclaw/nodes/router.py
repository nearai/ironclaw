"""Router node — mirrors src/agent/router.rs + submission.rs.

Parses the latest user message into a typed ``Submission`` before the
main agentic loop runs, just like the Rust ``SubmissionParser``.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from enum import Enum
from typing import Any

from langchain_core.messages import HumanMessage

from ironclaw.state import AgentState


# ---------------------------------------------------------------------------
# Submission types (mirror Rust Submission enum)
# ---------------------------------------------------------------------------


class SubmissionKind(str, Enum):
    Undo = "undo"
    Redo = "redo"
    Interrupt = "interrupt"
    Compact = "compact"
    Clear = "clear"
    Heartbeat = "heartbeat"
    NewThread = "new_thread"
    SwitchThread = "switch_thread"
    JobStatus = "job_status"
    JobCancel = "job_cancel"
    ApprovalResponse = "approval_response"
    SystemCommand = "system_command"
    UserInput = "user_input"
    CreateJob = "create_job"


@dataclass
class Submission:
    kind: SubmissionKind
    args: dict[str, Any] = field(default_factory=dict)


# ---------------------------------------------------------------------------
# Parser
# ---------------------------------------------------------------------------

_APPROVAL_YES = re.compile(r"^(yes|y|approve|ok|always|a)$", re.IGNORECASE)
_APPROVAL_NO = re.compile(r"^(no|n|deny|reject|cancel)$", re.IGNORECASE)


def parse_submission(text: str) -> Submission:
    """
    Parse a raw user message into a typed ``Submission``.

    Mirrors ``SubmissionParser::parse()`` from Rust.
    """
    stripped = text.strip()

    # Control commands
    if stripped in ("/undo",):
        return Submission(SubmissionKind.Undo)
    if stripped in ("/redo",):
        return Submission(SubmissionKind.Redo)
    if stripped in ("/interrupt", "/stop"):
        return Submission(SubmissionKind.Interrupt)
    if stripped in ("/compact",):
        return Submission(SubmissionKind.Compact)
    if stripped in ("/clear",):
        return Submission(SubmissionKind.Clear)
    if stripped in ("/heartbeat",):
        return Submission(SubmissionKind.Heartbeat)
    if stripped in ("/new", "/thread new"):
        return Submission(SubmissionKind.NewThread)
    if stripped in ("/quit", "/exit", "/shutdown"):
        return Submission(SubmissionKind.SystemCommand, {"command": "quit"})

    # Job commands
    if stripped.startswith("/job ") or stripped.startswith("/create "):
        rest = stripped.split(None, 1)[1] if " " in stripped else ""
        return Submission(SubmissionKind.CreateJob, {"description": rest})
    if stripped.startswith(("/status", "/progress", "/list")):
        parts = stripped.split()
        job_id = parts[1] if len(parts) > 1 else None
        return Submission(SubmissionKind.JobStatus, {"job_id": job_id})
    if stripped.startswith("/cancel "):
        job_id = stripped.split(None, 1)[1]
        return Submission(SubmissionKind.JobCancel, {"job_id": job_id})

    # Thread switch
    m = re.match(r"^/thread ([0-9a-f-]{36})$", stripped, re.IGNORECASE)
    if m:
        return Submission(SubmissionKind.SwitchThread, {"thread_id": m.group(1)})
    m = re.match(r"^/resume ([0-9a-f-]{36})$", stripped, re.IGNORECASE)
    if m:
        return Submission(SubmissionKind.SwitchThread, {"thread_id": m.group(1)})

    # Approval responses
    if _APPROVAL_YES.match(stripped):
        always = stripped.lower() in ("always", "a")
        return Submission(SubmissionKind.ApprovalResponse, {"approved": True, "always": always})
    if _APPROVAL_NO.match(stripped):
        return Submission(SubmissionKind.ApprovalResponse, {"approved": False})

    # System commands
    if stripped.startswith("/"):
        parts = stripped[1:].split()
        cmd = parts[0].lower() if parts else ""
        args = parts[1:]
        return Submission(SubmissionKind.SystemCommand, {"command": cmd, "args": args})

    # Default — natural language user input
    return Submission(SubmissionKind.UserInput, {"content": stripped})


# ---------------------------------------------------------------------------
# Router node
# ---------------------------------------------------------------------------


def route_input_node(state: AgentState) -> dict[str, Any]:
    """
    Parse the last user message into a Submission and tag the state.

    This node runs first, before the LLM is called.  It sets
    ``signal = 'stop'`` for control commands (quit, interrupt) so
    the graph can exit cleanly.
    """
    # Find the last human message
    last_human = next(
        (m for m in reversed(state.messages) if isinstance(m, HumanMessage)),
        None,
    )
    if last_human is None:
        return {}

    content = last_human.content if isinstance(last_human.content, str) else ""
    submission = parse_submission(content)

    updates: dict[str, Any] = {}

    if submission.kind == SubmissionKind.Interrupt:
        updates["signal"] = "stop"
    elif submission.kind == SubmissionKind.SystemCommand:
        if submission.args.get("command") == "quit":
            updates["signal"] = "stop"

    return updates
