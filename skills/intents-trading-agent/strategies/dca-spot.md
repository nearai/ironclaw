---
id: dca-spot
version: 1
kind: dca
timeframes: ["1h", "4h", "1d", "1w"]
parameters:
  notional_per_period_usd: 100
  period_candles: 7
  offset: 0
  skip_above_premium_bps: null
  opportunistic_below_discount_bps: null
  fee_bps: 10
  slippage_bps: 5
risk:
  min_total_periods: 6
  max_per_period_notional_usd: 1000
  max_total_notional_usd: 25000
  max_underlying_drawdown_pct: 70
gates:
  unsigned_only: true
  intents_supported_destination: true
---

# Dollar-Cost Averaging (Spot)

Recurring fixed-notional buys into a NEAR-Intents-supported destination
asset (NEAR, USDC, USDT, BTC, ETH, WETH, WBTC). The strategy doc bounds
two surfaces:

1. **`portfolio.backtest_dca`** — replays the schedule over candles and
   reports lots, average cost basis, breakeven price, mark-to-market,
   alpha vs lump-sum buy-and-hold, and the underlying drawdown the
   schedule had to survive.
2. **`portfolio.plan_dca_schedule`** — emits a cron expression, per-period
   plan list, deterministic risk gates, and a `build_intent` template
   the agent uses to produce one unsigned intent per period. The agent
   never signs.

## When to use

- The user has a multi-week / multi-month accumulation thesis and
  wants exposure averaged across regimes, not timed to one entry.
- The asset is illiquid enough that splitting size reduces solver
  refusal risk and per-period slippage.
- The user does not have a high-confidence directional view and wants
  drawdown-tolerance over peak return.

## When not to use

- Total notional fits comfortably in one solver quote with low slippage
  — lump-sum buy-and-hold has lower fee drag.
- The thesis is event-driven (governance vote, listing, unlock) and
  expected to resolve inside one cadence period.
- The destination asset is not in the NEAR Intents allowlist; the
  schedule planner will warn but the route may still fail.

## Optional symmetric price band

The backtest accepts two band parameters that turn vanilla DCA into a
mild "skip when stretched, double-up when discounted" variant without
breaking determinism:

- `skip_above_premium_bps`: skip a buy when the candle close is more
  than this many bps above the running average cost basis. Reduces
  buying into late-stage rallies.
- `opportunistic_below_discount_bps`: double the buy when the candle
  close is more than this many bps below the running average cost
  basis. Front-loads accumulation during selloffs.

Both are off by default. Setting either preserves the deterministic
replay contract — the band is evaluated on candle close, after the
running average is updated, with no lookahead.

## Backtest gates before live quoting

A DCA schedule is eligible for live (`mode="quote"`) intent
construction only if all of the following hold over the candle replay:

- the schedule completed at least `min_total_periods` lots,
- mark-to-market vs lump-sum buy-and-hold is within an explicit
  tolerance the operator set in project config,
- max underlying drawdown is within `max_underlying_drawdown_pct`,
- the destination asset is in the NEAR Intents allowlist, or the
  operator has explicitly approved an off-allowlist route,
- per-period notional does not exceed `max_per_period_notional_usd`,
- total schedule notional does not exceed `max_total_notional_usd`.

Failing any of these downgrades the proposal to `watch` and refuses to
emit a quote-mode build_intent template.

## Cron cadence

The schedule planner accepts:

- `daily` → `0 12 * * *`
- `weekly` → `0 12 * * 1` (Monday noon UTC)
- `biweekly` → `0 12 1,15 * *`
- `monthly` → `0 12 1 * *`
- a raw 5-or-6-field cron string for custom cadences.

Cron is advisory metadata. IronClaw schedules each period independently
through its mission/cron layer; the DCA planner does not run a daemon.
