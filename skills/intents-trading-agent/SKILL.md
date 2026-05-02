---
name: intents-trading-agent
version: 0.1.0
description: TradingAgents-style crypto trading workflow for IronClaw that researches multichain spot opportunities, debates bull/bear cases, applies risk gates, and builds unsigned NEAR Intent bundles through the portfolio tool.
activation:
  keywords:
    - intents trading
    - trading agent
    - crypto trading agent
    - autonomous crypto trading
    - multichain trading
    - near intents trading
    - intent based trading
    - rebalance into
    - swap thesis
    - token rotation
  patterns:
    - "(?i)(trade|swap|rotate|rebalance).*(NEAR Intents|intents|multichain|cross-chain)"
    - "(?i)(autonomous|agentic).*(crypto|token|trading|portfolio)"
    - "(?i)(build|try|prototype).*(TradingAgents|trading agents).*(IronClaw|intents)"
  exclude_keywords:
    - nft
    - mint
  tags:
    - crypto
    - trading
    - finance
    - intents
  max_context_tokens: 5000
requires:
  skills:
    - portfolio
    - llm-council
    - decision-capture
---

# Intents Trading Agent

You run a TradingAgents-style crypto workflow on top of IronClaw.
The goal is to turn market research into **unsigned NEAR Intent
bundles** that the user can inspect and sign in their own wallet.

This skill is a research and orchestration layer. It does not replace
the `portfolio` WASM tool. Use `portfolio.scan`, `portfolio.propose`,
and `portfolio.build_intent` for positions, deterministic DeFi
proposals, and intent bundle construction.

## Non-negotiables

1. **No private keys.** Never ask for seed phrases, private keys, or
   signing material. Never claim to have signed or submitted a trade.
2. **Unsigned only.** The agent may autonomously research, rank, and
   build unsigned intent artifacts. Signing is always a user-wallet
   action outside this skill.
3. **Paper mode by default.** Unless the user explicitly opts into live
   quoting, use fixture/replay paths and mark all output as test or
   paper. Do not present fixture bundles as executable quotes.
4. **Spot only in v0.** Do not propose leverage, perps, borrowing, or
   recursive collateral loops. Defensive deleveraging proposals from
   `portfolio.propose` are allowed.
5. **Risk gates before intents.** A trade idea becomes an intent only
   after the risk manager approves it against project config.
6. **Audit trail first.** Persist research, debate notes, risk checks,
   decisions, and built intents under `projects/intents-trading-agent/`.

## Project bootstrap

If the project does not exist, create `projects/intents-trading-agent/`
using workspace writes. Initialize:

- `AGENTS.md` - operating principles and tool boundaries.
- `config.json`:

```json
{
  "mode": "paper",
  "max_notional_usd": 100,
  "max_daily_turnover_usd": 250,
  "max_slippage_bps": 50,
  "min_confidence": 0.72,
  "min_expected_edge_bps": 75,
  "cooldown_minutes": 240,
  "allowed_chains": [],
  "allowed_assets": ["NEAR", "USDC", "USDT", "BTC", "ETH"],
  "disallowed_assets": [],
  "require_user_approval": true
}
```

- `watchlist.md` - token pairs, chains, and theses the user cares about.
- `addresses.md` - optional wallet addresses to scan.
- `state/latest.json` and `state/history/`.
- `research/`, `debates/`, `decisions/`, `risk/`, `intents/`,
  `journal/`, and `widgets/`.
- `strategies/` - copy or reference the baseline strategy docs bundled
  with this skill (`sma-cross-spot`, `breakout-spot`,
  `momentum-spot`, `mean-reversion-spot`,
  `rsi-mean-reversion-spot`, `buy-hold-benchmark`) plus signal/gate
  docs such as `hyperliquid-funding-carry-watch`,
  `hyperliquid-book-imbalance-gate`, and
  `hyperliquid-perp-momentum-signal`.
- `.system/widgets/near-intents-console/` - copy the bundled widget
  files from `skills/intents-trading-agent/widgets/near-intents-console/`
  so IronClaw Projects can render the NEAR Intents trading console.

If a `portfolio` project already exists, read it for wallet/address
context, but keep trading-agent research and decisions in
`projects/intents-trading-agent/`.

## Run procedure

### 1. Load state

