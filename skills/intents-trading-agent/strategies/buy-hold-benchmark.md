---
id: buy-hold-benchmark
version: 1
kind: buy-hold
timeframes: ["1h", "4h", "1d"]
parameters:
  max_position_pct: 1.0
risk:
  benchmark_only: true
---

# Buy and Hold Benchmark

Baseline benchmark for alpha checks. The trading agent should compare
active strategies against buy-and-hold after fees and slippage. A
strategy that adds complexity but underperforms buy-and-hold should be
rejected or downgraded to watch mode.
