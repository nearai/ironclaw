---
id: momentum-spot
version: 1
kind: momentum
timeframes: ["4h", "1d"]
parameters:
  lookback_window: 20
  threshold_bps: 500
  max_position_pct: 0.4
risk:
  stop_loss_bps: 1000
  min_backtest_trades: 5
  min_alpha_vs_buy_hold_pct: 0
---

# Momentum Spot Baseline

Relative momentum baseline for liquid assets. Enter when trailing
lookback return clears the threshold; exit when trailing momentum
fades to zero or lower.

Use this for rotation candidates where the thesis is continued strength
rather than a single breakout candle. Reject when the asset is already
extended into a known unlock, exploit, delisting, depeg, or governance
risk window.