Read config, watchlist, addresses, latest state, recent decisions, and
open unsigned intents. If the user provides a wallet address, append it
to `addresses.md` and scan it.

If addresses are present, call:

```json
{
  "action": "scan",
  "addresses": ["<address>"],
  "chains": "*",
  "source": "auto"
}
```

Save the returned positions exactly as `state/latest.json` and append a
dated copy under `state/history/`.

### 2. Analyst team

Run the analysis as distinct roles. Keep each role short and evidence
grounded.

- **Market Data Analyst**: price trend, volume, volatility, liquidity,
  and drawdown context for the watched pair. For Hyperliquid/HL ideas,
  include funding, candle snapshot, all-mids, and L2 book context when
  available, but treat those as signal/risk inputs unless execution
  support is explicitly implemented.
- **Onchain/Portfolio Analyst**: wallet exposure, idle balances,
  concentration, chain exposure, and any deterministic proposals from
  `portfolio.propose`.
- **News/Sentiment Analyst**: recent catalysts, governance, listings,
  exploits, unlocks, regulatory events, and social sentiment. Use web
  search for current facts when available.
- **Intents/Liquidity Analyst**: whether the move is suitable for
  intent execution, likely source/destination chains, route complexity,
  solver quote risk, slippage budget, and expiry risk.
- **Risk Analyst**: position sizing, downside scenario, correlation,
  liquidity, stale data, and whether the idea passes config gates.

Write role outputs to
`projects/intents-trading-agent/research/<YYYY-MM-DD>/<role>.md`.

### 3. Strategy lab and backtest

Before a strategy can become a trade proposal, run a deterministic
backtest when historical OHLCV candles are available. Prefer
`action="backtest_suite"` when evaluating a menu of strategies, and use
`action="backtest"` only for a single focused strategy check.

For a strategy menu, call `portfolio` with:

- `candles`: oldest-to-newest OHLCV candles,
- `candidates`: strategy configs with stable IDs,
- common fees, slippage, and starting cash.

Each candidate may include `max_position_pct`, `stop_loss_bps`, and
`take_profit_bps`. Persist the returned ranked suite exactly as
`projects/intents-trading-agent/backtests/<YYYY-MM-DDTHH-MM>-suite-<pair>.json`.

For a single strategy, call `portfolio` with `action="backtest"` and
pass:

- `candles`: oldest-to-newest OHLCV candles,
- `strategy`: one of `buy-hold`, `sma-cross`, `breakout`,
  `momentum`, `mean-reversion`, `rsi-mean-reversion`,
- fees, slippage, max position size, and optional stop/take-profit.

The backtester is inspired by common open-source trading frameworks:
dry-run/backtest-first workflow, explicit fee/slippage assumptions,
lookahead-safe next-open execution, analyzers/metrics, and a
buy-and-hold benchmark. Do not copy external strategy code into the
workspace; use these as design patterns and keep the local implementation
auditable.

Persist reports to
`projects/intents-trading-agent/backtests/<YYYY-MM-DDTHH-MM>-<strategy>-<pair>.json`.

Reject or downgrade to `watch` if:

- `lookahead_safe` is false,
- no suite candidate passes `passes_basic_gate`,
- completed trades are below the strategy doc minimum,
- max drawdown exceeds project risk budget,
- total return is negative after fees and slippage,
- alpha versus buy-and-hold is negative for a trend-following strategy,
- win rate or profit factor fails the strategy doc minimum,
- the test sample is too small or excludes a relevant stress regime.

Always run `buy-hold` as a benchmark when enough candles are available.
Always present the top ranked suite candidates and rejected candidates
separately so the user can see what was considered.

### 4. Bull/bear debate

Create two concise debate memos:

- Bull case: why the trade should be considered now.
- Bear case: why it should be skipped, sized down, or delayed.

Then write a manager synthesis with:

- consensus points,
- unresolved disagreements,
- key missing data,
- confidence score from 0 to 1,
- final stance: `skip`, `watch`, `paper-intent`, or `quote-intent`.

Persist to `projects/intents-trading-agent/debates/<YYYY-MM-DDTHH-MM>-<pair>.md`.

### 5. Trader proposal

Only create a trade proposal when the manager stance is
`paper-intent` or `quote-intent`.

A proposal must include:

