#!/usr/bin/env python3
"""Convert a downloaded Reborn run artifact into a reviewable LLM trace candidate."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
from collections import OrderedDict
from typing import Any

SCHEMA = "ironclaw.run_artifact.v1"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Convert a redacted run artifact into an LLM trace candidate. "
            "Review assertions and external-service determinism before committing."
        )
    )
    parser.add_argument("artifact", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument(
        "--model-name",
        help="Override the fixture model name (defaults to the captured provider model).",
    )
    return parser.parse_args()


def load_artifact(path: pathlib.Path) -> dict[str, Any]:
    try:
        artifact = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"could not read artifact {path}: {error}") from error
    if artifact.get("schema") != SCHEMA:
        raise ValueError(f"unsupported artifact schema: {artifact.get('schema')!r}")
    if artifact.get("redaction", {}).get("pipeline") != "deterministic-trace-redactor-v1":
        raise ValueError("artifact does not declare the required deterministic redaction pipeline")
    messages = artifact.get("messages")
    if not isinstance(messages, list) or not messages:
        raise ValueError("artifact has no run messages")
    return artifact


def trace_candidate(artifact: dict[str, Any], model_override: str | None) -> dict[str, Any]:
    messages = sorted(artifact["messages"], key=lambda item: item.get("sequence", 0))
    user = next((item for item in messages if item.get("kind") == "user"), None)
    if not user or not str(user.get("content", "")).strip():
        raise ValueError("artifact has no replayable user message")

    tool_groups: OrderedDict[str, list[dict[str, Any]]] = OrderedDict()
    for message in messages:
        tool_call = message.get("tool_call")
        if not isinstance(tool_call, dict):
            continue
        provider_turn_id = str(tool_call.get("provider_turn_id") or message.get("sequence"))
        tool_groups.setdefault(provider_turn_id, []).append(message)

    assistant = next(
        (
            item
            for item in reversed(messages)
            if item.get("kind") == "assistant" and str(item.get("content", "")).strip()
        ),
        None,
    )
    steps: list[dict[str, Any]] = []
    pending_results: list[dict[str, Any]] = []
    captured_models: list[str] = []

    for group in tool_groups.values():
        calls: list[dict[str, Any]] = []
        next_results: list[dict[str, Any]] = []
        for message in group:
            call = message["tool_call"]
            model = str(call.get("provider_model_id") or "").strip()
            if model:
                captured_models.append(model)
            calls.append(
                {
                    "id": call["provider_call_id"],
                    "name": call["capability_id"],
                    "arguments": call.get("arguments", {}),
                }
            )
            next_results.append(
                {
                    "tool_call_id": call["provider_call_id"],
                    "name": call["capability_id"],
                    "content": message.get("content", ""),
                }
            )
        step: dict[str, Any] = {
            "response": {
                "type": "tool_calls",
                "tool_calls": calls,
                "input_tokens": 0,
                "output_tokens": 0,
            }
        }
        if pending_results:
            step["expected_tool_results"] = pending_results
        steps.append(step)
        pending_results = next_results

    if assistant:
        step = {
            "response": {
                "type": "text",
                "content": assistant["content"],
                "input_tokens": 0,
                "output_tokens": 0,
            }
        }
        if pending_results:
            step["expected_tool_results"] = pending_results
        steps.append(step)
    elif pending_results:
        raise ValueError("artifact ends with tool results but no finalized assistant response")

    if not steps:
        raise ValueError("artifact has neither tool calls nor a finalized assistant response")

    model_name = model_override or (captured_models[0] if captured_models else "reborn-qa-import")
    tools = list(
        OrderedDict.fromkeys(
            call["name"]
            for step in steps
            for call in step["response"].get("tool_calls", [])
        )
    )
    return {
        "_review": {
            "status": "candidate",
            "source_schema": SCHEMA,
            "source_run_id": artifact.get("run", {}).get("run_id"),
            "logs_complete": artifact.get("logs", {}).get("complete", False),
            "required_actions": [
                "Add scenario-specific expects and caller-level end-state assertions.",
                "Review every redaction placeholder for acceptable fixture fidelity.",
                "Record or mock external HTTP/service exchanges before enabling hermetic CI replay.",
            ],
        },
        "model_name": model_name,
        "turns": [
            {
                "user_input": user["content"],
                "steps": steps,
                "expects": {"tools_used": tools} if tools else {},
            }
        ],
    }


def main() -> int:
    args = parse_args()
    try:
        candidate = trace_candidate(load_artifact(args.artifact), args.model_name)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(candidate, indent=2) + "\n", encoding="utf-8")
    except (ValueError, KeyError, TypeError, AttributeError) as error:
        print(error, file=sys.stderr)
        return 2
    print(f"wrote review-required fixture candidate: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
