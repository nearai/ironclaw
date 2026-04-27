---
id: near-lending-yield
version: 1
kind: yield-floor
applies_to:
  category: wallet
  chains: ["near"]
  tokens: ["USDC", "USDt", "USDT", "DAI", "NEAR", "wNEAR", "stNEAR", "LiNEAR"]
  min_principal_usd: 10
constraints:
  min_projected_delta_apy_bps: 50
  max_risk_score: 3
  gas_payback_days: 14
  prefer_same_chain: true
inputs:
  floor_apy: 0.01
---

# NEAR Lending Yield (Rhea / Burrow)

Idle tokens in a NEAR wallet can be supplied to Rhea (formerly Burrow)
to earn lending APY. Stablecoins (USDC, USDt) typically earn 2-8% APY.
NEAR and liquid staking derivatives (stNEAR, LiNEAR) can also be
supplied as collateral.

Rhea lending is instant-withdrawal (no lockup), making it low-friction
for idle assets.

When ranking:

1. Stablecoins → Rhea lending is nearly always the right move (low risk).
2. NEAR → consider staking first (higher APY), Rhea lending second.
3. Liquid staking tokens (stNEAR, LiNEAR) → can be supplied to Rhea
   for additional yield on top of staking rewards.
4. Monitor health factor if borrowing against supplied collateral.
