"""
Starter script: alert when any lending position's health factor drops below a threshold.

Copies itself to projects/<id>/scripts/alert_if_health_below.py on the
user's first "alert me when health…" request. The script reads the
latest state snapshot, checks every position with a `health` field,
and emits a message on the default channel if any value is below
THRESHOLD.

Meant to be called from the keeper mission or as a sub-mission of
its own. Written in the Monty-friendly subset (no imports beyond
what the IronClaw engine already exposes; no filesystem).

Configure by editing THRESHOLD and PROJECT_ID at the top.
"""

PROJECT_ID = "portfolio"
THRESHOLD = 1.5   # health-factor value below which to alert

# Read the latest state snapshot from the project workspace.
state_raw = tool_invoke("memory_read", {
    "target": f"projects/{PROJECT_ID}/state/latest.json",
})
import json  # Monty tolerates json; this is the one stdlib module used.
state = json.loads(state_raw["content"]) if isinstance(state_raw, dict) else json.loads(state_raw)

alerts = []
for p in state.get("positions", []):
    h = p.get("health")
    if not h:
        continue
    value = h.get("value")
    if value is None:
        continue
    if value < THRESHOLD:
        alerts.append({
            "protocol": p["protocol"]["name"],
            "chain": p["chain"],
            "metric": h["name"],
            "value": value,
        })

if alerts:
    lines = [f"**Portfolio health alert** — {len(alerts)} position(s) below {THRESHOLD}:"]
    for a in alerts:
        lines.append(f"- {a['protocol']} on {a['chain']}: {a['metric']} = {a['value']:.2f}")
    tool_invoke("message_send", {
        "channel": "default",
        "text": "\n".join(lines),
    })

# Return value is what appears in the mission's step result.
result = {"alerts": alerts, "checked": len(state.get("positions", []))}
