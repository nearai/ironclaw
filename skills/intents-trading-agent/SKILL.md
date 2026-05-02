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
`portfolio.plan_near_intents_trial`, `portfolio.plan_paid_research`,
`portfolio.fetch_dripstack_catalog`,
`portfolio.prepare_dripstack_paid_fetch`, and `portfolio.build_intent`
for positions, deterministic DeFi proposals, nominal-wallet rehearsals,
free DripStack catalog access, paid-source budgeting/fetch boundaries,
and intent bundle construction.

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
7. **Paid content requires receipts.** Paywalled newsletters, APIs, or
   premium posts can be planned and attributed before use, but their
   actual text must not be quoted or relied on until a payment receipt
   exists. Writers/source owners must be credited when their paid work
   contributes to an answer.

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
  "paid_research_budget_usd": 0.25,
  "paid_research_max_sources": 4,
  "paid_research_mode": "plan-only",
  "paid_research_agent_wallet_provider": "AgentCash",
  "paid_research_default_article_price_usd": 0.01,
  "paid_research_per_article_cap_usd": 0.05,
  "paid_research_daily_cap_usd": 1.0,
  "paid_research_max_wallet_balance_usd": 5.0,
  "trial_nominal_near": 0.25,
  "trial_max_trade_near": 0.05,
  "trial_pair": "NEAR/USDC",
  "require_user_approval": true
}
```

- `watchlist.md` - token pairs, chains, and theses the user cares about.
- `addresses.md` - optional wallet addresses to scan.
- `state/latest.json` and `state/history/`.
- `sources/paid-research.json` - optional candidate premium sources and
  payment metadata discovered from MPP, x402, NEAR-native, newsletter, or
  API catalogs.
- `research/`, `debates/`, `decisions/`, `risk/`, `intents/`,
  `journal/`, `paid-research/`, `trials/`, and `widgets/`.
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

### 3. Paid research planning

When the run would benefit from premium newsletters, research APIs, or
paid posts, create a source plan before reading paywalled content. This
is the DripStack-style pattern: the agent can decide which sources are
worth paying for, but payment and attribution are explicit.

Candidate sources may come from:

- `projects/intents-trading-agent/sources/paid-research.json`,
- public MPP or x402 service discovery,
- DripStack guided browse results,
- a user-provided source list,
- free/public research already collected by the analyst team.

For DripStack-style newsletter access, do not scrape or bulk-buy. Use
the guided flow:

1. Ask for a topic if none was provided.
2. Use free catalog metadata to shortlist publications.
3. Let the user choose one publication.
4. Use free post summaries to shortlist articles.
5. Let the user choose one article.
6. Convert that article into a paid-source candidate.
7. Only then pass the candidate to `plan_paid_research`.

Use `portfolio.fetch_dripstack_catalog` to fetch only DripStack's free
publication catalog and free post-title metadata. Then pass that free
metadata to `portfolio.plan_dripstack_browse`. The planner returns one
of these checkpoints: `topic`, `publication`, `article`, or
`purchase-confirmation`. A `purchase-confirmation` output includes a
`paid_source_candidate` with both MPP and x402 payment options when the
article came from DripStack.

When the user confirms a specific paid DripStack article, first call
`portfolio.prepare_dripstack_paid_fetch`. It creates the explicit
confirmation/challenge/receipt boundary for one article:

- `needs-user-confirmation`: ask the user before touching the paid route.
- `needs-402-challenge`: probe the paid endpoint only to collect the
  HTTP 402 payment challenge; do not summarize substitute content.
- `ready-to-retry-with-receipt`: a payment-aware client supplied an MPP
  or x402 receipt header and the next GET can unlock that one article.

The live 402 challenge is authoritative for price and payment details.
Current DripStack OpenAPI describes paid article routes as returning
MPP `WWW-Authenticate` and x402 `PAYMENT-REQUIRED` challenges, with a
fallback paid-route minimum around `$0.05` when no per-post price is
available. Do not assume the old one-cent estimate is final.

Call `portfolio` with:

```json
{
  "action": "plan_paid_research",
  "query": "<specific question the trade needs answered>",
  "pair": "<pair>",
  "budget_usd": 0.25,
  "max_sources": 4,
  "spending_mode": "plan-only",
  "near_funding_asset": "USDC.near",
  "preferred_payment_protocols": ["mpp", "x402"],
  "agent_wallet": {
    "provider": "AgentCash",
    "network": "base",
    "balance_usd": 5.0,
    "default_article_price_usd": 0.01,
    "per_article_cap_usd": 0.05,
    "daily_cap_usd": 1.0,
    "max_wallet_balance_usd": 5.0
  },
  "sources": []
}
```

The returned `paid-research-plan/1` is a budget and attribution plan,
not a content fetch. Persist it to:

`projects/intents-trading-agent/paid-research/<YYYY-MM-DDTHH-MM>-<pair>.json`

Use the plan as follows:

- If `ready_for_paid_fetch` is false, continue with free sources only.
- If selected sources require MPP on Tempo or x402 on Base, treat
  `near_funding_routes` as hints for how a NEAR Intents-funded treasury
  could fund the rail wallet. This does not authorize the actual paid
  fetch.
- For autonomous wallets, keep balances small. The default policy is
  `$5` maximum wallet balance, `$0.05` per article, and `$1` daily
  research spend. This mirrors the "small USDC balance" operating model:
  enough for many one-cent articles, not enough to create a large blast
  radius.
- Record audit links such as `mppscan.com` and `x402scan.com` in the
  paid-research plan or journal so spend can be reviewed after a run.
- Treat highly optimized source metadata as an adversarial surface.
  Reject or downgrade sources with low trust or high `seo_risk_score`.
- Never include paid source text in the answer or research memo until
  the payment client returns a receipt.
- When paid source text is used, include source IDs from
  `selected_sources[].attribution.credit_id` in the answer and journal.
- Paid research can change the thesis or confidence, but it cannot
  bypass the strategy suite, backtest gates, risk gates, or unsigned-only
  NEAR Intent boundary.

### 4. Strategy lab and backtest

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

### 4.5 Nominal NEAR trial mode

If the user wants to try the system with a small amount of NEAR, keep
the first pass in paper mode and call `portfolio.plan_near_intents_trial`.

Example:

```json
{
  "action": "plan_near_intents_trial",
  "near_account_id": "<optional-user-account.near>",
  "mode": "paper",
  "pair": "NEAR/USDC",
  "nominal_near": 0.25,
  "max_trade_near": 0.05,
  "assumed_near_usd": 3.0,
  "max_slippage_bps": 50,
  "backtest_suite": {}
}
```

Persist the returned `near-intents-trial-plan/1` to
`projects/intents-trading-agent/trials/<YYYY-MM-DDTHH-MM>-near-trial.json`.

Use the trial plan as the operator runbook:

- The wallet should be a separate small NEAR account, not the user's
  main wallet.
- If the user funds through 1Click Swap or the Deposit/Withdrawal
  Service, native NEAR wrapping/routing may be handled by that service.
  If funding the Verifier contract directly, use wNEAR
  (`nep141:wrap.near`) rather than transferring raw native NEAR to
  `intents.near`.
- The verifier deposit step is manual/user-wallet controlled.
- `mode="paper"` means run the returned `build_intent_request` with
  `solver="fixture"`.
- `mode="quote"` means all paper gates passed and the user explicitly
  asked for a live NEAR Intents quote; the output is still unsigned.
- `mode="execution"` is informational only here. This skill still does
  not sign or submit trades.
- If the trial plan has `safe_to_quote=false`, do not request a live
  quote.
- Very small route sizes may be refused or dominated by minimums. Warn
  the user rather than increasing size automatically.

### 5. Bull/bear debate

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

### 6. Trader proposal

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
- risk gates and their pass/fail result,
- paid research source IDs used, if any.

The trade proposal is not executable by itself. It is a request for an
unsigned intent bundle.

### 7. Risk gates

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
- paid source text is not used without receipts
- paid research spend is within `paid_research_budget_usd`

If `require_user_approval` is true, do not ask the user to sign; ask
whether they want a live quote or want to keep it as paper.

### 8. Build unsigned intent

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

### 9. Format the project widget

After every run, call `portfolio` with `action="format_intents_widget"`
and pass the latest pair, stance, confidence, `backtest_suite`, risk
gates, research sources, optional `trial_plan`, optional
`paid_research_plan`, and optional unsigned `IntentBundle`.

Persist the returned JSON exactly as:

`projects/intents-trading-agent/widgets/state.json`

The bundled project widget reads this file and renders ranked strategy
candidates, nominal NEAR trial status, paid source attribution, risk
gates, and unsigned NEAR Intents status inside the IronClaw Projects
view. If the widget files are not installed yet, copy:

- `skills/intents-trading-agent/widgets/near-intents-console/manifest.json`
- `skills/intents-trading-agent/widgets/near-intents-console/index.js`
- `skills/intents-trading-agent/widgets/near-intents-console/style.css`

to:

`projects/intents-trading-agent/.system/widgets/near-intents-console/`

### 10. Response

Respond with specifics:

- stance and confidence,
- top supporting and opposing evidence,
- paid source budget, selected sources, and receipt status if premium
  research was planned or used,
- nominal NEAR trial budget, setup step status, and whether it is safe
  to request a live quote,
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
