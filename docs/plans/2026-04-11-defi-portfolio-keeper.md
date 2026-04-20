# Portfolio — Full Plan

**Status**: draft
**Owner**: tbd
**Date**: 2026-04-11
**Target**: user pastes a wallet address into chat → IronClaw discovers cross-chain DeFi positions, scores them, suggests improvements with projected Δ earnings, constructs unsigned NEAR Intents to execute, runs as a recurring mission, and projects a live widget. Extensible by user via custom scripts. Viral-ready.

---

## 0. Guiding principles

1. **Nothing DeFi-specific is hardcoded in the IronClaw core.** All protocol and strategy knowledge lives as data (JSON/Markdown) embedded in the `portfolio` WASM tool or authored in the project workspace. Adding a new protocol or strategy is a PR to a data file, not to Rust.
2. **Agent never holds private keys.** The only execution path is the construction of *unsigned NEAR Intents*. Signing happens in the user's wallet (Phase 6).
3. **NEAR Intents is the only movement primitive.** No raw EVM tx building. If a route isn't reachable via a solver, the suggestion is surfaced as an `unmet-route`, not built from scratch.
4. **Project-scoped.** The whole capability lives under a v2-engine `Project`. Files, mission, memory docs, scripts, and widget state are all scoped to that project. A user can have many projects (`portfolio`, `portfolio-treasury`, `portfolio-dao`) that share the same skill/tool/registries.
5. **Deterministic and replayable.** The tool's scan operation accepts `at_block` / `at_timestamp`. Every mission run is reproducible against a pinned historical state. Test fixtures are recorded, not synthesized.
6. **LLM data is never deleted.** State snapshots, suggestions, and intents are retained forever in the project workspace. They are the backtest corpus.
7. **Everything through dispatch.** All mutations go through `ToolDispatcher::dispatch()` for audit, redaction, and safety.
8. **One tool, one trust boundary.** The entire capability is a single `portfolio` WASM tool exposing multiple operations. Simpler install, versioning, capabilities, and auditing than a fleet of coupled micro-tools.
9. **YAGNI for pluggability.** Dune REST is hardcoded as the v1 data source. No sources registry, no transport abstraction, no escape hatches until a second source actually shows up.

## 1. Architecture overview

```
 ┌────────────────────────────────────────────────────────────────┐
 │                      skills/portfolio/                         │
 │  SKILL.md — activation on address/keywords, requires the       │
 │  `portfolio` WASM tool, instructs LLM on project bootstrap,    │
 │  mission creation, script authoring, widget writing, and       │
 │  suggestion format.                                            │
 └────────────────────────────┬───────────────────────────────────┘
                              │
                              ▼
            ┌───────────────────────────────────┐
            │   WASM tool: portfolio            │
            │                                   │
            │   operations:                     │
            │     scan(addr, at?)               │
            │     propose(positions, cfg)       │
            │     build_intent(plan, cfg)       │
            │                                   │
            │   internal:                       │
            │     ├── indexer (Dune REST)       │
            │     ├── analyzer (protocols/*)    │
            │     ├── strategy (strategies/*)   │
            │     └── intents (solver client)   │
            └──────┬────────────┬───────────────┘
                   │            │
            Dune REST API   NEAR Intents solver
            (HTTP allowlisted; key via CredentialInjector)

                    ▼
            v2 engine Project: "portfolio"
              └── Mission: portfolio-keeper  (cron 0 */6 * * *)
                    └── workspace://projects/<id>/...
                           ├── addresses.md
                           ├── strategies/*.md
                           ├── config.json
                           ├── state/{latest.json, history/*.json}
                           ├── suggestions/*.md
                           ├── intents/*.json
                           ├── scripts/*.py         (user custom flows)
                           └── widgets/state.json   (web widget data)
```

**Count**: one WASM tool. Two embedded registries (`protocols/`, `strategies/`). One skill. One mission. One widget. One Python-scripting extensibility seam. One final wallet UI phase.

## 2. Components

### 2.1 Skill: `skills/portfolio/SKILL.md`

**Frontmatter:**
```yaml
name: portfolio
version: 0.1.0
description: Cross-chain DeFi portfolio discovery, rebalancing suggestions, and
             NEAR Intent execution. Runs as a recurring mission per project.
activation:
  keywords: [portfolio, defi, yield, apy, rebalance, positions, wallet, farming,
             stake, lending, liquidity]
  patterns:
    - '0x[a-fA-F0-9]{40}'              # EVM address
    - '[a-zA-Z0-9_-]+\.near'           # NEAR account
    - '[a-zA-Z0-9-]+\.eth'             # ENS
  exclude_keywords: [nft, mint]
  tags: [crypto, defi, finance]
  max_context_tokens: 4000
requires:
  tools:
    - portfolio
  env:
    - DUNE_API_KEY                      # optional; fixture backend works without
  config:
    project_bootstrap: true             # skill is allowed to create a project
```

**Body (markdown instructions to the LLM):**

A short playbook:

1. **Project bootstrap.** If no `portfolio` project exists for this user, create one via `project_create(name="portfolio", description=...)`. **Only create additional portfolio projects when the user explicitly asks** ("create a separate treasury portfolio", "track this one under a different project"). Otherwise append to the existing default `portfolio` project — multiple wallets live inside one project by default.
2. **Address capture.** Append to `projects/<id>/addresses.md` using `memory_write`. Multiple addresses are the norm, not the exception. Never store addresses outside the project.
3. **Config defaults.** On first run, write `config.json` with sensible defaults: `{floor_apy: 0.04, max_risk_score: 3, notify_threshold_usd: 100, auto_intent_ceiling_usd: 1000, max_slippage_bps: 50}`.
4. **Scan.** Call `portfolio.scan(addresses, at: null)`. Returns `ClassifiedPosition[]`.
5. **Propose.** Call `portfolio.propose(positions, strategies, config)`. Returns deterministic candidate `Proposal[]` filtered by strategy constraints.
6. **Rank.** LLM ranks/prunes the candidate set using each proposal's `rationale` and the strategy doc prose.
7. **Build.** For each top-ranked proposal, call `portfolio.build_intent(plan, config)` to produce an unsigned NEAR Intent bundle. Bounded checks must pass before the intent is written.
8. **Persist.** Write `state/latest.json`, `state/history/<date>.json`, `suggestions/<date>.md`, `intents/<ts>-<label>.json`, and `widgets/state.json`.
9. **Summarize.** Return a concise table to the channel: top 3 proposals, projected Δ APY, total projected annual gain, a reference to the widget view.
10. **Mission offer.** If no keeper mission exists yet, offer to create one (`mission_create` with `project_id` scope). Do not create it automatically on the first interaction — ask first.
11. **Scripts awareness.** If `projects/<id>/scripts/` contains any `.py` files, list them in the response. If the user asks for a custom alert/report/backtest, author a new script in that folder (see §2.7).

