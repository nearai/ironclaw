#!/usr/bin/env python3
"""Convert Claude Code JSONL conversation history to IronClaw trace fixtures.

Takes a Claude Code session JSONL file and produces one or more IronClaw
trace fixture JSON files suitable for deterministic E2E replay via TraceLlm.

Usage:
    python3 scripts/convert_claude_trace.py <session.jsonl> [--output-dir <dir>]
    python3 scripts/convert_claude_trace.py <session.jsonl> --turn 2  # single turn
    python3 scripts/convert_claude_trace.py <session.jsonl> --list    # list turns
    python3 scripts/convert_claude_trace.py <session.jsonl> --all     # one file per turn

Tool mapping (Claude Code -> IronClaw):
    Bash   -> shell       (command -> command)
    Read   -> read_file   (file_path -> path, offset, limit)
    Write  -> write_file  (file_path -> path, content)
    Edit   -> apply_patch (file_path -> path, old_string, new_string, replace_all)
    Grep   -> grep        (pattern, path, glob, output_mode, context, ...)
    Glob   -> glob        (pattern, path)

Unmappable tools (Agent, ToolSearch, Skill, Task*, etc.) are skipped.
Turns that contain only unmappable tools are excluded.
"""

import argparse
import json
import os
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

# ── Tool mapping ──────────────────────────────────────────────────────

TOOL_MAP: dict[str, str] = {
    "Bash": "shell",
    "Read": "read_file",
    "Write": "write_file",
    "Edit": "apply_patch",
    "Grep": "grep",
    "Glob": "glob",
}

# Tools we silently skip (no IronClaw equivalent or not useful in replay)
SKIP_TOOLS: set[str] = {
    "Agent",
    "ToolSearch",
    "Skill",
    "TaskCreate",
    "TaskUpdate",
    "TaskGet",
    "TaskList",
    "TaskOutput",
    "TaskStop",
    "ScheduleWakeup",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "EnterWorktree",
    "ExitWorktree",
    "CronCreate",
    "CronDelete",
    "CronList",
    "NotebookEdit",
    "LSP",
    "Monitor",
    "RemoteTrigger",
    "WebSearch",
    "SendMessage",
}


def map_tool_args(claude_name: str, claude_input: dict[str, Any]) -> dict[str, Any] | None:
    """Map Claude Code tool arguments to IronClaw tool arguments.

    Returns None if the tool is unmappable.
    """
    ic_name = TOOL_MAP.get(claude_name)
    if ic_name is None:
        return None

    if ic_name == "shell":
        args: dict[str, Any] = {"command": claude_input.get("command", "")}
        if "timeout" in claude_input:
            # Claude Code uses milliseconds, IronClaw uses seconds
            timeout_ms = claude_input["timeout"]
            if isinstance(timeout_ms, (int, float)) and timeout_ms > 0:
                args["timeout"] = max(1, int(timeout_ms / 1000))
        return args

    if ic_name == "read_file":
        args = {"path": claude_input.get("file_path", "")}
        if "offset" in claude_input:
            args["offset"] = claude_input["offset"]
        if "limit" in claude_input:
            args["limit"] = claude_input["limit"]
        return args

    if ic_name == "write_file":
        return {
            "path": claude_input.get("file_path", ""),
            "content": claude_input.get("content", ""),
        }

    if ic_name == "apply_patch":
        args = {
            "path": claude_input.get("file_path", ""),
            "old_string": claude_input.get("old_string", ""),
            "new_string": claude_input.get("new_string", ""),
        }
        if claude_input.get("replace_all"):
            args["replace_all"] = True
        return args

    if ic_name == "grep":
        args = {"pattern": claude_input.get("pattern", "")}
        if "path" in claude_input:
            args["path"] = claude_input["path"]
        if "glob" in claude_input:
            args["glob"] = claude_input["glob"]
        if "output_mode" in claude_input:
            args["output_mode"] = claude_input["output_mode"]
        # Map context flags
        for flag in ("context", "-C", "-A", "-B"):
            if flag in claude_input:
                ic_flag = {
                    "context": "context",
                    "-C": "context",
                    "-A": "after_context",
                    "-B": "before_context",
                }.get(flag, flag)
                args[ic_flag] = claude_input[flag]
        if claude_input.get("-i"):
            args["case_insensitive"] = True
        if claude_input.get("-n") is False:
            args["line_numbers"] = False
        if "type" in claude_input:
            args["file_type"] = claude_input["type"]
        if "head_limit" in claude_input:
            args["max_results"] = claude_input["head_limit"]
        return args

    if ic_name == "glob":
        args = {"pattern": claude_input.get("pattern", "")}
        if "path" in claude_input:
            args["path"] = claude_input["path"]
        return args

    return None


