---
id: lp-impermanent-loss-watch
version: 1
kind: lp-impermanent-loss-watch
applies_to:
  category: dex-lp
constraints:
  max_risk_score: 5
---

# LP Impermanent-Loss Watch

Fire a `ready` proposal for any concentrated-liquidity LP position
whose `in_range` metadata flag is `false`. An out-of-range LP earns
zero fees while still accruing impermanent loss — it's worse than
just holding the underlying tokens.

The proposal suggests **closing and reopening** the LP around the
current price. This strategy does not project annual yield; the
value comes from stopping the bleed.

When ranking, prefer:

1. LPs that have been out of range the longest.
2. LPs whose notional is largest (bigger IL drag).
3. LPs on low-gas chains first.
