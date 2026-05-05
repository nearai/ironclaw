---
id: hyperliquid-book-imbalance-gate
version: 1
kind: gate-hyperliquid-orderbook
venue: hyperliquid
status: risk-gate
timeframes: ["1m", "5m", "15m"]
data_sources:
  - hyperliquid_l2_book
  - hyperliquid_all_mids
parameters:
  depth_bps: 50
  max_spread_bps: 20
  min_bid_ask_depth_ratio: 0.65
  max_bid_ask_depth_ratio: 1.55
risk:
  blocks_intent_when_book_thin: true
  blocks_intent_when_spread_wide: true
---

# Hyperliquid Book Imbalance Gate

Risk gate for checking whether a Hyperliquid market is too thin,
one-sided, or wide-spread to support a spot intent thesis. This is not a
market-making strategy. It should block or downsize trades when book
conditions imply poor execution or fragile momentum.

Pass only when the top-of-book spread is inside budget, depth near the
intended route size is adequate, and bid/ask depth is not badly skewed.

Fail the gate when a trade would be forced through a thin book, when
spread exceeds the configured threshold, or when imbalance suggests the
signal is already crowded.
