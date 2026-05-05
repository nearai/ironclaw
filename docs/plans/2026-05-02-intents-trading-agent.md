# Intents Trading Agent on IronClaw

**Status**: prototype scaffold
**Date**: 2026-05-02
**Owner**: tbd

## Goal

Build a TradingAgents-style crypto trading workflow on top of IronClaw
that can research multichain spot opportunities, debate bull and bear
cases, apply deterministic risk gates, and produce unsigned NEAR Intent
bundles for user review.

The first version is intentionally conservative: paper mode by default,
quote-only when explicitly requested, no private keys, no signing, and
no raw chain transaction builders.

## Existing IronClaw foundation

IronClaw already has the right execution primitive in
`tools-src/portfolio`:

- `scan` discovers EVM and NEAR portfolio positions.
- `propose` emits deterministic DeFi movement proposals from strategy
  docs.
- `build_intent` turns a `MovementPlan` into a `portfolio-intent/1`
  unsigned intent bundle through fixture, replay, or live NEAR Intents
  solver paths.
- `backtest` runs deterministic long-only spot strategy tests over
  caller-provided OHLCV candles before a trade idea is eligible for
  intent construction.
- `backtest_suite` ranks a menu of strategy candidates over the same
  OHLCV episode so users can choose from tested alternatives.

The new `skills/intents-trading-agent` skill is the orchestration layer
above that tool.

## TradingAgents role mapping

| TradingAgents role | IronClaw prototype role | Output |
|---|---|---|
| Fundamentals analyst | Market data analyst | Trend, volume, volatility, liquidity context |
| News analyst | News/sentiment analyst | Catalyst and risk event memo |
| Technical analyst | Market data analyst | Setup, invalidation, regime notes |
| Research bull | Bull memo | Best case for the trade |
| Research bear | Bear memo | Best case against the trade |
| Trader | Trader proposal | Direction, notional, route, expected output |
| Risk manager | Risk analyst | Gate pass/fail table |
| Portfolio manager | Manager synthesis | Final stance: skip, watch, paper-intent, quote-intent |

## Open-source references

The local implementation borrows design patterns from mature
open-source trading frameworks without copying code:

- [Freqtrade](https://github.com/freqtrade/freqtrade): dry-run first,
  explicit backtesting, fee/slippage-aware reporting, strategy
  discovery, hyperopt, and lookahead/recursive-analysis commands.
- [Backtrader](https://www.backtrader.com/): strategy/analyzer split
  and reusable metrics.
- [Jesse](https://github.com/jesse-ai/jesse): crypto-native strategy
  research, multi-symbol/timeframe discipline, risk helpers, metrics,
  optimization, Monte Carlo analysis, and debug workflow.
- [TradingAgents](https://github.com/TauricResearch/TradingAgents):
  analyst, bull/bear debate, trader, risk, and portfolio manager role
  decomposition.

The code remains an IronClaw-native Rust/WASM tool path so licensing,
audit boundaries, and unsigned-intent constraints stay simple.

## Project layout

The skill writes state under `projects/intents-trading-agent/`:

```text
projects/intents-trading-agent/
|-- AGENTS.md
|-- config.json
|-- watchlist.md
|-- addresses.md
|-- .system/
|   `-- widgets/
|       `-- near-intents-console/
|           |-- manifest.json
|           |-- index.js
|           `-- style.css
|-- state/
|   |-- latest.json
|   `-- history/
|-- research/
|-- debates/
|-- decisions/
|-- risk/
|-- backtests/
|-- intents/
|-- journal/
`-- widgets/
    `-- state.json
```

This is separate from the existing `portfolio` project so market
research, trading theses, and intent artifacts are not mixed with the
general DeFi portfolio keeper.

## Execution model

1. Load project config, watchlist, wallet addresses, recent decisions,
   and pending intents.
2. Scan wallet addresses through `portfolio.scan` when available.
3. Run analyst memos for market data, onchain exposure, news/sentiment,
   intents/liquidity, and risk.
4. Run and rank a backtest suite when OHLCV candles are available.
5. Write bull and bear cases, then a manager synthesis.
6. If the stance is `paper-intent` or `quote-intent`, build a
   `MovementPlan`.
7. Apply risk gates.
8. Call `portfolio.build_intent`.
9. Call `portfolio.format_intents_widget`.
10. Persist the unsigned bundle, widget state, and summarize what was built.

## Modes

| Mode | Solver | Intended use |
|---|---|---|
| `paper` | `fixture` | Local experiments, replayable tests, no live quote claims |
| `quote` | `near-intents` | Live quote only, still unsigned |
| `execution` | future wallet flow | Requires explicit user signing outside the agent |

The default config sets `mode` to `paper`.

## Risk gates

A proposal cannot become an intent unless all configured gates pass:

- notional <= `max_notional_usd`,
- daily turnover <= `max_daily_turnover_usd`,
- confidence >= `min_confidence`,
- expected edge >= `min_expected_edge_bps`,
- asset and chain allowlists,
- slippage <= `max_slippage_bps`,
- backtest return, drawdown, trade count, and alpha gates pass when
  candles are available,
- no stale data,
- no unresolved security or exploit concern,
- no conflicting pending intent.

## M0 deliverables

- `skills/intents-trading-agent/SKILL.md` defines the workflow.
- `skills/intents-trading-agent/strategies/*.md` defines the first
  spot strategy test corpus plus Hyperliquid signal/risk-gate notes.
- `portfolio.backtest` evaluates `buy-hold`, `sma-cross`, `breakout`,
  `momentum`, `mean-reversion`, and `rsi-mean-reversion` strategies
  with fee/slippage, stop/take-profit, and lookahead-safe next-open
  execution.
- `portfolio.backtest_suite` evaluates a menu of candidates with common
  assumptions, ranks them by a transparent selection score, and returns
  basic gate pass/fail flags.
- `tools-src/portfolio/scenarios/intents-trading-agent-backtest-*.yaml`
  covers the strategy baselines and suite ranking in the replay suite.
- `tools-src/portfolio/scenarios/intents-trading-agent-swap.yaml`
  proves a swap-shaped trading plan can be converted into an unsigned
  intent bundle in replay tests.
- `portfolio.format_intents_widget` emits `intents-trading-widget/1`
  state for the IronClaw Projects widget.
- `skills/intents-trading-agent/widgets/near-intents-console/` provides
  the project widget that renders ranked strategies, risk gates, and
  unsigned NEAR Intents status.
- `tools-src/portfolio/fixtures/solver/ita-swap-near-usdc-btc-near.json`
  records the deterministic solver response for the scenario.

## Next milestones

1. Add a project widget that shows stance, backtest metrics, risk gates,
   pending intents,
   and recent paper PnL.
2. Add a data-source adapter for token prices and volatility, keeping
   it read-only.
3. Add replay scenarios for cross-chain spot rotation and route refusal.
4. Add walk-forward validation, parameter grids, and regime-split reports over
   `state/history`, `journal`, and external candle files.
5. Integrate a wallet-signing handoff only after the unsigned-artifact
   flow is stable and auditable.
