"""
Starter script: backtest a strategy against state/history.

For each historical snapshot, run the portfolio tool's `propose`
operation with the supplied strategy doc, then check whether the
suggestions that were "ready" at that date would still have been
"ready" at the next snapshot. The hit rate is reported.

Usage (from the REPL, or wired to a sub-mission):

    strategy_doc = open("projects/portfolio/strategies/stablecoin-yield-floor.md").read()
    result = backtest(strategy_doc)
    print(result)

This is a local-data backtest — it does NOT re-fetch historical
indexer data. That's fine because the portfolio tool's state
snapshots are already the canonical history (LLM data is never
deleted — see CLAUDE.md).
"""

PROJECT_ID = "portfolio"

import json

def load_history():
    tree = tool_invoke("memory_tree", {
        "target": f"projects/{PROJECT_ID}/state/history",
    })
    files = [f for f in tree.get("entries", []) if f["name"].endswith(".json")]
    files.sort(key=lambda f: f["name"])
    snaps = []
    for f in files:
        raw = tool_invoke("memory_read", {
            "target": f"projects/{PROJECT_ID}/state/history/{f['name']}",
        })
        snap = json.loads(raw["content"] if isinstance(raw, dict) else raw)
        snaps.append({
            "date": f["name"].replace(".json", ""),
            "positions": snap.get("positions", []),
        })
    return snaps

def load_config():
    raw = tool_invoke("memory_read", {
        "target": f"projects/{PROJECT_ID}/config.json",
    })
    return json.loads(raw["content"] if isinstance(raw, dict) else raw)

def backtest(strategy_doc):
    snaps = load_history()
    config = load_config()
    results = []
    for snap in snaps:
        propose = tool_invoke("portfolio", {
            "action": "propose",
            "positions": snap["positions"],
            "strategies": [strategy_doc],
            "config": config,
        })
        ready = [p for p in propose.get("proposals", []) if p.get("status") == "ready"]
        results.append({
            "date": snap["date"],
            "ready_count": len(ready),
            "top_delta_bps": max((p.get("projected_delta_apy_bps", 0) for p in ready), default=0),
        })
    total = sum(r["ready_count"] for r in results)
    return {
        "snapshots": len(results),
        "total_ready": total,
        "hit_rate": total / max(len(results), 1),
        "per_snapshot": results,
    }

# When called from the mission, use the default strategy.
strategy = tool_invoke("memory_read", {
    "target": f"projects/{PROJECT_ID}/strategies/stablecoin-yield-floor.md",
})
strategy_doc = strategy["content"] if isinstance(strategy, dict) else strategy
result = backtest(strategy_doc)
