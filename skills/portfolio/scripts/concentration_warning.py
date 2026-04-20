"""
Starter script: warn if the portfolio is over-concentrated on one
chain or protocol.

Defaults: flag when any chain or protocol holds > MAX_FRACTION of
total net value. Meant to run as a sub-mission on the same cadence
as the keeper (or a slower one — concentration doesn't change fast).
"""

PROJECT_ID = "portfolio"
MAX_FRACTION = 0.60   # 60% in one chain or protocol is too much

import json

raw = tool_invoke("memory_read", {
    "target": f"projects/{PROJECT_ID}/state/latest.json",
})
state = json.loads(raw["content"] if isinstance(raw, dict) else raw)

positions = state.get("positions", [])
total = 0.0
by_chain = {}
by_protocol = {}
for p in positions:
    principal = float(p.get("principal_usd", "0") or 0)
    total += principal
    by_chain[p["chain"]] = by_chain.get(p["chain"], 0) + principal
    pid = p["protocol"]["id"]
    by_protocol[pid] = by_protocol.get(pid, 0) + principal

warnings = []
if total > 0:
    for chain, value in by_chain.items():
        frac = value / total
        if frac > MAX_FRACTION:
            warnings.append(f"{frac * 100:.0f}% of net value concentrated on **{chain}**")
    for protocol, value in by_protocol.items():
        frac = value / total
        if frac > MAX_FRACTION:
            warnings.append(f"{frac * 100:.0f}% of net value concentrated in **{protocol}**")

if warnings:
    text = "Portfolio concentration warning:\n- " + "\n- ".join(warnings)
    tool_invoke("message_send", {"channel": "default", "text": text})

result = {
    "total_usd": total,
    "by_chain": by_chain,
    "by_protocol": by_protocol,
    "warnings": warnings,
}