def map_tool_call(
    claude_name: str, claude_input: dict[str, Any], call_id: str
) -> dict[str, Any] | None:
    """Map a single Claude Code tool_use to an IronClaw TraceToolCall.

    Returns None if the tool is unmappable.
    """
    ic_name = TOOL_MAP.get(claude_name)
    if ic_name is None:
        return None

    args = map_tool_args(claude_name, claude_input)
    if args is None:
        return None

    return {"id": call_id, "name": ic_name, "arguments": args}


# ── JSONL parsing ─────────────────────────────────────────────────────


def parse_jsonl(path: str) -> list[dict]:
    """Parse a Claude Code JSONL file into a list of message objects."""
    messages = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                messages.append(json.loads(line))
    return messages


def build_uuid_index(messages: list[dict]) -> dict[str, dict]:
    """Build a UUID -> message index."""
    idx = {}
    for msg in messages:
        uuid = msg.get("uuid")
        if uuid:
            idx[uuid] = msg
    return idx


def group_by_api_msg_id(messages: list[dict]) -> dict[str, list[dict]]:
    """Group assistant messages by their API message ID (streaming chunks)."""
    groups: dict[str, list[dict]] = defaultdict(list)
    for msg in messages:
        if msg.get("type") == "assistant":
            api_id = msg.get("message", {}).get("id", "")
            if api_id:
                groups[api_id].append(msg)
    return groups


# ── Conversation linearization ────────────────────────────────────────


def linearize_conversation(messages: list[dict]) -> list[dict]:
    """Extract the linear conversation from the JSONL stream.

    The JSONL file is already in temporal order. We just filter to
    user/assistant messages on the main chain (not sidechains) and
    skip metadata entries.
    """
    return [
        m
        for m in messages
        if m.get("type") in ("user", "assistant")
        and not m.get("isSidechain", False)
    ]


# ── LLM response grouping ────────────────────────────────────────────


class LlmResponse:
    """A single logical LLM response (may span multiple streaming chunks)."""

    def __init__(self):
        self.api_msg_id: str = ""
        self.thinking: list[str] = []
        self.text_blocks: list[str] = []
        self.tool_calls: list[dict] = []  # {name, input, id, uuid}
        self.stop_reason: str | None = None
        self.input_tokens: int = 0
        self.output_tokens: int = 0

    def add_chunk(self, msg: dict):
        api_msg = msg.get("message", {})
        self.api_msg_id = api_msg.get("id", self.api_msg_id)

        if api_msg.get("stop_reason"):
            self.stop_reason = api_msg["stop_reason"]

        usage = api_msg.get("usage", {})
        # Take the max token counts (they accumulate in streaming)
        self.input_tokens = max(
            self.input_tokens, usage.get("input_tokens", 0)
        )
        self.output_tokens = max(
            self.output_tokens, usage.get("output_tokens", 0)
        )

        for block in api_msg.get("content", []):
            if not isinstance(block, dict):
                continue
            btype = block.get("type")
            if btype == "thinking":
                text = block.get("thinking", "")
                if text:
                    self.thinking.append(text)
            elif btype == "text":
                text = block.get("text", "")
                if text:
                    self.text_blocks.append(text)
            elif btype == "tool_use":
                self.tool_calls.append(
                    {
                        "name": block.get("name", ""),
                        "input": block.get("input", {}),
                        "id": block.get("id", ""),
                        "uuid": msg.get("uuid", ""),
                    }
                )


