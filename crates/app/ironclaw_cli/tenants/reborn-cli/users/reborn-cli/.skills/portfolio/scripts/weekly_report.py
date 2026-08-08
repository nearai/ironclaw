"""
Starter script: generate a 7-day portfolio report from state/history.

Computes:
  - 7-day weighted net APY via the portfolio tool's `progress` op
  - Total value change
  - Protocol-level attribution (where the yield came from)

Writes the report to projects/<id>/reports/<YYYY-MM-DD>.md and
posts a summary to the default channel.
"""

PROJECT_ID = "portfolio"

import json

# Load the last 7 history snapshots.
history = []
tree = tool_invoke("memory_tree", {
    "target": f"projects/{PROJECT_ID}/state/history",
})
files = [f for f in tree.get("entries", []) if f["name"].endswith(".json")]
files.sort(key=lambda f: f["name"], reverse=True)
for f in files[:7]:
    raw = tool_invoke("memory_read", {
        "target": f"projects/{PROJECT_ID}/state/history/{f['name']}",
    })
    snap = json.loads(raw["content"]) if isinstance(raw, dict) else json.loads(raw)
    history.append({
        "date": f["name"].replace(".json", ""),
        "positions": snap.get("positions", []),
    })

# Load config.
config_raw = tool_invoke("memory_read", {
    "target": f"projects/{PROJECT_ID}/config.json",
})
config = json.loads(config_raw["content"] if isinstance(config_raw, dict) else config_raw)

# Compute the progress metric via the portfolio tool.
progress = tool_invoke("portfolio", {
    "action": "progress",
    "history": history,
    "config": config,
})

# Build the markdown report.
lines = [
    "# Weekly portfolio report",
    "",
    f"- **Samples**: {progress['samples']}",
    f"- **Realized net APY (7d)**: {progress['realized_net_apy_7d'] * 100:.2f}%",
    f"- **Floor APY**: {progress['floor_apy'] * 100:.2f}%",
    f"- **Δ vs floor**: {progress['delta_vs_floor'] * 100:+.2f}%",
    f"- **Avg total value**: ${progress['average_total_value_usd']}",
    f"- **Progress score**: {progress['progress_score']:+.3f}",
]
md = "\n".join(lines)

# Persist.
from datetime import date
today = date.today().isoformat()
tool_invoke("memory_write", {
    "target": f"projects/{PROJECT_ID}/reports/{today}.md",
    "content": md,
})

# Post a short summary.
summary = (
    f"Weekly portfolio report ({today}): realized APY "
    f"{progress['realized_net_apy_7d'] * 100:.2f}% vs floor "
    f"{progress['floor_apy'] * 100:.2f}%."
)
tool_invoke("message_send", {"channel": "default", "text": summary})

result = progress
