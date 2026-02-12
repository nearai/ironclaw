#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="$ROOT_DIR/interface/data"
ARTIFACT_DIR="$ROOT_DIR/artifacts"
mkdir -p "$DATA_DIR" "$ARTIFACT_DIR"

TEMPLATE_CONFIG="$ROOT_DIR/config/openclaw.nuro.safe.template.json5"
HANDOFF_PATH="/Users/nuro/Documents/dev/omin/sync/LATEST_HANDOFF.md"
WORKLOG_PATH="/Users/nuro/Documents/dev/omin/sync/worklog.jsonl"

STATUS_OUT="$ARTIFACT_DIR/openclaw_status_latest.txt"
DEEP_OUT="$ARTIFACT_DIR/openclaw_status_deep_latest.txt"

if command -v openclaw >/dev/null 2>&1; then
  OPENCLAW_PRESENT=1
  (openclaw status || true) > "$STATUS_OUT" 2>&1
  (openclaw status --deep || true) > "$DEEP_OUT" 2>&1
else
  OPENCLAW_PRESENT=0
  echo "openclaw CLI not found" > "$STATUS_OUT"
  echo "openclaw CLI not found" > "$DEEP_OUT"
fi

LATEST_PREFLIGHT=""
if ls "$ARTIFACT_DIR"/preflight_*.log >/dev/null 2>&1; then
  LATEST_PREFLIGHT="$(ls -1t "$ARTIFACT_DIR"/preflight_*.log | head -n 1)"
fi

export ROOT_DIR DATA_DIR TEMPLATE_CONFIG HANDOFF_PATH WORKLOG_PATH STATUS_OUT DEEP_OUT LATEST_PREFLIGHT OPENCLAW_PRESENT
python3 <<'PY'
from __future__ import annotations
import json
import os
import re
from datetime import datetime, timezone
from pathlib import Path


def read_text(path: str) -> str:
    p = Path(path)
    return p.read_text() if p.exists() else ""


def tail_lines(path: str, n: int) -> list[str]:
    txt = read_text(path)
    if not txt:
        return []
    lines = txt.splitlines()
    return lines[-n:]


def extract(text: str, pattern: str, default: str = "unknown") -> str:
    m = re.search(pattern, text)
    return m.group(1) if m else default


root_dir = Path(os.environ["ROOT_DIR"])
data_dir = Path(os.environ["DATA_DIR"])
template_path = Path(os.environ["TEMPLATE_CONFIG"])
handoff_path = Path(os.environ["HANDOFF_PATH"])
worklog_path = Path(os.environ["WORKLOG_PATH"])
status_out = Path(os.environ["STATUS_OUT"])
deep_out = Path(os.environ["DEEP_OUT"])
preflight = os.environ.get("LATEST_PREFLIGHT", "")
openclaw_present = os.environ.get("OPENCLAW_PRESENT", "0") == "1"

cfg_text = read_text(str(template_path))

config_snapshot = {
    "gateway_mode": extract(cfg_text, r"gateway\s*:\s*\{[\s\S]*?mode:\s*\"([^\"]+)\"", "unknown"),
    "dm_scope": extract(cfg_text, r"dmScope:\s*\"([^\"]+)\"", "unknown"),
    "sandbox_mode": extract(cfg_text, r"sandbox:\s*\{[\s\S]*?mode:\s*\"([^\"]+)\"", "unknown"),
    "group_policy": extract(cfg_text, r"groupPolicy:\s*\"([^\"]+)\"", "unknown"),
    "elevated_enabled": extract(cfg_text, r"elevated:\s*\{[\s\S]*?enabled:\s*(true|false)", "unknown"),
}

handoff_text = read_text(str(handoff_path))
handoff_title = extract(handoff_text, r"- `title`: (.+)", "unknown")
handoff_summary = extract(handoff_text, r"- `summary`: (.+)", "unknown")
next_actions = re.findall(r"^- \[ \] (.+)$", handoff_text, flags=re.MULTILINE)