def group_llm_responses(linear: list[dict]) -> list[dict]:
    """Process the linear message list into grouped LLM responses.

    Returns a new list where each assistant entry is a fully merged
    LlmResponse, and user messages are preserved as-is. This makes
    it easy to iterate and build trace steps.

    Streaming chunks of the same API response (same message.id) are
    merged even when tool_result messages are interleaved between them.
    This happens because Claude Code executes tool calls mid-stream and
    inserts tool results into the tree between streaming chunks.
    """
    result: list[dict] = []
    current_response: LlmResponse | None = None
    current_api_id: str | None = None
    # Buffer tool_results that arrive between streaming chunks of the
    # same API response.
    buffered_tool_results: list[dict] = []

    def flush_response():
        nonlocal current_response, current_api_id
        if current_response is not None:
            result.append({"_type": "llm_response", "_data": current_response})
            current_response = None
            current_api_id = None

    def flush_tool_results():
        nonlocal buffered_tool_results
        for tr in buffered_tool_results:
            result.append(tr)
        buffered_tool_results = []

    for msg in linear:
        if msg.get("type") == "assistant":
            api_id = msg.get("message", {}).get("id", "")
            if api_id == current_api_id and current_response is not None:
                # Continue the same streaming response — flush any
                # buffered tool results first (they belong between the
                # previous tool_calls step and this continuation).
                flush_tool_results()
                current_response.add_chunk(msg)
            else:
                # New API response — flush everything
                flush_response()
                flush_tool_results()
                current_response = LlmResponse()
                current_response.add_chunk(msg)
                current_api_id = api_id
        else:
            # User message — could be a tool_result between streaming
            # chunks or a genuine new user message.
            content = msg.get("message", {}).get("content", "")
            is_tool_result = isinstance(content, list) and any(
                isinstance(c, dict) and c.get("type") == "tool_result"
                for c in content
            )
            if is_tool_result and current_response is not None:
                # Buffer tool_result — might be mid-stream
                buffered_tool_results.append(msg)
            else:
                # Genuine user text message — flush everything
                flush_response()
                flush_tool_results()
                result.append(msg)

    flush_response()
    flush_tool_results()
    return result


# ── Turn extraction ──────────────────────────────────────────────────


def extract_tool_result(msg: dict) -> dict | None:
    """Extract tool result info from a user message containing tool_result."""
    content = msg.get("message", {}).get("content", [])
    if not isinstance(content, list):
        return None
    for block in content:
        if isinstance(block, dict) and block.get("type") == "tool_result":
            tool_use_result = msg.get("toolUseResult", {})
            if isinstance(tool_use_result, str):
                tool_use_result = {"stdout": tool_use_result}
            elif not isinstance(tool_use_result, dict):
                tool_use_result = {}
            return {
                "tool_use_id": block.get("tool_use_id", ""),
                "content": block.get("content", ""),
                "is_error": block.get("is_error", False),
                "stdout": tool_use_result.get("stdout", ""),
                "stderr": tool_use_result.get("stderr", ""),
            }
    return None


