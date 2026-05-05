---
id: mean-reversion-spot
version: 1
kind: mean-reversion
timeframes: ["1h", "4h"]
parameters:
  lookback_window: 20
  threshold_bps: 300
  max_position_pct: 0.35
risk:
  stop_loss_bps: 800
  min_backtest_trades: 8
  min_win_rate_pct: 45
---

# Mean Reversion Spot Baseline

Liquidity-reversion baseline for large-cap pairs. Enter when price
closes materially below a moving average and exit when price reverts to
the average.

Use only on assets with deep liquidity and no obvious impairment
catalyst. Do not use this to catch falling knives after hacks, token
unlock shocks, depegs, regulatory events, or protocol insolvency
rumors.
