---
id: breakout-spot
version: 1
kind: breakout
timeframes: ["1h", "4h", "1d"]
parameters:
  lookback_window: 20
  threshold_bps: 25
  max_position_pct: 0.5
risk:
  stop_loss_bps: 1000
  take_profit_bps: 1800
  min_backtest_trades: 5
  min_alpha_vs_buy_hold_pct: 0
---

# Breakout Spot Baseline

Trend-continuation baseline for high-liquidity pairs. Enter when a
closed candle breaks above the prior lookback high by the threshold;
exit on a downside break, stop-loss, or take-profit.

This is meant to catch strong directional moves while still being
compatible with intent-based spot execution. Reject it when volume is
thin, spreads are wide, or the projected route needs fragile
cross-chain hops.