def extract_turns(messages: list[dict]) -> list[dict]:
    """Extract conversation turns from the linearized, grouped message list.

    Each turn starts with a user text message and contains the LLM response
    steps that follow until the next user text message.

    Returns: list of {user_input: str, steps: list[step]}
    """
    grouped = group_llm_responses(messages)

    turns: list[dict] = []
    current_turn: dict | None = None
    pending_tool_results: dict[str, dict] = {}  # tool_use_id -> result info

    for entry in grouped:
        if entry.get("_type") == "llm_response":
            if current_turn is None:
                continue  # Skip LLM responses before first user message

            resp: LlmResponse = entry["_data"]

            # Build expected_tool_results from any pending results
            expected_results = []
            for tool_use_id, result in pending_tool_results.items():
                expected_results.append(
                    {
                        "tool_call_id": tool_use_id,
                        "name": "",  # Will be filled below
                        "content": result.get("content", result.get("stdout", "")),
                    }
                )
            pending_tool_results.clear()

            # If response has tool calls, create a tool_calls step
            if resp.tool_calls:
                mapped_calls = []
                skipped_tool_ids: set[str] = set()
                for tc in resp.tool_calls:
                    mapped = map_tool_call(tc["name"], tc["input"], tc["id"])
                    if mapped is not None:
                        mapped_calls.append(mapped)
                    elif tc["name"] in SKIP_TOOLS or tc["name"] not in TOOL_MAP:
                        skipped_tool_ids.add(tc["id"])

                if mapped_calls:
                    step: dict[str, Any] = {
                        "response": {
                            "type": "tool_calls",
                            "tool_calls": mapped_calls,
                            "input_tokens": resp.input_tokens or 60,
                            "output_tokens": resp.output_tokens or 20,
                        }
                    }

                    # Add expected_tool_results if we have them
                    if expected_results:
                        # Try to fill in tool names from the mapped calls
                        call_id_to_name = {c["id"]: c["name"] for c in mapped_calls}
                        for er in expected_results:
                            if er["tool_call_id"] in call_id_to_name:
                                er["name"] = call_id_to_name[er["tool_call_id"]]
                        # Only include results for mapped tools
                        expected_results = [
                            er for er in expected_results if er["name"]
                        ]
                        if expected_results:
                            step["expected_tool_results"] = expected_results

                    current_turn["steps"].append(step)

            # If response has text, create a text step
            if resp.text_blocks:
                text = "\n".join(resp.text_blocks)
                step = {
                    "response": {
                        "type": "text",
                        "content": text,
                        "input_tokens": resp.input_tokens or 80,
                        "output_tokens": resp.output_tokens or 15,
                    }
                }
                if expected_results and not resp.tool_calls:
                    # Attach expected_tool_results to text step if no tool_calls step
                    step["expected_tool_results"] = [
                        er for er in expected_results if er["content"]
                    ]
                current_turn["steps"].append(step)

        elif entry.get("type") == "user":
            content = entry.get("message", {}).get("content", "")

            # Check if this is a tool_result message
            tool_result = extract_tool_result(entry)
            if tool_result is not None:
                pending_tool_results[tool_result["tool_use_id"]] = tool_result
                continue

            # This is a text user message — starts a new turn
            if isinstance(content, str) and content.strip():
                if current_turn is not None:
                    turns.append(current_turn)
                current_turn = {
                    "user_input": content.strip(),
                    "steps": [],
                    "timestamp": entry.get("timestamp", ""),
                    "git_branch": entry.get("gitBranch", ""),
                }

    # Flush last turn
    if current_turn is not None:
        turns.append(current_turn)

    return turns


# ── Trace generation ──────────────────────────────────────────────────


# ── Git commit resolution ─────────────────────────────────────────────


def detect_repo_root(messages: list[dict]) -> str | None:
    """Detect the original repo root from user message cwd or file paths.

    Looks at cwd fields and tool call arguments to find the common repo path.
    """
    candidates: list[str] = []
    for msg in messages:
        cwd = msg.get("cwd", "")
        if cwd:
            candidates.append(cwd)

    if not candidates:
        return None

    # The most common cwd is likely the repo root
    from collections import Counter
    most_common = Counter(candidates).most_common(1)[0][0]
    return most_common


def resolve_commit(
    repo_path: str, branch: str, timestamp: str
) -> str | None:
    """Resolve the git commit active on `branch` at `timestamp`.

    Tries local branch first, then origin/<branch>.
    """
    import subprocess

    for ref in [branch, f"origin/{branch}"]:
        result = subprocess.run(
            [
                "git",
                "-C",
                repo_path,
                "log",
                f"--before={timestamp}",
                "--format=%H",
                "-1",
                ref,
            ],
            capture_output=True,
            text=True,
        )
        commit = result.stdout.strip()
        if commit:
            return commit
    return None


