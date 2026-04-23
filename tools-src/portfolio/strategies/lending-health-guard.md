---
id: lending-health-guard
version: 1
kind: health-guard
applies_to:
  category: lending
constraints:
  max_risk_score: 5
inputs:
  danger_threshold: 1.3
  critical_threshold: 1.1
---

# Lending Health Guard

Watch the `health_factor` (or protocol-equivalent) on any borrowing
position. Fire a `ready` proposal suggesting a partial deleverage
whenever the metric drops below `danger_threshold`, and escalate the
severity to `critical` below `critical_threshold`.

This strategy does not project yield — its job is defense, not
offense. Rank high in the LLM's final order regardless of Δ APY.

When ranking the output the LLM should:

1. Execute critical warnings immediately (suggest signing in the
   same turn if the user is present).
2. Treat non-critical warnings as notifications — the user can
   schedule the rebalance within 24 hours.
3. Prefer repaying the smallest debt that restores health over
   more aggressive unwinds.