worklog_events = []
if worklog_path.exists():
    for raw in worklog_path.read_text().splitlines()[-12:]:
        try:
            entry = json.loads(raw)
        except Exception:
            continue
        ts = entry.get("timestamp_utc") or entry.get("ts") or "unknown"
        agent = entry.get("agent", "unknown")
        title = entry.get("title") or entry.get("summary") or entry.get("event", "event")
        worklog_events.append({"ts": ts, "agent": agent, "title": title})

status_preview = tail_lines(str(status_out), 24)
deep_status_preview = tail_lines(str(deep_out), 24)

preflight_tail = tail_lines(preflight, 60) if preflight else ["preflight log not found. run: bash ironclaw/scripts/preflight_hardening.sh"]

checks = [
    {
        "label": "OpenClaw CLI",
        "status": "ok" if openclaw_present else "warn",
        "detail": "available" if openclaw_present else "not installed in PATH",
    },
    {
        "label": "DM scope isolation",
        "status": "ok" if config_snapshot["dm_scope"] == "per-channel-peer" else "warn",
        "detail": f"dmScope={config_snapshot['dm_scope']}",
    },
    {
        "label": "Sandbox posture",
        "status": "ok" if config_snapshot["sandbox_mode"] == "non-main" else "warn",
        "detail": f"sandbox.mode={config_snapshot['sandbox_mode']}",
    },
    {
        "label": "Group policy",
        "status": "ok" if config_snapshot["group_policy"] == "allowlist" else "warn",
        "detail": f"groupPolicy={config_snapshot['group_policy']}",
    },
    {
        "label": "Elevated exec",
        "status": "ok" if config_snapshot["elevated_enabled"] == "false" else "bad",
        "detail": f"tools.elevated.enabled={config_snapshot['elevated_enabled']}",
    },
]

artifacts = [
    {"label": "OpenClaw video transcript (SRT)", "path": str(root_dir / "research/matthewberman_openclaw_full_transcript.srt")},
    {"label": "OpenClaw video transcript (TXT)", "path": str(root_dir / "research/matthewberman_openclaw_full_transcript.txt")},
    {"label": "Synthesis", "path": str(root_dir / "research/synthesis_and_implementation.md")},
    {"label": "Safe template config", "path": str(root_dir / "config/openclaw.nuro.safe.template.json5")},
]

quick_commands = [
    {"label": "refresh interface data", "command": "bash /Users/nuro/Documents/dev/ironclaw/nuro_agent/scripts/refresh_nuro_interface_data.sh"},
    {"label": "run hardening preflight", "command": "bash /Users/nuro/Documents/dev/ironclaw/nuro_agent/scripts/preflight_hardening.sh"},
    {"label": "run hardening with auto-fix", "command": "bash /Users/nuro/Documents/dev/ironclaw/nuro_agent/scripts/preflight_hardening.sh --fix"},
    {"label": "openclaw status", "command": "openclaw status --all"},
    {"label": "security audit", "command": "openclaw security audit --deep"},
    {"label": "start local interface", "command": "bash /Users/nuro/Documents/dev/ironclaw/nuro_agent/scripts/run_nuro_interface.sh"},
]

out = {
    "generated_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "profile": {"name": "nuro", "mission": "free the mind", "realm": "nuro realm"},
    "config_snapshot": config_snapshot,
    "runtime": {
        "openclaw_cli_present": openclaw_present,
        "status_preview": status_preview,
        "deep_status_preview": deep_status_preview,
        "preflight_log_path": preflight,
        "preflight_log_tail": preflight_tail,
    },
    "checks": checks,
    "handoff": {
        "title": handoff_title,
        "summary": handoff_summary,
        "next_actions": next_actions,
    },
    "events": worklog_events,
    "quick_commands": quick_commands,
    "artifacts": artifacts,
}

(data_dir / "status.json").write_text(json.dumps(out, indent=2) + "\n")
print(str(data_dir / "status.json"))
PY

echo "snapshot refreshed"