def resolve_turn_commits(
    turns: list[dict], repo_path: str
) -> dict[str, str]:
    """Resolve git commits for each unique (branch, timestamp) pair.

    Returns a map of branch -> commit hash.
    """
    resolved: dict[str, str] = {}
    for turn in turns:
        branch = turn.get("git_branch", "")
        ts = turn.get("timestamp", "")
        if not branch or not ts or branch in resolved:
            continue
        commit = resolve_commit(repo_path, branch, ts)
        if commit:
            resolved[branch] = commit
    return resolved


# ── Path rewriting ────────────────────────────────────────────────────

# Placeholder used in trace fixtures for the repo root.
# At replay time, the test harness replaces this with the worktree path.
REPO_ROOT_PLACEHOLDER = "{{repo_root}}"


def rewrite_paths_in_value(value: Any, old_prefix: str) -> Any:
    """Recursively rewrite absolute paths in a JSON value.

    Replaces `old_prefix` with the placeholder `{{repo_root}}` so that
    the test harness can substitute the actual worktree path at runtime.
    """
    if isinstance(value, str):
        if old_prefix and old_prefix in value:
            return value.replace(old_prefix, REPO_ROOT_PLACEHOLDER)
        return value
    if isinstance(value, dict):
        return {k: rewrite_paths_in_value(v, old_prefix) for k, v in value.items()}
    if isinstance(value, list):
        return [rewrite_paths_in_value(v, old_prefix) for v in value]
    return value


def rewrite_paths_in_trace(trace: dict, repo_root: str):
    """Rewrite all absolute paths in a trace to use {{repo_root}} placeholder."""
    # Normalize: ensure no trailing slash
    repo_root = repo_root.rstrip("/")

    for turn in trace.get("turns", []):
        for step in turn.get("steps", []):
            resp = step.get("response", {})
            if resp.get("type") == "tool_calls":
                resp["tool_calls"] = [
                    {
                        **tc,
                        "arguments": rewrite_paths_in_value(
                            tc["arguments"], repo_root
                        ),
                    }
                    for tc in resp.get("tool_calls", [])
                ]
            if "expected_tool_results" in step:
                step["expected_tool_results"] = [
                    {**er, "content": er["content"].replace(repo_root, REPO_ROOT_PLACEHOLDER)}
                    if isinstance(er.get("content"), str)
                    else er
                    for er in step["expected_tool_results"]
                ]


def add_request_hints(turns: list[dict]):
    """Add request_hint to the first step of each turn."""
    for turn in turns:
        if not turn.get("steps"):
            continue
        user_input = turn.get("user_input", "")
        # Pick a distinctive substring from the user message
        hint_text = user_input[:80].strip()
        if hint_text:
            turn["steps"][0]["request_hint"] = {
                "last_user_message_contains": hint_text
            }


def build_expects(turn: dict) -> dict:
    """Build an expects block for a turn based on its steps."""
    expects: dict[str, Any] = {}
    tools_used: set[str] = set()

    for step in turn.get("steps", []):
        resp = step.get("response", {})
        if resp.get("type") == "tool_calls":
            for tc in resp.get("tool_calls", []):
                tools_used.add(tc["name"])

    if tools_used:
        expects["tools_used"] = sorted(tools_used)
        expects["all_tools_succeeded"] = True

    if turn.get("steps"):
        expects["min_responses"] = 1

    return expects


