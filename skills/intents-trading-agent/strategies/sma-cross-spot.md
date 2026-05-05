---
id: sma-cross-spot
version: 1
kind: sma-cross
timeframes: ["1h", "4h", "1d"]
parameters:
  fast_window: 8
  slow_window: 21
  max_position_pct: 0.5
risk:
  stop_loss_bps: 800
  min_backtest_trades: 5
  min_alpha_vs_buy_hold_pct: 0
---

# SMA Cross Spot Baseline

Trend-following baseline for liquid spot pairs. Enter when the fast
moving average crosses above the slow moving average; exit on the
reverse cross or a stop-loss.

Use this as a sanity benchmark, not a production strategy. It should
only graduate to an intent candidate when the backtest shows enough
trades, positive alpha after fees and slippage, and drawdown inside the
project risk budget.
