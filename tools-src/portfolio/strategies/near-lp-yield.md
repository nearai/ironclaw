---
id: near-lp-yield
version: 1
kind: yield-floor
applies_to:
  category: wallet
  chains: ["near"]
  min_principal_usd: 100
constraints:
  min_projected_delta_apy_bps: 200
  max_risk_score: 3
  gas_payback_days: 14
  prefer_same_chain: true
inputs:
  floor_apy: 0.01
---

# NEAR LP Yield (Rhea / Ref Finance)

For larger idle positions, providing liquidity on Rhea (Ref Finance DEX)
can earn trading fees + farming rewards. Common pairs:

- NEAR/USDC — moderate IL risk, good volume
- NEAR/stNEAR — low IL (correlated assets)
- Stablecoin pairs (USDC/USDt) — minimal IL

LP positions carry impermanent loss risk, so this strategy requires a
higher `min_projected_delta_apy_bps` (200 bps = 2%) to justify the
added complexity.

When ranking:

1. Correlated pairs (NEAR/stNEAR, USDC/USDt) over uncorrelated ones.
2. Higher TVL pools over low-liquidity ones.
3. Consider concentrated liquidity (DCL v2) for better capital efficiency
   but higher management overhead.
4. Flag any LP position where in_range = false for rebalancing.
