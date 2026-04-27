---
id: near-staking-yield
version: 1
kind: yield-floor
applies_to:
  category: wallet
  chains: ["near"]
  min_principal_usd: 50
constraints:
  min_projected_delta_apy_bps: 100
  max_risk_score: 2
  gas_payback_days: 7
  prefer_same_chain: true
inputs:
  floor_apy: 0.01
---

# NEAR Staking Yield

Idle NEAR tokens sitting in a wallet earn 0% APY. Propose liquid staking
via Linear Protocol (~8-10% APY) or Meta Pool (stNEAR, ~8-9% APY) for
any NEAR wallet position above `min_principal_usd`.

Liquid staking tokens (LiNEAR, stNEAR) are tradable and usable as
collateral on Rhea (Burrow), so the user retains liquidity while
earning staking rewards.

When ranking:

1. Prefer Linear or Meta Pool based on current APY.
2. Consider withdrawal delay (~48h for unstaking).
3. NEAR staking is low-risk (risk_score 1) — always propose if idle.
