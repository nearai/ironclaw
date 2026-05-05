---
id: rsi-mean-reversion-spot
version: 1
kind: rsi-mean-reversion
timeframes: ["1h", "4h"]
parameters:
  lookback_window: 14
  entry_threshold: 30
  exit_threshold: 55
  max_position_pct: 0.3
risk:
  stop_loss_bps: 700
  min_backtest_trades: 8
  min_win_rate_pct: 45
---

# RSI Mean Reversion Spot Baseline

Oscillator-driven reversion baseline. Enter when RSI is oversold and
exit when RSI normalizes. This is only appropriate for deep-liquidity
assets where the move looks like flow exhaustion, not fundamental
impairment.

Reject the strategy during protocol exploit news, stablecoin depegs,
forced unlocks, liquidation cascades, chain halts, or any case where
the "oversold" move may be information rather than noise.