def generate_trace(
    turns: list[dict],
    model_name: str,
    session_id: str = "",
    commits: dict[str, str] | None = None,
    repo_root: str | None = None,
    rewrite_paths: bool = True,
) -> dict:
    """Generate an IronClaw trace fixture from extracted turns."""
    add_request_hints(turns)

    trace_turns = []
    for turn in turns:
        if not turn.get("steps"):
            continue  # Skip empty turns

        trace_turn: dict[str, Any] = {
            "user_input": turn["user_input"],
            "steps": turn["steps"],
        }

        expects = build_expects(turn)
        if expects:
            trace_turn["expects"] = expects

        trace_turns.append(trace_turn)

    if not trace_turns:
        return {}

    # Build top-level expects
    all_tools: set[str] = set()
    for turn in trace_turns:
        for step in turn.get("steps", []):
            resp = step.get("response", {})
            if resp.get("type") == "tool_calls":
                for tc in resp.get("tool_calls", []):
                    all_tools.add(tc["name"])

    trace: dict[str, Any] = {
        "model_name": model_name,
        "turns": trace_turns,
    }

    if all_tools:
        trace["expects"] = {
            "tools_used": sorted(all_tools),
            "all_tools_succeeded": True,
            "min_responses": 1,
        }

    # Embed repo metadata for worktree-based replay.
    # The test harness reads these to set up a git worktree at the right commit.
    if commits or repo_root:
        repo_meta: dict[str, Any] = {}
        if repo_root:
            repo_meta["repo_root"] = repo_root
        if commits:
            # Use the first turn's branch as primary commit
            first_branch = turns[0].get("git_branch", "")
            if first_branch and first_branch in commits:
                repo_meta["commit"] = commits[first_branch]
                repo_meta["branch"] = first_branch
            # Include all branch->commit mappings if multi-branch
            if len(commits) > 1:
                repo_meta["branch_commits"] = commits
        trace["repo"] = repo_meta

    # Rewrite absolute paths to {{repo_root}} placeholders
    if rewrite_paths and repo_root:
        rewrite_paths_in_trace(trace, repo_root)

    return trace


# ── CLI ───────────────────────────────────────────────────────────────


def list_turns(turns: list[dict]):
    """Print a summary of each turn."""
    for i, turn in enumerate(turns):
        user_msg = turn["user_input"][:80]
        n_steps = len(turn.get("steps", []))
        tools = set()
        for step in turn.get("steps", []):
            resp = step.get("response", {})
            if resp.get("type") == "tool_calls":
                for tc in resp.get("tool_calls", []):
                    tools.add(tc["name"])
        tool_str = ", ".join(sorted(tools)) if tools else "(text only)"
        branch = turn.get("git_branch", "")
        print(f"  Turn {i}: [{n_steps} steps] [{tool_str}] {user_msg}")
        if branch:
            print(f"          branch: {branch}")