**Trust model**: the skill ships in-tree (trusted), so it has full access to the `portfolio` tool. Installed-from-registry variants (future community strategies) would be read-only.

### 2.2 WASM tool: `portfolio`

A single tool with multiple operations. Internal module layout separates concerns, but consumers see one tool with one capability file and one install target.

**Operations:**

```
scan(
  addresses: list<Address>,
  chains: "*" | list<ChainId>,
  at: Option<{ block?: map<ChainId, u64>, timestamp?: i64 }>,
) -> Result<list<ClassifiedPosition>, ScanError>

propose(
  positions: list<ClassifiedPosition>,
  strategies: list<StrategyDoc>,
  config: ProjectConfig,
) -> list<Proposal>

build_intent(
  plan: MovementPlan,
  config: ProjectConfig,
) -> Result<IntentBundle, BuildError>
```

`scan` returns already-classified positions — the indexer and analyzer are internal stages of one operation. Callers don't need to sequence them.

**Internal module layout** (`tools-src/portfolio/src/`):

```
src/
├── lib.rs              # WIT bindings, operation dispatch
├── indexer/
│   ├── mod.rs          # scan entry point
│   ├── dune.rs         # Dune REST client (hardcoded)
│   └── fixture.rs      # fixture-backed scan for tests
├── analyzer/
│   ├── mod.rs          # classify() — raw -> ClassifiedPosition
│   └── registry.rs     # loads protocols/*.json via include_dir!
├── strategy/
│   ├── mod.rs          # propose() entry point
│   ├── parser.rs       # YAML frontmatter + body parser
│   └── filter.rs       # deterministic constraint filter
├── intents/
│   ├── mod.rs          # build_intent() entry point
│   ├── solver.rs       # NEAR Intents solver client
│   ├── bundling.rs     # multi-leg ordering
│   └── bounded.rs      # min_out / slippage / expiry checks
├── types/
│   ├── position.rs     # RawPosition, ClassifiedPosition
│   ├── proposal.rs     # Proposal, MovementPlan
│   └── intent.rs       # IntentBundle, IntentLeg
└── protocols/          # embedded via include_dir!
    ├── aave-v3.json
    ├── compound-v3.json
    ├── uniswap-v3.json
    ├── lido.json
    └── morpho-blue.json
```

**Why one tool, not four:**
- All four internal stages share types. Splitting them into separate WASM tools means those types must live in a shared crate (`ironclaw_defi_types`) or be re-serialized at each boundary. Both are overhead without upside.
- They are always co-installed. A user with `portfolio_indexer` but not `strategy_engine` is broken.
- One capabilities file, one credentials scope, one version bump, one audit trail class.
- Internal module boundaries still enforce clean separation — this is "small crates vs. workspace" at the WASM-tool scale, and the small-crates option wasn't earning its keep.

**Capabilities** (`portfolio.capabilities.json`):
- HTTP allowlist:
  - `api.dune.com` (path prefix `/api/v1/`, `/api/beta/sim/`).
  - NEAR Intents solver endpoint(s).
- Credentials:
  - `X-Dune-API-Key: ${DUNE_API_KEY}` — injected at host boundary.
- Workspace read: `projects/*/strategies/**` (so `propose` can load strategy docs the skill passes by path).
- Workspace write: none. All writes are done by the skill playbook via `memory_write`.
- Rate limit: 30 req/min on Dune endpoints.

### 2.3 Indexer stage (`indexer/dune.rs`)

Direct Dune REST client. No MCP. No transport abstraction.

**Endpoints used** (pin exact versions in implementation):
- Sim balances endpoint for multi-chain token balances.
- Sim positions / activity endpoint for DeFi positions (lending, LP, staking).
- Named SQL queries (pinned by query ID) for anything the Sim API doesn't natively cover — e.g., Morpho Blue market-scoped positions. Named queries are stored in the `dune.rs` module as constants so they're visible in code review.

**Historical queries**: `at.block` compiles down to a `block_number` query param where Dune supports it, or to a SQL parameter on a named query. If a given endpoint doesn't support history for a chain, `scan` returns `ScanError::HistoryNotSupported { chain, reason }` so tests fail loudly instead of silently returning current state.

**Fixture mode**: when `PORTFOLIO_INDEXER_BACKEND=fixture`, the indexer reads canned normalized responses from `fixtures/portfolio_indexer/{hash(address, chains, at)}.json` instead of calling Dune. Used in all replay scenarios and CI. Recording is one-shot via `RECORD=1 cargo test <scenario>`.

**Output**: internal `RawPosition[]` which is immediately handed to the analyzer. The scan operation returns `ClassifiedPosition[]`.

### 2.4 Analyzer stage (`analyzer/`)

Classifies raw positions into a typed, yield-aware shape using `protocols/*.json` embedded via `include_dir!`.

**Output shape:**
```
ClassifiedPosition {
  protocol: ProtocolRef,
  category: "lending" | "dex-lp" | "staking" | "restaking" | "vault"
          | "perps-collateral" | "stablecoin-idle" | "wrapped" | "other",
  principal_usd: Decimal,
  debt_usd: Decimal,
  net_yield_apy: Decimal,
  unrealized_pnl_usd: Decimal,
  risk_score: u8,                     // 1-5
  exit_cost_estimate_usd: Decimal,
  withdrawal_delay_seconds: u64,
  liquidity_tier: "instant" | "minutes" | "hours" | "days" | "epoch",
  health: Option<HealthMetric>,       // e.g. Aave health factor
  tags: list<string>,
  raw_position: RawPosition,          // kept for strategy engine
}
```

**Protocol registry entry** (`protocols/aave-v3.json`):
```json
{
  "id": "aave-v3",
  "name": "Aave v3",
  "category": "lending",
  "chains": ["ethereum", "base", "arbitrum", "optimism", "polygon"],
  "contract_addrs_by_chain": { "ethereum": ["0x87870Bca..."], "base": ["0xA238..."] },
  "position_detector": {
    "matches": [
      { "field": "protocol_id", "equals": "aave_v3" },
      { "field": "raw_metadata.pool_contract", "equals_any_of": "$.contract_addrs_by_chain[$chain]" }
    ]
  },
  "yield_model": {
    "type": "variable_apy",
    "supply_apy_field": "$.raw_metadata.supplyAPY",
    "borrow_apy_field": "$.raw_metadata.borrowAPY",
    "net_formula": "principal_usd * supply_apy - debt_usd * borrow_apy"
  },
  "risk": {
    "base_score": 2,
    "audits": ["OpenZeppelin", "Trail of Bits", "Certora"],
    "tvl_floor_usd": 1000000000,
    "oracle_deps": ["Chainlink"],
    "composability_tags": ["aave-atoken", "borrowable-collateral"]
  },
  "fee_model": { "entry_bps": 0, "exit_bps": 0 },
  "withdrawal_delay_seconds": 0,
  "liquidity_tier": "instant",
  "health_metric": {
    "name": "health_factor",
    "formula": "totalCollateralETH * avgLiquidationThreshold / totalDebtETH",
    "danger_threshold": 1.1
  }
}
```

