---
id: hyperliquid-funding-carry-watch
version: 1
kind: signal-hyperliquid-funding
venue: hyperliquid
status: signal-only
timeframes: ["1h", "8h", "1d"]
data_sources:
  - hyperliquid_funding
  - hyperliquid_all_mids
  - spot_reference_price
parameters:
  min_abs_funding_apr_pct: 12
  min_observation_hours: 24
  max_premium_bps: 75
risk:
  no_perp_execution_in_v0: true
  require_spot_liquidity_check: true
  require_funding_reversal_check: true
---

# Hyperliquid Funding Carry Watch

Signal-only strategy for identifying when perp funding is extreme
enough to matter for spot positioning. The agent may use this as
context for a spot rotation thesis, but v0 must not open perp positions
or claim carry execution through NEAR Intents.

Use the signal when funding is persistent across multiple observations,
the perp premium is not already mean-reverting violently, and spot
liquidity is deep enough for the candidate intent route.

Reject the signal if funding flipped recently, market depth is thin,
oracle/mark divergence is unstable, or the proposed trade only works
because of leverage.
