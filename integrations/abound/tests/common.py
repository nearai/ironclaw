"""Shared utilities for abound wire-send test pipelines."""

import json
import re

import requests

RESP_PAT = re.compile(r"^resp_([0-9a-f]{32})([0-9a-f]{32})$")
CHOICE_PAT = re.compile(r"\[\[choice_set\]\](.*?)\[\[/choice_set\]\]", re.DOTALL)


def agent_text(response) -> str:
    return " ".join(
        c.text
        for item in response.output if item.type == "message"
        for c in item.content if c.type == "output_text"
    )


def get_tool_calls(response) -> list[dict]:
    calls = []
    for item in response.output:
        if item.type == "function_call":
            try:
                args = json.loads(item.arguments)
            except Exception:
                args = {}
            calls.append({"name": item.name, "args": args, "call_id": getattr(item, "call_id", None)})
        elif item.type == "function_call_output":
            for c in calls:
                if c.get("call_id") == getattr(item, "call_id", None):
                    c["output"] = getattr(item, "output", None)
    return calls


def format_tool_calls(calls: list[dict], prefix: str) -> list[str]:
    lines = []
    for c in calls:
        action = c["args"].get("action", "")
        label = f"{c['name']}({action})" if action else c["name"]
        lines.append(f"{prefix}  call:   {label}")
        args_display = {k: v for k, v in c["args"].items() if k != "action"}
        if args_display:
            lines.append(f"{prefix}  args:   {json.dumps(args_display)}")
        if c.get("output"):
            out = c["output"]
            if isinstance(out, str) and len(out) > 300:
                out = out[:300] + "..."
            lines.append(f"{prefix}  output: {out}")
    return lines


def first_choice_prompt(text: str) -> str | None:
    for m in CHOICE_PAT.finditer(text):
        try:
            data = json.loads(m.group(1).strip())
            items = data.get("items", [])
            if items:
                return items[0].get("prompt")
        except Exception:
            pass
    return None


def fetch_account_info(read_token: str, api_key: str) -> dict:
    resp = requests.get(
        "https://devneobank.timesclub.co/times/bank/remittance/agent/account/info",
        headers={
            "Authorization": f"Bearer {read_token}",
            "X-API-KEY": api_key,
            "device-type": "WEB",
        },
        timeout=30,
    )
    resp.raise_for_status()
    return resp.json()["data"]