**Embedded protocol set at M2** (5 protocols, common-enough to cover most wallets):
1. Aave v3 (lending)
2. Compound v3 (lending)
3. Uniswap v3 (LP)
4. Lido stETH (staking)
5. Morpho Blue (lending)

**Adding a protocol** = PR a JSON file into `tools-src/portfolio/src/protocols/` and rebuild the WASM tool.

### 2.5 Strategy stage (`strategy/`)

Reads `ClassifiedPosition[]` + user strategy docs + config, emits ranked `Proposal[]`.

**Strategies are declarative Markdown with YAML frontmatter.** Not Rust. Not Python. Just docs the tool parses.

**Proposal shape:**
```
Proposal {
  id: string,                          // deterministic hash(inputs)
  strategy_id: string,
  from_positions: list<PositionRef>,
  to_protocol: ProtocolRef,
  movement_plan: MovementPlan,         // legs: withdraw → bridge → deposit
  projected_delta_apy_bps: i32,
  projected_annual_gain_usd: Decimal,
  confidence: f32,                     // 0-1
  risk_delta: i8,
  cost_breakdown: {
    gas_estimate_usd: Decimal,
    bridge_fee_estimate_usd: Decimal,
    solver_fee_estimate_usd: Decimal,
    slippage_budget_usd: Decimal,
  },
  gas_payback_days: f32,
  rationale: string,                   // for suggestion md
  status: "ready" | "unmet-route" | "below-threshold" | "blocked-by-constraint",
}
```

**Strategy doc example** (`projects/<id>/strategies/stablecoin-yield-floor.md`):
```markdown
---
id: stablecoin-yield-floor
version: 1
applies_to:
  category: stablecoin-idle
  min_principal_usd: 100
constraints:
  min_projected_delta_apy_bps: 50        # 0.5%
  max_risk_score: 3
  max_bridge_legs: 1
  gas_payback_days: 30
  prefer_same_chain: true
  prefer_near_intents: true
inputs:
  floor_apy: 0.04
---

# Stablecoin Yield Floor

Keep idle stablecoins at or above `floor_apy` net APY. For any qualifying
position yielding below that, propose the highest-net-APY alternative
within the risk budget, preferring same-chain moves, then single-bridge
moves via NEAR Intents.

Do not propose a move whose gas + bridge + slippage cost would take more
than `gas_payback_days` to recoup at the Δ APY.
```

**Two-stage processing:**
1. **Deterministic constraint filter** (inside the WASM tool): parses YAML, filters positions the strategy applies to, enumerates candidate destinations, computes projected Δ APY and cost, discards anything violating constraints. Output: a candidate `Proposal[]` per strategy, each with a `status` and `rationale`.
2. **LLM ranking** (back in the skill playbook): the LLM receives the candidate set and the strategy's prose body, and ranks/prunes using reasoning the YAML can't capture. The WASM tool itself never calls an LLM.

**Ships with** at M3, embedded in `tools-src/portfolio/strategies/` as defaults the skill copies into new projects on bootstrap:
- `stablecoin-yield-floor.md`
- `lending-health-guard.md`
- `lp-impermanent-loss-watch.md`

### 2.6 Intents stage (`intents/`)

Translates a `MovementPlan` into one or more **unsigned NEAR Intent bundles**. Intent-only, no raw EVM tx.

**Output shape:**
```
IntentBundle {
  id: string,                          // hash(plan)
  legs: list<IntentLeg>,               // ordered; each is a solver-quoted intent
  total_cost_usd: Decimal,
  bounded_checks: {
    min_out_per_leg: list<TokenAmount>,
    max_slippage_bps: u16,
    solver_quote_version: string,
  },
  expires_at: i64,                     // solver quote TTL
  signer_placeholder: "<signed-by-user>",
  schema_version: "portfolio-intent/1",
}

IntentLeg {
  kind: "swap" | "bridge" | "deposit" | "withdraw",
  chain: ChainId,
  near_intent_payload: Json,           // solver-shaped, signable as-is
  depends_on: Option<string>,          // previous leg id
  min_out: TokenAmount,
  quoted_by: string,                   // solver identifier
}
```

**Solver integration:**
- Primary: NEAR Intents solver quote API, called via HTTP with allowlisted host.
- Each leg gets a fresh quote at build time. Quote is re-verified against user's `max_slippage_bps` before the intent is written.
- If no solver route exists, `build_intent` returns `BuildError::NoRoute { from, to }` and the skill playbook marks the proposal's `status` as `unmet-route`. The mission logs the opportunity for future runs.

**Bounded checks (mandatory before write):**
- `min_out >= plan.expected_out * (1 - config.max_slippage_bps/10000)`
- `total_cost_usd <= proposal.cost_breakdown.total`
- `expires_at > now + 5min` (leave signing headroom)
- Each leg's chain ∈ user's `config.allowed_chains` (defaults to all).

**Output is written** by the skill playbook to `projects/<id>/intents/<ts>-<strategy>-<proposal_id>.json`. Skill references it by ID in the chat summary.

### 2.7 User-authored scripts (Monty/Python)

The engine has Tier-1 Python scripting via Monty (`crates/ironclaw_engine/src/executor/scripting.rs`). Reuse it instead of building a macro system.

- **Location**: `projects/<id>/scripts/*.py`.
- **Discovery**: skill lists available scripts in LLM context on each activation.
- **Authoring UX**: user says *"alert me when my Aave health factor drops below 1.5"* → LLM writes `scripts/alert_aave_health.py`, drops it in the folder, and (if requested) wires it into a sub-mission with its own cadence.
- **What scripts can do**: call `portfolio.*` operations via `tool_invoke`, read/write project workspace, call `llm_query()` for sub-reasoning, emit messages via the channel.
- **Safety**: Monty runs in the existing sandbox with the skill's tool ceiling. No fresh powers.
- **Example starter scripts documented in the skill prose**:
  - `alert_if_health_below.py` — health factor watchdog.
  - `weekly_report.py` — 7-day yield attribution from `state/history/*.json`.
  - `backtest_strategy.py` — replay a strategy against history, report projected vs realized.
  - `concentration_warning.py` — warn if >X% of portfolio is on one chain or in one protocol.

### 2.8 Mission: `portfolio-keeper`

One mission, scoped to the project, using v2 engine missions (not v1 routines).