- pair and direction,
- source asset, destination asset, source chain, destination chain,
- notional USD and token amount,
- expected output and expected cost,
- thesis,
- invalidation condition,
- expiry or review time,
- confidence,
- risk gates and their pass/fail result.

The trade proposal is not executable by itself. It is a request for an
unsigned intent bundle.

### 6. Risk gates

Reject or downgrade to `watch` if any gate fails:

- `notional_usd <= max_notional_usd`
- daily turnover including pending intents <= `max_daily_turnover_usd`
- confidence >= `min_confidence`
- expected edge >= `min_expected_edge_bps`
- source and destination assets are allowed and not disallowed
- slippage <= `max_slippage_bps`
- data freshness is acceptable
- no unresolved security/exploit concern
- no existing conflicting pending intent
- backtest gates pass when candles are available

If `require_user_approval` is true, do not ask the user to sign; ask
whether they want a live quote or want to keep it as paper.

### 7. Build unsigned intent

For an approved `paper-intent`, call `portfolio.build_intent` with
`solver: "fixture"` so the output is clearly a deterministic test
bundle.

For an approved `quote-intent`, call `portfolio.build_intent` with
`solver: "near-intents"` only if the user explicitly requested live
quoting and the environment supports it. The returned bundle is still
unsigned.

Use a `MovementPlan` shaped like:

```json
{
  "proposal_id": "ita-<YYYYMMDD>-<pair>-<slug>",
  "legs": [
    {
      "kind": "swap",
      "chain": "<source-chain>",
      "from_token": {
        "symbol": "<source-symbol>",
        "address": null,
        "chain": "<source-chain>",
        "amount": "<amount>",
        "value_usd": "<notional-usd>"
      },
      "to_token": {
        "symbol": "<destination-symbol>",
        "address": null,
        "chain": "<destination-chain>",
        "amount": "<expected-amount>",
        "value_usd": "<expected-usd>"
      },
      "description": "Swap <source> to <destination> via NEAR Intents"
    }
  ],
  "expected_out": {
    "symbol": "<destination-symbol>",
    "address": null,
    "chain": "<destination-chain>",
    "amount": "<expected-amount>",
    "value_usd": "<expected-usd>"
  },
  "expected_cost_usd": "<max-cost-usd>"
}
```

For cross-chain moves, use ordered `withdraw`, `bridge`, `swap`, and
`deposit` legs as needed. Never hand-build a raw EVM transaction.

Persist returned bundles to
`projects/intents-trading-agent/intents/<YYYY-MM-DDTHH-MM>-<proposal_id>.json`.

### 8. Format the project widget

After every run, call `portfolio` with `action="format_intents_widget"`
and pass the latest pair, stance, confidence, `backtest_suite`, risk
gates, research sources, and optional unsigned `IntentBundle`.

Persist the returned JSON exactly as:

`projects/intents-trading-agent/widgets/state.json`

The bundled project widget reads this file and renders ranked strategy
candidates, risk gates, and unsigned NEAR Intents status inside the
IronClaw Projects view. If the widget files are not installed yet, copy:

- `skills/intents-trading-agent/widgets/near-intents-console/manifest.json`
- `skills/intents-trading-agent/widgets/near-intents-console/index.js`
- `skills/intents-trading-agent/widgets/near-intents-console/style.css`

to:

`projects/intents-trading-agent/.system/widgets/near-intents-console/`

### 9. Response

Respond with specifics:

- stance and confidence,
- top supporting and opposing evidence,
- backtest summary versus buy-and-hold when available,
- top strategy-suite candidates when available,
- risk gate table,
- proposed notional and route,
- intent status: `none`, `paper-built`, `live-quote-built`, or
  `blocked`,
- widget status and state path,
- exact project paths written.

Make clear that this is research and unsigned intent construction, not
financial advice or executed trading.

## Recurring mission

Offer, but do not auto-create, a mission:

- `name`: `intents-trading-agent`
- `project_id`: `intents-trading-agent`
- `cadence`: `0 */4 * * *`
- `goal`: "Run the intents trading workflow in paper mode for the
  configured watchlist. Research current market/onchain/news context,
  run and rank a strategy suite when candles are available, debate bull
  and bear cases, apply risk gates, and build unsigned paper intent
  bundles only when every gate passes. Never sign or submit trades."

If the user later opts into live quotes, update the mission goal to say
`live quote bundles are allowed, signing remains user-only`.
