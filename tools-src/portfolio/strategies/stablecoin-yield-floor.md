---
id: stablecoin-yield-floor
version: 1
applies_to:
  category: stablecoin-idle
  min_principal_usd: 100
constraints:
  min_projected_delta_apy_bps: 50
  max_risk_score: 3
  max_bridge_legs: 1
  gas_payback_days: 30
  prefer_same_chain: true
  prefer_near_intents: true
inputs:
  floor_apy: 0.04
---

# Stablecoin Yield Floor

Keep idle stablecoins at or above `floor_apy` net APY. For any qualifying
position yielding below that, propose the highest-net-APY alternative
within the risk budget, preferring same-chain moves, then single-bridge
moves via NEAR Intents.

Do not propose a move whose gas + bridge + slippage cost would take more
than `gas_payback_days` to recoup at the Δ APY.

When ranking candidates the LLM should weight:

1. Net Δ APY (higher is better).
2. Same-chain over cross-chain (avoid bridge complexity).
3. Lower exit cost.
4. Longer-standing protocols (higher TVL, more audits) over new ones.
5. Lower risk score delta (do not blindly chase yield).