```
name:         portfolio-keeper
project_id:   <portfolio project id>
cadence:      Cron { expression: "0 */6 * * *", timezone: user tz }
goal:         "Keep this project's DeFi portfolio at or above the declared
               yield floor, within the declared risk budget, while minimizing
               realized gas and bridge costs. Surface actionable suggestions
               every run and build NEAR Intents for any proposal exceeding
               the notify threshold."
context_paths:
  - projects/<id>/addresses.md
  - projects/<id>/strategies/**
  - projects/<id>/config.json
  - projects/<id>/state/latest.json
progress_metric:
  name: realized_net_apy_7d_vs_floor
  formula: (realized_net_apy_7d - config.floor_apy) / config.floor_apy
  target: ">= 0"
writes:
  - projects/<id>/state/latest.json
  - projects/<id>/state/history/<date>.json
  - projects/<id>/suggestions/<date>.md
  - projects/<id>/intents/*.json  (when applicable)
  - projects/<id>/widgets/state.json
notify:
  - condition: any proposal with projected_annual_gain_usd > config.notify_threshold_usd
    channel: user's default channel
idempotency:
  content_hash: sha256(state/latest.json minus timestamps)
  if unchanged: no-op after scan, no new suggestion file
```

Runs a full thread so it has access to all tools including `portfolio.*`.

**Learner mission deferred.** The keeper's goal language already includes the learning intent. A separate learner mission was premature — revisit after M5 when we have real suggestion-vs-outcome data to learn from.

### 2.9 Project-scoped workspace

```
projects/<project_id>/
├── addresses.md                # user-maintained; skill appends on first capture
├── config.json                 # risk tolerance, thresholds, slippage
├── strategies/
│   ├── stablecoin-yield-floor.md
│   ├── lending-health-guard.md
│   ├── lp-impermanent-loss-watch.md
│   └── <user-authored>.md
├── state/
│   ├── latest.json             # ClassifiedPosition[] + totals
│   └── history/
│       └── 2026-04-11.json     # never deleted; the backtest corpus
├── suggestions/
│   └── 2026-04-11.md           # human-readable proposal table
├── intents/
│   └── 2026-04-11T12-00-stablecoin-yield-floor-abc123.json
├── scripts/
│   └── *.py                    # user-authored custom flows
└── widgets/
    └── state.json              # render-ready view model for the widget
```

**Why history is valuable**: it *is* the local time series. Custom scripts can backtest new strategies against it without calling Dune.

### 2.10 Web widget

Lives at `.system/gateway/widgets/portfolio/manifest.json`. Served by the existing `/api/frontend/widgets` endpoint.

**Data flow**: the keeper mission writes `projects/<id>/widgets/state.json` on every run. The widget polls (or subscribes via SSE if available) and renders.

**`widgets/state.json` schema** (render-ready view model):
```json
{
  "generated_at": "2026-04-11T12:00:00Z",
  "project_id": "...",
  "totals": {
    "net_value_usd": "12345.67",
    "realized_net_apy_7d": 0.048,
    "floor_apy": 0.04,
    "delta_vs_last_run_usd": "+34.12",
    "risk_score_weighted": 2.3
  },
  "positions": [
    {
      "protocol": "Aave v3",
      "chain": "base",
      "category": "lending",
      "principal_usd": "5000.00",
      "net_apy": 0.053,
      "risk_score": 2,
      "health": { "name": "health_factor", "value": 2.1, "warning": false },
      "tags": ["stablecoin", "core"]
    }
  ],
  "top_suggestions": [
    {
      "id": "abc123",
      "strategy": "stablecoin-yield-floor",
      "rationale": "USDC on Ethereum Aave @ 3.8% → Morpho Blue on Base @ 5.4%",
      "projected_delta_apy_bps": 160,
      "projected_annual_gain_usd": "48.30",
      "gas_payback_days": 12,
      "intent_id": "2026-04-11T12-00-stablecoin-yield-floor-abc123",
      "status": "ready"
    }
  ],
  "pending_intents": [
    { "id": "...", "status": "awaiting-signature", "legs": 2, "expires_at": "..." }
  ],
  "next_mission_run": "2026-04-11T18:00:00Z",
  "progress_metric": { "name": "realized_net_apy_7d_vs_floor", "value": 0.20 }
}
```

**Rendering**: the widget framework is declarative layout. Portfolio view = stacked layout of: totals card → positions table → suggestions list → pending intents list → "last updated / next run" footer. No bespoke React/iframe work.

**M6 addition**: suggestions gain a "Sign" button that triggers `portfolio_intent_submit` (see §2.11).

### 2.11 Built-in tool (M6): `portfolio_intent_submit`

Not WASM — lives in `src/tools/builtin/portfolio.rs` because it interacts with the web channel and the wallet UI.

**Interface:**
```
portfolio_intent_submit(intent_id: string) -> SubmitResult
```

**Flow:**
1. Reads `projects/<id>/intents/<intent_id>.json`.
2. Marks it `awaiting-signature` and emits a SSE event (`portfolio.intent.awaiting_signature`) that the web widget picks up.
3. Web UI opens wallet modal (WalletConnect / NEAR wallet selector) with the intent payload.
4. User signs. Signed payload POSTed back to `/api/frontend/portfolio/submit/:intent_id`.
5. Tool submits signed intent to the solver's submit endpoint, records tx hash, updates intent file with `status: submitted, signed_payload_hash, tx_hash`.
6. Written back through `ToolDispatcher::dispatch()`.

Until M6, this tool doesn't exist and intents are pasted into the user's own wallet manually.

## 3. Data source: Dune REST API

### 3.1 Why direct HTTP, not MCP

