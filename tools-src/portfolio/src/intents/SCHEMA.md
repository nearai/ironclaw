# `portfolio-intent/1` — intent bundle schema

This is the locked-down shape the `portfolio` tool produces from
`build_intent` and that the Money Center (M6) consumes for signing.
It is an **unsigned** artifact — the user's wallet supplies the
signature at sign time.

**Version**: `portfolio-intent/1` (locked as of M4).

## Top-level shape

```json
{
  "id": "q-bridge-2026-04-11-01",
  "legs": [ ... ],
  "total_cost_usd": "15.00",
  "bounded_checks": {
    "min_out_per_leg": [ ... ],
    "max_slippage_bps": 50,
    "solver_quote_version": "near-intents/1"
  },
  "expires_at": 1800000000,
  "signer_placeholder": "<signed-by-user>",
  "schema_version": "portfolio-intent/1"
}
```

Fields:

| Field | Type | Notes |
|---|---|---|
| `id` | string | Stable identifier echoed from the solver quote. |
| `legs` | `IntentLeg[]` | Ordered topologically by `depends_on`. |
| `total_cost_usd` | decimal string | Sum of gas + bridge fees + solver fees in USD. |
| `bounded_checks` | object | The slippage/min-out envelope the bundle was validated against. |
| `bounded_checks.min_out_per_leg` | `TokenAmount[]` | Parallel array to `legs`. |
| `bounded_checks.max_slippage_bps` | u16 | Copied from the user's project config at build time. |
| `bounded_checks.solver_quote_version` | string | Opaque solver version string. |
| `expires_at` | i64 | Unix seconds. After this, the quote is stale and must be rebuilt before signing. |
| `signer_placeholder` | string | Always `"<signed-by-user>"`. Replaced by the wallet in M6. |
| `schema_version` | string | Always `"portfolio-intent/1"` for this version. |

## `IntentLeg`

```json
{
  "id": "bridge-eth-to-base",
  "kind": "bridge",
  "chain": "base",
  "near_intent_payload": { ... },
  "depends_on": "withdraw-eth-aave",
  "min_out": { "symbol": "USDC", "chain": "base", "amount": "4985", "value_usd": "4985.00" },
  "quoted_by": "relay-solver/v1"
}
```

Fields:

| Field | Type | Notes |
|---|---|---|
| `id` | string | Unique within the bundle. |
| `kind` | string | One of: `"withdraw"`, `"bridge"`, `"swap"`, `"deposit"`, `"repay"`, `"rebalance-lp"`. |
| `chain` | string | Destination chain for this leg. |
| `near_intent_payload` | object | Solver-shaped. Opaque to IronClaw; passed through as-is to the wallet. |
| `depends_on` | optional string | Another leg's `id`. Must appear in the same bundle. No cycles. |
| `min_out` | `TokenAmount` | Minimum expected output. Bounded checks refuse if below `(expected_out * (1 - slippage))`. |
| `quoted_by` | string | Solver identifier. For audit. |

## `TokenAmount`

```json
{
  "symbol": "USDC",
  "address": "0xa0b8...",
  "chain": "base",
  "amount": "5000.000000",
  "value_usd": "5000.00"
}
```

Amounts are **decimal strings** (never floats) to avoid precision
loss. `address` may be omitted for native gas tokens.

## Invariants (enforced by `intents/bounded.rs`)

1. `bundle.legs.len() >= 1`.
2. For single-leg bundles, `min_out.value_usd >= expected_out.value_usd * (1 - max_slippage_bps/10_000)`.
3. `total_cost_usd <= plan.expected_cost_usd`.
4. If `config.allowed_chains` is non-empty, every `leg.chain` is in it.
5. `depends_on` must reference a leg in the same bundle; no cycles; no self-loops; no duplicate ids.

## Versioning

A breaking change to any of the above becomes
`portfolio-intent/2`. New fields that older consumers can ignore do
not bump the version. The `schema_version` string is the single
source of truth; never infer compatibility from tool version alone.

## Producers / consumers

- **Producer**: `portfolio` tool's `build_intent` action (this file's
  directory). The only producer.
- **Consumers**:
  - M5 widget (`.system/gateway/widgets/portfolio/`) reads the
    `pending_intents` section of `widgets/state.json`.
  - M6 Money Center's `portfolio_intent_submit` tool reads the
    full bundle, sends it to the wallet for signing, and POSTs the
    signed result to the solver submit endpoint.
  - Tests (replay and live) validate the shape and bounded checks.

Nothing else is allowed to construct `portfolio-intent/1` payloads.
If a new producer emerges, route it through `build_intent` so the
bounded checks run.