def write_trace(trace: dict, output_path: str):
    """Write a trace fixture to a JSON file."""
    os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(trace, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(f"  Wrote {output_path} ({os.path.getsize(output_path) / 1024:.1f} KB)")


def sanitize_name(s: str) -> str:
    """Make a string safe for use in a filename."""
    return "".join(c if c.isalnum() or c in "-_" else "_" for c in s)[:60]


def main():
    parser = argparse.ArgumentParser(
        description="Convert Claude Code JSONL history to IronClaw trace fixtures"
    )
    parser.add_argument("input", help="Path to Claude Code .jsonl session file")
    parser.add_argument(
        "--output-dir",
        "-o",
        default="tests/fixtures/llm_traces/imported",
        help="Output directory for trace files (default: tests/fixtures/llm_traces/imported)",
    )
    parser.add_argument(
        "--list", "-l", action="store_true", help="List turns without converting"
    )
    parser.add_argument(
        "--turn", "-t", type=int, help="Convert only this turn number"
    )
    parser.add_argument(
        "--all", "-a", action="store_true", help="Generate one trace file per turn"
    )
    parser.add_argument(
        "--model-name",
        "-m",
        help="Model name prefix for traces (default: imported-<session_id>)",
    )
    parser.add_argument(
        "--include-expected-results",
        action="store_true",
        help="Include expected_tool_results for regression checking",
    )
    parser.add_argument(
        "--deterministic-only",
        "-d",
        action="store_true",
        help="Skip turns with non-deterministic tools (shell commands that depend on external state)",
    )
    parser.add_argument(
        "--repo",
        help="Path to the original git repo for commit resolution and path rewriting "
        "(default: auto-detect from cwd in JSONL)",
    )
    parser.add_argument(
        "--no-rewrite-paths",
        action="store_true",
        help="Keep absolute paths as-is (don't rewrite to {{repo_root}} placeholders)",
    )

    args = parser.parse_args()

    if not os.path.exists(args.input):
        print(f"Error: {args.input} not found", file=sys.stderr)
        sys.exit(1)

    # Parse session ID from filename
    session_id = Path(args.input).stem

    print(f"Parsing {args.input}...")
    messages = parse_jsonl(args.input)
    print(f"  {len(messages)} raw messages")

    # Linearize the conversation tree
    linear = linearize_conversation(messages)
    print(f"  {len(linear)} linearized messages (excluding sidechains)")

    # Extract turns
    turns = extract_turns(linear)
    print(f"  {len(turns)} turns extracted")

    # Filter out turns with no mappable steps
    nonempty_turns = [t for t in turns if t.get("steps")]
    skipped = len(turns) - len(nonempty_turns)
    if skipped:
        print(f"  {skipped} turns skipped (no mappable tool calls)")
    turns = nonempty_turns

    if not turns:
        print("No convertible turns found.", file=sys.stderr)
        sys.exit(1)

    # Detect repo root and resolve commits
    repo_root = args.repo or detect_repo_root(messages)
    commits: dict[str, str] = {}
    if repo_root and os.path.isdir(os.path.join(repo_root, ".git")):
        commits = resolve_turn_commits(turns, repo_root)
        if commits:
            print(f"  Resolved commits:")
            for branch, commit_hash in commits.items():
                print(f"    {branch} -> {commit_hash[:12]}")
    elif repo_root:
        print(f"  Warning: {repo_root} is not a git repo, skipping commit resolution")

    # Remove internal metadata from turns before output
    if not args.include_expected_results:
        for turn in turns:
            for step in turn.get("steps", []):
                step.pop("expected_tool_results", None)

    if args.list:
        list_turns(turns)
        return

    model_prefix = args.model_name or f"imported-{session_id[:8]}"

    if args.turn is not None:
        if args.turn < 0 or args.turn >= len(turns):
            print(
                f"Error: turn {args.turn} out of range (0-{len(turns) - 1})",
                file=sys.stderr,
            )
            sys.exit(1)

        turn = turns[args.turn]
        model_name = f"{model_prefix}-turn{args.turn}"
        trace = generate_trace(
            [turn], model_name, session_id,
            commits=commits, repo_root=repo_root,
            rewrite_paths=not args.no_rewrite_paths,
        )
        if trace:
            fname = f"{sanitize_name(model_name)}.json"
            write_trace(trace, os.path.join(args.output_dir, fname))
        return

    if args.all:
        for i, turn in enumerate(turns):
            model_name = f"{model_prefix}-turn{i}"
            trace = generate_trace(
                [turn], model_name, session_id,
                commits=commits, repo_root=repo_root,
                rewrite_paths=not args.no_rewrite_paths,
            )
            if trace:
                fname = f"{sanitize_name(model_name)}.json"
                write_trace(trace, os.path.join(args.output_dir, fname))
        return

    # Default: all turns in one file
    model_name = model_prefix
    trace = generate_trace(
        turns, model_name, session_id,
        commits=commits, repo_root=repo_root,
        rewrite_paths=not args.no_rewrite_paths,
    )
    if trace:
        # Clean up internal metadata from turns
        for turn in trace.get("turns", []):
            turn.pop("timestamp", None)
            turn.pop("git_branch", None)

        fname = f"{sanitize_name(model_name)}.json"
        write_trace(trace, os.path.join(args.output_dir, fname))


if __name__ == "__main__":
    main()