- One less moving part. No user-facing MCP server registration step.
- Single trust boundary: the `portfolio` WASM tool's capability file.
- `DUNE_API_KEY` goes through the existing `CredentialInjector` — tool code never sees the raw value.
- Leak detection and HTTP allowlisting already cover this path.
- If MCP later proves a better fit (e.g. for historical queries via Dune's SQL surface), the `indexer/` module can add it as an alternative transport without changing the tool's public operations.

### 3.2 Endpoint inventory (pinned at implementation time)

- Sim balances — multi-chain token balances for an address.
- Sim positions / activity — DeFi positions (lending, LP, staking).
- Named SQL queries (by query ID) — anything Sim doesn't cover directly; stored as module-level constants in `indexer/dune.rs` so they're visible in code review and diffable.

All endpoints pinned to a specific API version. Upgrade path: bump the version in one place, re-record fixtures, run replay suite.

### 3.3 Historical queries

`scan(at: ...)` compiles to Dune's `block_number` query param where supported, or to a SQL parameter on a named query. Chains/endpoints without history support return `ScanError::HistoryNotSupported { chain, reason }` so tests fail loudly rather than silently returning current state.

### 3.4 Rate limiting

30 req/min declared in the capabilities file. Skill playbook batches per-address scans and caches within a mission run.

## 4. Testing system

### 4.1 Layers

1. **Unit tests** (inside `tools-src/portfolio/`):
   - `strategy::filter` on synthetic positions — deterministic, no LLM.
   - `analyzer` JSON registry → `ClassifiedPosition` mapping per embedded protocol.
   - `intents::bundling` multi-leg ordering.
   - `intents::bounded` min_out/slippage/expiry enforcement on mocked solver responses.
   - `indexer::dune` response-projection tests against recorded fixtures.
2. **Integration tests** (`cargo test --features integration`): drive through `MissionManager::fire_mission()` rather than individual tool helpers. Honors the "test through the caller" rule.
3. **Replay scenarios** (see 4.2): the main regression suite.
4. **Live tests** (`#[ignore]`): one scenario per milestone that hits real Dune + real solver. Run manually, not CI.

### 4.2 Replay harness

`tests/portfolio_replay.rs` (`--features integration`).

**Scenario format** (`tests/portfolio_scenarios/*.yaml`):
```yaml
id: bull-2024-03
description: Mid-2024 portfolio with idle USDC and a profitable Lido position
inputs:
  addresses:
    - "0xAbC...1"
  at_block:
    ethereum: 19500000
    base: 12000000
  now: "2026-04-11T12:00:00Z"
  random_seed: 42
  llm:
    mode: mock            # or "record" or "live"
    fixture: llm/bull-2024-03.jsonl
  indexer:
    backend: fixture
  solver:
    backend: fixture
    fixture: solver/bull-2024-03.json
project_config:
  floor_apy: 0.04
  max_risk_score: 3
  notify_threshold_usd: 100
expectations:
  state_latest:
    position_count: ">=3"
    contains_protocols: ["aave-v3", "lido", "uniswap-v3"]
    total_value_usd: "between:15000:25000"
  suggestions:
    min_count: 1
    top_proposal:
      strategy_id: stablecoin-yield-floor
      projected_delta_apy_bps: ">=50"
      status: ready
  intents:
    min_count: 1
    all:
      schema_version: portfolio-intent/1
      bounded_checks_passed: true
  widget_state:
    schema_valid: true
    top_suggestions_len: ">=1"
  no_leaks: true           # leak detector scan over all emitted text
  snapshot:
    suggestions_md: snapshots/bull-2024-03.suggestions.md
    widget_json:    snapshots/bull-2024-03.widget.json
```

**Harness steps per scenario:**
1. Spin up a temp project workspace.
2. Load LLM/indexer/solver fixtures; set clock to `now`; seed RNG.
3. Create the project and the keeper mission.
4. Fire the mission once via `MissionManager::fire_mission(mission_id)`.
5. Assert against written workspace files.
6. Snapshot-test `suggestions/*.md` and `widgets/state.json` (via `insta`).
7. Run the leak detector over every file and every SSE event emitted during the run.

### 4.3 Recording fixtures

Three fixture families, all recorded by flipping `RECORD=1` and running the same scenario:

- `fixtures/portfolio_indexer/<hash>.json` — recorded Dune REST responses, keyed by `(address, chains, at)`.
- `fixtures/solver/<scenario>.json` — solver quote responses, keyed by scenario.
- `fixtures/llm/<scenario>.jsonl` — recorded LLM exchanges for the mission thread. Replayed in `mode: mock` scenarios so tests are hermetic.

Recording is one-shot: `RECORD=1 cargo test <scenario>`, commit the new fixtures. Replay mode (`mode: mock`) is the default CI path.

### 4.4 Scenario catalog (ships incrementally across milestones)

| Scenario | Milestone | Purpose |
|---|---|---|
| `smoke-empty-wallet` | M1 | mission runs end-to-end, no crashes, empty state |
| `smoke-single-usdc` | M1 | one position, one strategy, one suggestion |
| `bull-2024-03` | M2 | real multi-protocol address recorded from Dune |
| `stale-aave-rebalance` | M3 | triggers stablecoin-yield-floor strategy |
| `lending-health-warning` | M3 | health factor near danger threshold |
| `idempotent-rerun` | M3 | run twice with no state change, second run is no-op |
| `bridge-opportunity` | M4 | cross-chain move via NEAR Intents |
| `hostile/fake-token-dust` | M4 | spam tokens in wallet, must be ignored |
| `hostile/malicious-protocol` | M4 | unknown protocol, must be skipped not crash |
| `hostile/solver-bad-quote` | M4 | solver returns quote outside slippage, must refuse |
| `widget-shape` | M5 | widget JSON schema and snapshot stability |
| `custom-script-alert` | M5 | user-authored script triggers on synthetic health drop |
| `backtest-strategy` | M5 | custom script replays a strategy against `state/history` |
| `sign-flow-stub` | M6 | `portfolio_intent_submit` happy path (wallet mocked) |

### 4.5 Performance & flake budget

- Every replay scenario must complete in <10s on CI (mock LLM, fixture backends).
- Live tests are `#[ignore]` and not counted toward CI budget.
- Any test calling a real network is quarantined behind a `live` feature flag.

## 5. Security model

### 5.1 Keys, secrets, redaction

- Agent never handles private keys, mnemonics, or signed payloads in plaintext log surfaces. Leak detector's existing mnemonic/private-key patterns apply; verify coverage and extend if needed.
- `DUNE_API_KEY` and future solver auth tokens flow through `CredentialInjector`. Tool code never sees raw values.
- Wallet addresses are public and are **not** redacted, but are tagged in logs so audit queries can filter by address.

### 5.2 Dispatch discipline

- Every workspace write, tool call, and mission action goes through `ToolDispatcher::dispatch()`. No direct access to `state.store` / `workspace` from the skill or Monty scripts.
- The skill's playbook is explicit about this — no "shortcut" language.
- Pre-commit hook continues to guard against regressions.

### 5.3 Signing boundary

- Until M6: agent only writes unsigned intent files.
- At M6: `portfolio_intent_submit` is the *only* tool that touches signed payloads, and only at the web-channel boundary. Signing itself happens in the user's wallet.
- Signed payloads are stored hashed (for audit) but the raw signed blob is discarded after submission.

### 5.4 Destructive action gate

- Any proposal with `total_cost_usd > config.auto_intent_ceiling_usd` requires an explicit user confirmation in chat before the intent file is materialized. Mission cannot self-confirm.
- The confirmation is a tool call (`portfolio_confirm_proposal(id)`), not a free-text "yes". Makes the approval auditable.

### 5.5 Solver trust

- Solver quotes are never trusted blindly. `build_intent` enforces:
  - `min_out >= expected * (1 - max_slippage_bps/10000)`
  - `total_cost_usd <= proposal.cost_breakdown.total`
  - `expires_at > now + 5min`
  - Chain allowlist from `config.allowed_chains`
- Violations produce `BuildError::QuoteOutOfBounds`. Suggestion is degraded to `unmet-route` and logged.

### 5.6 Sandbox inheritance

- `portfolio` WASM tool runs in the existing wasmtime sandbox with fuel metering, memory limits, HTTP allowlists, and credential injection.
- Monty scripts run in the existing Python sandbox.
- Neither gets new powers.

## 6. Virality hooks

Assumes the user is logged in — no anonymous/ephemeral flow in v1. Hooks are still cheap because the widget and skill do most of the work.

1. **Paste-address onboarding** — a logged-in user pastes any wallet address into any channel, the skill bootstraps the default portfolio project on the fly, scan runs, widget renders. First-use latency is the dominant UX signal.
2. **Shareable snapshot URL** — the widget has a "share" action that produces a read-only link to a frozen view of `widgets/state.json`, served from the web channel's static asset dir. The snapshot excludes addresses unless the user explicitly opts in ("include wallets in share").
3. **"Add to my portfolio"** — any address surfaced in chat (e.g. a friend shared theirs, or one came up in research) gets a one-tap "add to my portfolio" action that appends it to the default project's `addresses.md` and re-scans.
4. **Low friction by design** — because M1–M5 never require signing, onboarding is "paste address." Users get value before the wallet UI exists in M6.

## 7. Milestones

Each milestone ends with (a) green `cargo fmt && cargo clippy && cargo test --features integration`, (b) at least one new replay scenario, (c) updated docs.

### M1 — Skeleton + fixture path

**Goal**: the full pipeline runs end-to-end against canned fixtures. No real network.

**Deliverables:**
- `skills/portfolio/SKILL.md` with frontmatter + playbook.
- `tools-src/portfolio/` WASM tool with all three operations (`scan`, `propose`, `build_intent`) and internal stages wired.
- `indexer/fixture.rs` fixture backend (only).
- `analyzer/` with one synthetic `test-lending` protocol JSON.
- `strategy/` with one `stablecoin-yield-floor.md` doc embedded as default.
- `intents/` with a fixture solver (no HTTP yet).
- Project bootstrap path: on skill activation with no project, create one.
- Mission creation via `mission_create` scoped to the new project.
- Workspace writes: `state/`, `suggestions/`, `intents/`, `widgets/state.json`.
- Replay scenarios: `smoke-empty-wallet`, `smoke-single-usdc`.
- Integration test: fire the mission through `MissionManager`.

**Exit criteria:**
- `cargo test --features integration portfolio::smoke` passes.
- Widget JSON validates against schema.
- No hardcoded DeFi knowledge outside the `portfolio` tool's internal data files.

**Risks/unknowns:**
- v2 engine's `project_create` tool surface may need a minor addition if it doesn't expose `description`. Verify first.
- Skill can list script files without running them, so M1 doesn't need Monty execution.

### M2 — Dune REST integration + real protocols

**Goal**: real data on real addresses.

**Deliverables:**
- `indexer/dune.rs` Dune REST client with pinned endpoints.
- HTTP allowlist + `DUNE_API_KEY` credential injection in `capabilities.json`.
- 5 real protocol JSONs: Aave v3, Compound v3, Uniswap v3 LP, Lido, Morpho Blue.
- Fixture recording harness: `RECORD=1 cargo test bull-2024-03`.
- Record fixtures for `bull-2024-03` scenario from a real public address.
- Live test (`#[ignore]`) that runs against real Dune.
- Classifier correctness tests per protocol.

**Exit criteria:**
- `bull-2024-03` replay scenario passes against recorded fixtures.
- Live test passes manually against real Dune REST.
- Protocol registry has 5 JSON files, each with audit metadata and yield model.

**Risks/unknowns:**
- Dune endpoint surface for DeFi positions — confirm coverage for the 5 protocols. For anything Sim doesn't cover, fall back to named SQL queries and pin the query IDs.
- Historical query support may be patchy across chains; scenario fixtures hide this in CI but the live test will reveal it.

### M3 — Strategy engine + mission goal/progress

**Goal**: the mission produces *useful* suggestions with measurable progress.

**Deliverables:**
- Strategy constraint filter, YAML frontmatter parser, candidate enumeration (inside `strategy/`).
- Three shipped strategy docs: `stablecoin-yield-floor.md`, `lending-health-guard.md`, `lp-impermanent-loss-watch.md`.
- Mission progress metric (`realized_net_apy_7d_vs_floor`) written to project memory on each run.
- Mission idempotency: content-hash of state, skip suggestion write if unchanged.
- Replay scenarios: `stale-aave-rebalance`, `lending-health-warning`, `idempotent-rerun`.
- Snapshot tests for `suggestions/*.md` format.

**Exit criteria:**
- Three new scenarios pass.
- Progress metric visible in mission history.
- Rerunning the mission with identical state produces no new suggestion file.
- Rejected proposals produce deterministic `blocked-by-constraint` entries with clear reasons.

**Risks/unknowns:**
- LLM ranking stability in snapshot tests — solve by using mock-mode LLM fixtures in CI, live-mode only manually.
- Cost estimation accuracy (gas, bridge, slippage) — ship best-effort estimates; M4 hardens them.

### M4 — NEAR Intents builder + hostile scenarios

**Goal**: every ready suggestion produces a validated, signable intent package.

**Deliverables:**
- `intents/solver.rs` NEAR Intents solver quote integration (allowlisted HTTP).
- Multi-leg bundling with `depends_on` ordering.
- Bounded-check enforcement.
- `unmet-route` status for uncovered pairs.
- Replay scenarios: `bridge-opportunity`, `hostile/fake-token-dust`, `hostile/malicious-protocol`, `hostile/solver-bad-quote`.
- Live test against real NEAR Intents solver for one scenario.
- Intent file schema v1 locked down and documented.

**Exit criteria:**
- All M4 scenarios pass.
- Adversarial tests confirm: dust ignored, unknown protocols skipped (not crash), solver out-of-bounds quotes refused.
- Intent files validate against schema and can be round-tripped by an external signer tool (manual check).

**Risks/unknowns:**
- NEAR Intents solver API stability — pin a specific version, record fixtures, live test catches drift.
- Multi-leg timing: expiry across a bundle needs a safety margin.

### M5 — Widget + custom scripts (Monty)

**Goal**: rich UI state + user-authored extensibility.

**Deliverables:**
- `.system/gateway/widgets/portfolio/manifest.json` and render layout.
- `widgets/state.json` writer in the mission playbook.
- Widget polls (or SSE-subscribes if available) and re-renders.
- Monty script discovery in the skill playbook — scripts listed in LLM context.
- Four starter scripts documented in the skill's playbook:
  - `alert_if_health_below.py`
  - `weekly_report.py`
  - `backtest_strategy.py`
  - `concentration_warning.py`
- Skill prompt teaches the LLM to author new scripts on user request and optionally wire them into sub-missions.
- Replay scenarios: `widget-shape`, `custom-script-alert`, `backtest-strategy`.

**Exit criteria:**
- Widget renders live state in the web channel.
- User can ask "alert me when..." and get a working script committed to `projects/<id>/scripts/`.
- Backtest script replays a strategy against `state/history/*.json` and reports realized vs projected.
- Script execution runs inside Monty sandbox with tool ceiling inherited from the skill.

**Risks/unknowns:**
- Widget framework capabilities — confirmed declarative layout exists; advanced interactions (sign button, poll cadence) may need minor widget framework additions.
- SSE subscription for workspace writes may not exist yet; poll at 30s until it does.

### M6 — Money Center (wallet UI + signing)

**Goal**: signing flow completes the loop. Generalized wallet/money center lays groundwork for future banking/card integrations.

**Deliverables:**
- `src/channels/web/money_center/` module: single "Money" tab aggregating all per-user money projects.
- **NEAR wallet auto-login**: if the user is authenticated to IronClaw via a NEAR wallet, the Money Center picks up that session automatically — no second login, no second modal. This is the default path for NEAR-side signing.
- WalletConnect v2 integration for EVM chains (separate modal, only triggered when an EVM leg needs signing).
- NEAR wallet selector integration as fallback for users whose IronClaw session isn't already NEAR-authenticated.
- `portfolio_intent_submit` built-in tool (see §2.11).
- Widget "Sign" action wired to the tool.
- SSE events: `portfolio.intent.awaiting_signature`, `portfolio.intent.signed`, `portfolio.intent.submitted`, `portfolio.intent.failed`.
- `BalanceSource` abstraction with three conceptual implementations: `CryptoWalletBalanceSource` (used by portfolio), `BankAccountBalanceSource` (stub), `CardBalanceSource` (stub).
- Money Center landing page lists all money projects with their widgets.
- Replay scenario: `sign-flow-stub` with mocked wallet.
- Live test (`#[ignore]`): end-to-end sign + submit on testnet.

**Exit criteria:**
- User can sign and submit an intent entirely in the web UI.
- Signed payloads never touch server logs or SSE broadcasts in plaintext.
- Audit trail: intent built → awaiting signature → signed (hash only) → submitted (tx hash) → confirmed.
- Money Center tab shows portfolio project alongside placeholder bank/card cards.

**Risks/unknowns:**
- WalletConnect v2 integration depth — scope to read-only wallet info + sign a single intent payload; no full wallet management.
- NEAR wallet selector UX quality.
- Regulatory considerations for bank/card integrations — stubs only until legal/product sign-off.

## 8. Open questions

Decided (kept here so the rationale is visible):

- **Auth model**: logged-in users only. No anonymous/ephemeral project flow in v1.
- **Multi-wallet**: multiple addresses live in one default `portfolio` project. A separate project is created only when the user explicitly asks.
- **Widget aggregation**: portfolio totals aggregate across all addresses in the project; widget has a per-address drill-down section.
- **Money Center default sign-in**: if the user is NEAR-authenticated to IronClaw, the Money Center reuses that session automatically.
- **Learner mission**: folded into the keeper mission's goal for v1. Revisit as a standalone after-thread mission post-M5 once there's suggestion/outcome history.
- **Bank/card integrations**: stubs only in M6.

Still open:

1. **Strategy doc trust for community contributions.** In-repo and user-workspace strategies are trusted. Community-contributed strategies would need a signed registry — out of scope for M1–M6 but the schema is version-tagged from day one so it can evolve without a break.
2. **Mission cadence defaults.** 6h is arbitrary. Consider risk-driven cadence: faster for portfolios with near-liquidation positions, slower for pure idle stablecoin portfolios. Easy to adjust; punted until we see real mission runs.
3. **Script execution on cadence.** Monty scripts can be one-shot (called from mission) or their own sub-missions with independent cadence. Pick a default pattern in the skill playbook so users don't have to decide. Leaning: default to "one-shot called from keeper", allow "promote to sub-mission" when the user explicitly wants a different cadence.
4. **Re-introducing pluggable sources.** If/when a second indexer becomes desirable (self-hosted RPC, alternative commercial indexer), `indexer/` grows a thin dispatcher. Defer until the second source exists.

## 9. Non-goals (for the record)

- Custodial wallets.
- Automatic execution without user confirmation.
- Raw EVM transaction building (intents only).
- Off-chain CEX integrations (might slot in as a `BalanceSource` in the Money Center much later).
- Tax reporting / accounting features.
- Social / copy-trading features.
- Any LLM fine-tuning on user positions.
- Source pluggability in v1 (see §8 Open Questions).
- Separate learner mission in v1 (folded into keeper goal).
- Anonymous/ephemeral portfolio sessions in v1 (logged-in users only).
- Event-driven or news-driven supporting missions in v1 (see §12).

## 10. File and code map (reference)

New or modified locations:

```
skills/portfolio/
  SKILL.md

tools-src/portfolio/
  Cargo.toml
  capabilities.json
  src/
    lib.rs
    indexer/{mod.rs, dune.rs, fixture.rs}
    analyzer/{mod.rs, registry.rs}
    strategy/{mod.rs, parser.rs, filter.rs}
    intents/{mod.rs, solver.rs, bundling.rs, bounded.rs}
    types/{position.rs, proposal.rs, intent.rs}
  protocols/
    aave-v3.json
    compound-v3.json
    uniswap-v3.json
    lido.json
    morpho-blue.json
  strategies/
    stablecoin-yield-floor.md
    lending-health-guard.md
    lp-impermanent-loss-watch.md

src/tools/builtin/portfolio.rs          # M6 only: portfolio_intent_submit

src/channels/web/money_center/          # M6 only
  mod.rs
  routes.rs
  wallet_connect.rs
  near_wallet.rs
  balance_source.rs

.system/gateway/widgets/portfolio/
  manifest.json
  layout.json

tests/portfolio_replay.rs
tests/portfolio_scenarios/
  *.yaml
tests/fixtures/
  portfolio_indexer/*.json
  solver/*.json
  llm/*.jsonl

docs/plans/2026-04-11-defi-portfolio-keeper.md   # this file
```

No changes expected to:
- `src/agent/` (uses existing mission/thread infrastructure)
- `src/db/` (uses existing project/memory schema)
- `src/tools/dispatch.rs` (new tool registers through existing path)
- `src/tools/mcp/` (no MCP server used)
- `crates/ironclaw_safety/` (new tool inherits the pipeline)

## 11. Follow-on: event-driven supporting missions (post-M6)

M1–M6 give us one mission on a 6h cron. That's enough to ship, but the right shape for a real portfolio keeper is a **mesh of event-driven missions** that supply signal to the keeper and can trigger unscheduled rebalances between cron runs. This section is the target architecture; none of it is required for v1.

### 12.1 Shape

```
              ┌──────────── price-watcher ─────────┐
              │  cadence: OnEvent(price_move)      │
              │  watches: per-position token prices│
              │  action:  emit portfolio.signal    │
              └───────────────┬─────────────────────┘
                              │
                              ▼
    ┌─────────────────────── event bus ────────────────────────┐
    │     portfolio.signal.*  (price, news, onchain, risk)     │
    └───────────────┬──────────────────────────┬───────────────┘
                    │                          │
                    ▼                          ▼
       ┌─────── news-watcher ──────┐   ┌──── onchain-watcher ────┐
       │ cadence: OnCron + trigger │   │ cadence: OnEvent(tx)    │
       │ sources: RSS, API, LLM    │   │ watches: protocol events│
       │ action: emit signal       │   │ action: emit signal     │
       └──────────────┬────────────┘   └────────────┬────────────┘
                      │                             │
                      ▼                             ▼
                  ┌────────── portfolio-keeper ─────────┐
                  │ cadence: Cron + OnEvent(signal)     │
                  │ on signal: run a bounded scan,      │
                  │            reuse strategies, build  │
                  │            intents if criteria met  │
                  └─────────────────────────────────────┘

       ┌──────────── research-missions ─────────────┐
       │ cadence: OnEvent(signal) | OnDemand        │
       │ produce: memory/research/*.md, annotations │
       │          to strategy docs, no intents      │
       └────────────────────────────────────────────┘
```

### 12.2 Missions

**`price-watcher`** — event-driven, per-project.
- **Cadence**: `OnEvent(price_move)`. Backed by a lightweight polling loop (or webhook if a source supports it) that fires only when a watched token moves more than `config.price_trigger_bps` within a rolling window.
- **Watches**: every token appearing in the project's latest state.
- **Action**: emits `portfolio.signal.price` with `{token, chain, old_price, new_price, delta_bps, affected_positions}`.
- **No intents, no strategies.** Pure signal generation.

**`news-watcher`** — hybrid cron + event.
- **Cadence**: `Cron(1h)` for polling + `OnEvent(high_severity_news)` for webhook-capable sources.
- **Sources**: RSS (protocol blogs, audit firms), public news APIs, and LLM-summarized aggregators. Each source is a JSON entry in the mission's config, parallel to how `protocols/` is set up — pluggable, not hardcoded.
- **Action**: emits `portfolio.signal.news` with `{protocol_id, severity, headline, url, summary}`. Severity is LLM-assigned against a small rubric (exploit, pause, governance, minor).
- **Only fires signals for protocols present in the project's state** — global DeFi news doesn't generate noise for a user who only holds Aave.

**`onchain-watcher`** — event-driven, direct chain.
- **Cadence**: `OnEvent(chain_event)`. Subscribes to protocol-specific events the user has positions in — e.g. Aave `ReserveDataUpdated`, Lido `Slashed`, Morpho `MarketPaused`, large oracle deviations.
- **Action**: emits `portfolio.signal.onchain`.
- **Implementation**: a WASM tool `portfolio_onchain_watch` registers event filters with an RPC or subgraph; host loop forwards matching events to the mission.

**`portfolio-keeper`** (upgraded).
- **Cadence**: `Cron(0 */6 * * *) + OnEvent(portfolio.signal.*)`.
- **On signal**: runs a *bounded* scan — only re-scans positions affected by the signal, not the whole portfolio — and feeds strategies with an extra `trigger_signal` context field so they can branch on severity.
- **Idempotency key** now includes the signal ID so multiple signals for the same underlying cause collapse.

**`research-mission`** — on-demand or signal-triggered.
- **Cadence**: `OnEvent(portfolio.signal.*) | OnDemand`.
- **Goal**: "When a signal crosses a research threshold (e.g. a governance proposal affecting a held protocol), do deep-dive research: read the proposal, check historical votes, check social sentiment, write a `memory/research/<topic>.md` doc and optionally annotate affected strategy docs with a time-boxed caveat."
- **Never builds intents.** Its output is knowledge, consumed by the keeper on its next run.

### 12.3 Event bus

Uses the existing v2 engine event surface (`OnSystemEvent` mission cadence + `event_emit` tool). No new infrastructure. Signals are typed:

```
portfolio.signal.price      { token, chain, delta_bps, affected_positions }
portfolio.signal.news       { protocol_id, severity, url, summary }
portfolio.signal.onchain    { protocol_id, chain, event_name, payload }
portfolio.signal.risk       { kind, severity, rationale }   # synthesized
```

Anyone — a user script, another mission, even a human via tool call — can emit a signal. The keeper and research missions subscribe. This is the main extensibility seam for event-driven behavior.

### 12.4 Rate limits and noise control

- Each watcher has a per-signal debounce (`min_interval_per_token = 5min` default).
- Keeper's event-triggered runs cost budget: max N event-triggered runs per hour before it falls back to cron-only until the window clears.
- Signal severity gates: low-severity signals accumulate into a summary digest rather than firing the keeper immediately.
- All of this is in the project's `config.json` under an `event_triggers` block, so users can tune without code changes.

### 12.5 Testing

Replay harness extends cleanly:
- Scenarios can include a `signals:` array that the harness injects at specified offsets.
- Watcher missions have their own unit tests (deterministic signal generation from synthetic inputs).
- Integration test: run keeper with a pre-queued signal, assert it produced a bounded scan + a suggestion referencing the signal in its rationale.

### 12.6 Milestone placement

This is a **post-M6 phase** — call it **M7: Event-driven mesh**. It depends on:
- M1–M5 for the core keeper + strategies + tools.
- M6 only for the Money Center signing flow (so event-triggered rebalances can actually execute).

Splitting into sub-milestones:
- **M7a**: event bus wiring + `price-watcher` (simplest signal source).
- **M7b**: `news-watcher` with pluggable sources + LLM severity classifier.
- **M7c**: `onchain-watcher` (most complex — needs RPC/subgraph subscription).
- **M7d**: `research-mission` + memory/research integration.

Each substage ends with new replay scenarios exercising the new signal type.

## 12. Success criteria (end-to-end)

1. An anonymous user pastes an EVM address into the web channel, waits ~10s, and sees an accurate positions table + at least one actionable suggestion with projected Δ earnings and gas payback days.
2. A logged-in user can convert that one-shot into a persistent project + keeper mission in one click. The mission runs on schedule and produces fresh suggestion files + widget state.
3. A user can say "alert me when X" and get a working script committed to their project.
4. The replay test suite has ≥10 scenarios covering smoke, real-data, strategy triggering, adversarial inputs, idempotency, widget shape, and custom scripts. All pass in CI against mock LLM + fixture backends.
5. A developer can add a new protocol by writing a single JSON file and recompiling the `portfolio` WASM tool. No core Rust edits.
6. A developer can add a new strategy by writing a single Markdown file with YAML frontmatter. No code edits.
7. At M6, a user can sign and submit an intent entirely within the Money Center UI, and the audit trail is complete from suggestion to confirmed tx hash.
8. At M6, a stubbed bank/card `BalanceSource` compiles and is listed in Money Center, proving the abstraction is honest.
