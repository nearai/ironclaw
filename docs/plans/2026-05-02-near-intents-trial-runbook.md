# Nominal NEAR Intents Trial Runbook

**Date**: 2026-05-02

This runbook is for testing the Intents Trading Agent with a small NEAR
wallet while keeping every agent-produced artifact unsigned.

## Safety Model

- Use a separate NEAR account funded only with the amount you are
  willing to test.
- Start with `mode="paper"` and `solver="fixture"`.
- Move to `mode="quote"` only after backtest and risk gates pass.
- The agent never sees private keys, seed phrases, wallet exports, or
  signing material.
- A live quote is not an executed trade. Wallet signing remains outside
  IronClaw.

## NEAR Intents Funding Notes

NEAR Intents supports deposits across supported assets through higher
level routes such as the 1Click Swap API and the Deposit/Withdrawal
Service. If you use those routes, native NEAR wrapping can be handled for
you. The manual warning in this runbook is narrower: when integrating
directly with the Verifier contract, it accepts NEP-141 token balances,
so native NEAR is represented as wNEAR (`nep141:wrap.near`) and raw
native NEAR should not be sent directly to `intents.near`.

The bridge/deposit service is available at
`https://bridge.chaindefuser.com/rpc`, and the solver relay is at
`https://solver-relay-v2.chaindefuser.com/rpc`.

Useful docs:

- [Deposit/Withdrawal Service](https://docs.near-intents.org/integration/market-makers/deposit-withdrawal-service)
- [Verifier deposits](https://docs.near-intents.org/near-intents/market-makers/verifier/deposits-and-withdrawals/deposits)
- [Solver relay API](https://docs.near-intents.org/near-intents/market-makers/bus/solver-relay)
- [Intent types and execution](https://docs.near-intents.org/integration/verifier-contract/intent-types-and-execution)

## IronClaw Flow

1. Run `portfolio.backtest_suite` with fresh OHLCV candles and the
   bundled strategy menu.
2. Run `portfolio.plan_near_intents_trial`:

```json
{
  "action": "plan_near_intents_trial",
  "near_account_id": "your-small-test-account.near",
  "mode": "paper",
  "pair": "NEAR/USDC",
  "nominal_near": 0.25,
  "max_trade_near": 0.05,
  "assumed_near_usd": 3.0,
  "max_slippage_bps": 50,
  "backtest_suite": {}
}
```

3. Persist the returned `near-intents-trial-plan/1` under
   `projects/intents-trading-agent/trials/`.
4. If `safe_to_quote=false`, stop.
5. In paper mode, run the returned `build_intent_request`; it uses
   `solver="fixture"`.
6. If the user explicitly requests a live quote later, rerun
   `plan_near_intents_trial` with `mode="quote"` and fresh prices, then
   run the returned `build_intent_request`; it uses
   `solver="near-intents"` and remains unsigned.
7. Write widget state with `format_intents_widget`, including
   `trial_plan`.

## What To Watch

- Very small routes may fail solver minimums. Do not auto-increase size.
- Recompute NEAR/USD before quote mode; the default price is only a
  sizing placeholder.
- Do not use paid research text unless a receipt-backed fetch succeeded.
- Record every quote/intent artifact in the project journal before
  asking the user whether to sign.
