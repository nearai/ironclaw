# Intents Trading Agent Research Notes

**Date**: 2026-05-02
**Purpose**: evaluate open-source trading/backtesting systems and
IronClaw skills before expanding the intents trading agent beyond the
initial scaffold.

## Source Review

| Project | What it contributes | What to avoid |
|---|---|---|
| [Freqtrade](https://github.com/freqtrade/freqtrade) | Mature crypto-bot lifecycle: data download, backtesting, backtest analysis, hyperopt, plotting, dry-run/live split, strategy listing, lookahead analysis, recursive analysis. | Do not import strategy code or live exchange execution. IronClaw should keep execution unsigned and wallet-mediated. |
| [Freqtrade hyperopt docs](https://www.freqtrade.io/en/stable/hyperopt/) | Hyperparameter search is just repeated backtesting over historical data with explicit loss functions, min-trades, fee, timerange, pair, timeframe, and data-format controls. | Do not add heavy optimization dependencies into the WASM tool yet. Start with deterministic grid/replay hooks. |
| [Freqtrade lookahead analysis](https://docs.freqtrade.io/en/latest/lookahead-analysis/) | Bias detection is a first-class command, not a footnote. Backtests must prove they do not use future candles. | Do not accept indicator code that only works because it saw the whole dataframe. |
| [Jesse](https://github.com/jesse-ai/jesse) | Crypto-native research framework: simple strategy syntax, indicator library, multiple symbols/timeframes, risk helpers, metrics, debug mode, optimization, Monte Carlo, ML pipeline. | No leverage/shorts in v0. No partial-fill simulation until solver route and wallet handoff exist. |
| [Backtrader](https://www.backtrader.com/) | Clear separation between strategy, data feeds, broker/store, observers, and analyzers. Reusable analyzers are the right mental model for metrics. | Backtrader is Python/process heavy; do not embed it in the sandbox path. Use its architecture pattern. |
| [Backtrader analyzers](https://www.backtrader.com/docu/analyzers/analyzers/) | Analyzers evaluate one strategy's performance and return compact results after a run. This maps well to `portfolio.backtest` returning `metrics`, `trades`, and `equity_curve`. | Avoid opaque metrics without trade logs; every metric should be traceable to candles and fills. |
| [Hummingbot](https://github.com/hummingbot/hummingbot) | Connector/strategy split, market-making posture, CEX+DEX venue abstraction, and bot config discipline. | Market making needs live order management, inventory controls, and cancel/replace mechanics; do not pretend NEAR Intent swaps are an order book maker. |
| [Hummingbot perpetual market making](https://hummingbot.org/strategies/v1-strategies/perpetual-market-making/) | Funding-aware perp strategies, mid-price references, spreads, and position management are useful design references for future Hyperliquid research. | Perps/leverage stay out of v0; use HL only as a signal source until execution semantics are explicit. |
| [CCXT](https://github.com/ccxt/ccxt) | Unified exchange market-data API across many venues; public endpoints include tickers, order books, trades, and OHLCV. | Private exchange trading APIs conflict with the unsigned-wallet constraint; do not route execution through API keys. |
| [vectorbt](https://vectorbt.dev/) | Fast vectorized parameter sweeps and heatmaps; good mental model for running many strategy variants over the same series. | Python/numpy research can live outside the WASM boundary; production gates should remain deterministic and replayable. |
| [TradingAgents](https://github.com/TauricResearch/TradingAgents) | Analyst team, bull/bear research debate, trader, risk management, and portfolio-management decomposition. | Do not let the debate layer bypass deterministic gates. LLM confidence is advisory, not execution authority. |

## Crypto Portal / Data Source Inventory

| Source | Useful data | Fit for IronClaw | Notes |
|---|---|---|---|
| [NEAR Intents solver relay](https://docs.near-intents.org/near-intents/market-makers/bus/solver-relay) | Quote requests, signed-intent publishing, status checks. | Execution/quote path, but unsigned until user wallet signs. | `portfolio.build_intent` already models fixture/replay/live solver modes. |
| [NEAR Intents chain/address support](https://docs.near-intents.org/near-intents/chain-address-support) | Supported signing standards, token support, chain support. | Capability discovery and route gating. | Use this before promising a multichain route. |
| [Hyperliquid info endpoint](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint) | All mids, user fills, L2 book snapshots, candle snapshots. | Strong read-only signal source for Hyperliquid spot/perp market context. | Candle snapshots are capped; long history needs recording or external archives. |
| [Hyperliquid funding docs](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/funding) | Hourly funding mechanics and premium/oracle relationship. | Signal source for perp/spot divergence and risk-on/risk-off regimes. | Funding/carry execution is not v0 because v0 stays spot-only. |
| [Hyperliquid historical data](https://hyperliquid.gitbook.io/hyperliquid-docs/historical-data) | S3 archives for L2 books, asset contexts, fills, explorer blocks. | Backfill source for serious HL strategy research. | The docs warn archive timeliness/completeness is not guaranteed. |
| [Dune Sim API](https://docs.sim.dune.com/) | Wallet balances, token metadata, activity, transactions across many chains. | Existing `portfolio.scan` EVM source. | Requires API key; read-only. |
| [CoinGecko trading guide](https://docs.coingecko.com/trading) | Spot prices, OHLC candles, market chart ranges, onchain OHLCV. | Candidate market-data adapter for backtests. | Good enough for daily/hourly research; verify plan/rate limits before automation. |
| [DefiLlama API](https://defillama.com/docs/api) | Protocol TVL, yields, unlocks, stablecoins, broader DeFi context. | Risk/catalyst context and yield comparison. | Great for gating and context, not execution. |
| [CCXT](https://github.com/ccxt/ccxt) | Exchange OHLCV/order books/trades from many CEX venues. | Optional external research adapter. | Use public market data only unless user explicitly designs a separate API-key execution mode. |

## Hyperliquid / "HL" Strategy Notes

Assumption: "HL" means Hyperliquid. If the user meant another portal,
rename this section and keep the same gate structure.

| Strategy family | Signal inputs | Can intents execute it now? | Research decision |
|---|---|---|---|
| Perp funding/carry | Funding rate, oracle/perp premium, spot hedge availability. | No, because v0 is spot-only and NEAR Intents are swap/bridge oriented here. | Track as signal and future module; no autonomous perp execution. |
| Perp momentum / breakout | HL candle snapshots, all mids, L2 liquidity, open interest context from external sources. | Partially: signal can map to spot token rotation through intents. | Candidate v1 signal source after candle ingestion exists. |
| Order-book imbalance | HL L2 book snapshots, spread, depth, impact price. | No as a maker; maybe as a slippage/liquidity gate for taker intents. | Use for risk gates before trade selection, not as standalone execution. |
| Liquidation / squeeze context | Funding, premium, order-book thinning, liquidation feeds from third-party portals. | Signal only. | Needs robust data provenance and false-positive controls. |
| Vault/copy-trading | Public vault/user portfolio and fills data. | No direct copy execution. | Could become an analyst input after privacy and survivorship-bias review. |

## Strategy Selection Model

The local tool should expose two levels:

1. `portfolio.backtest`: one strategy config over one OHLCV episode.
2. `portfolio.backtest_suite`: many candidate configs over the same
   episode, ranked by a transparent deterministic score.

The suite is not optimizer magic. It is the minimum "people can choose
from strategies" primitive:

- all candidates share the same candles, fee, slippage, and starting cash,
- each candidate may override size, stop-loss, and take-profit,
- output includes metrics, warnings, `selection_score`, and
  `passes_basic_gate`,
- no intent can be built from the suite result unless the risk manager
  still approves it.

## IronClaw Skill Evaluation

| Skill/tool | Use in framework | Decision |
|---|---|---|
| `portfolio` tool | Position scans, DeFi proposals, intent construction, now backtesting. | Core execution and research substrate. |
| `portfolio` skill | Project bootstrap and portfolio keeper conventions. | Reuse file layout ideas; keep trading-agent state separate. |
| `llm-council` | Optional multi-model bull/bear/manager review. | Advisory only; never a substitute for risk gates. |
| `decision-capture` | Durable records for accepted/rejected trade decisions. | Companion skill, useful for trade journal and later outcome review. |
| `trader-setup` | Existing equities/options commitment workflow. | Pattern reference only; crypto intents gets its own project and config. |
| `commitment-*` skills | Missions, digests, follow-up reminders. | Useful after strategy/backtest corpus stabilizes; not required for M0. |

## Strategy Families To Offer

| Strategy | Kind | Best market | Primary gate |
|---|---|---|---|
| Buy and hold benchmark | `buy-hold` | Any candidate pair | Benchmark only |
| SMA cross | `sma-cross` | Trending, liquid markets | Positive alpha vs buy-hold after costs |
| Breakout | `breakout` | Strong trend continuation | Drawdown and false-breakout controls |
| Momentum | `momentum` | Rotation/risk-on markets | Regime split and turnover cost |
| Mean reversion | `mean-reversion` | Rangebound, deep-liquidity assets | Win rate and stop discipline |
| RSI mean reversion | `rsi-mean-reversion` | Oversold but unimpaired large caps | Catalyst/security veto |

## Backtest Requirements

Minimum framework requirements before a strategy can build an intent:

1. Historical candles are oldest-to-newest and pass OHLC consistency.
2. Signals are computed on closed candles and executed at next open.
3. Fees and slippage are explicit.
4. Result includes trade log, equity curve, drawdown, win rate, profit
   factor, exposure, average trade return, and buy-hold alpha.
5. Strategy-specific minimums are enforced from the strategy doc.
6. Any exploit/depeg/unlock/regulatory risk can veto an otherwise good
   backtest.
7. Strategy menus are evaluated via suite ranking before choosing a
   paper-intent candidate.
8. Walk-forward and out-of-sample splits are required before claiming a
   strategy is robust rather than merely fit to one episode.

## Near-Term Gaps

- Live OHLCV ingestion is still missing.
- Walk-forward and out-of-sample splits are still missing.
- Hyperparameter search is not implemented yet.
- Monte Carlo / trade-order shuffling is not implemented yet.
- Solver-route replay and market-data replay are separate; they need a
  shared episode format.
- No widget yet for backtest/risk/intent status.

## Implementation Slice From This Sprint

- Added `portfolio.backtest` for long-only spot strategy replay.
- Added `portfolio.backtest_suite` to rank a strategy menu over a common
  candle episode.
- Added replay scenarios for individual strategies, suite ranking, and
  a swap-shaped unsigned intent bundle.
- Added the first bundled strategy corpus:
  `buy-hold`, `sma-cross`, `breakout`, `momentum`, `mean-reversion`,
  and `rsi-mean-reversion`.
- Added first Hyperliquid-specific docs as signal/risk inputs:
  funding carry watch, book imbalance gate, and perp momentum signal.
- Added `portfolio.format_intents_widget` and a NEAR Intents project
  widget template so the workflow has an operator-facing IronClaw UI.

## Multi-Day Roadmap

| Phase | Work | Exit criteria |
|---|---|---|
| M1 data ingestion | CoinGecko/CCXT CSV import, Hyperliquid candle snapshot recorder, Dune/NEAR portfolio context. | Backtests can run from saved market-data episodes, not hand-written candles. |
| M2 strategy lab | Parameter grids, walk-forward splits, regime splits, benchmark comparisons, strategy doc thresholds. | A strategy report shows train/test windows, drawdown, turnover, and rejection reasons. |
| M3 risk/intent bridge | Map approved suite results to `MovementPlan`, quote replay, solver refusal tests, route expiry tests, widget state updates. | A paper-intent can be rebuilt deterministically from research + backtest + quote fixture and inspected in the Projects UI. |
| M4 portals and HL signals | Hyperliquid funding/book/candle adapters, DefiLlama risk context, optional CoinGecko fallback. | HL data is used as signal/risk context without pretending to execute perps. |
| M5 UI/mission | Widget for watchlist, ranked strategies, risk gates, pending intents, and paper PnL journal. | User can inspect the autonomous run without reading JSON. |
