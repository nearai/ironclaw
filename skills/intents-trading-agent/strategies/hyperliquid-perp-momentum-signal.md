---
id: hyperliquid-perp-momentum-signal
version: 1
kind: signal-hyperliquid-momentum
venue: hyperliquid
status: signal-only
timeframes: ["15m", "1h", "4h"]
data_sources:
  - hyperliquid_candle_snapshot
  - hyperliquid_l2_book
  - hyperliquid_all_mids
parameters:
  lookback_window: 20
  min_momentum_bps: 250
  max_chase_distance_bps: 150
risk:
  require_spot_backtest_confirmation: true
  require_book_gate: true
  require_news_veto_check: true
---

# Hyperliquid Perp Momentum Signal

Signal-only strategy for using Hyperliquid perp market structure as an
early warning that a liquid asset may be entering a momentum regime. In
v0, this can only support a spot intent candidate; it cannot authorize
perp orders, leverage, or API-key execution.

Promote the signal only when momentum persists beyond the lookback
window, the current price is not too far from the breakout level, and a
matching spot strategy also passes `portfolio.backtest_suite`.

Reject or downgrade when the move is already extended, book depth is
weak, funding is crowded, or the catalyst is unverifiable.
