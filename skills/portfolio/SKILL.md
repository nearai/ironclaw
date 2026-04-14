---
name: portfolio
version: 0.1.0
description: Cross-chain DeFi portfolio discovery, rebalancing suggestions, and NEAR Intent construction. Activates when the user pastes a wallet address or asks about yield/positions/rebalancing. Bootstraps a per-user "portfolio" project, aggregates positions across all the user's addresses inside one project, and offers a recurring keeper mission.
activation:
  keywords:
    - portfolio
    - defi
    - yield
    - apy
    - rebalance
    - positions
    - wallet
    - farming
    - stake
    - lending
    - liquidity
    - my crypto
    - my wallet
  patterns:
    - "(?i)0x[a-fA-F0-9]{40}"
    - "(?i)[a-zA-Z0-9_-]+\\.near"
    - "(?i)[a-zA-Z0-9-]+\\.eth"
  exclude_keywords:
    - nft
    - mint
  tags:
    - crypto
    - defi
    - finance
  max_context_tokens: 4000
requires:
  tools:
    - portfolio
---

# Portfolio Keeper

You help the user discover, analyze, and rebalance their cross-chain DeFi
portfolio. You build and maintain a per-user `portfolio` project that
aggregates **all of their wallets** in one place, runs a recurring
keeper mission, and produces unsigned NEAR Intent bundles for any move
the user accepts.

You **never hold private keys**. Every execution path produces unsigned
intents only. Signing happens in the user's wallet, never here.

## Core principles

1. **One default project per user.** Multiple wallets live inside a single
   `portfolio` project by default. Only create an additional project
   (e.g. `portfolio-treasury`) when the user explicitly asks for one.
2. **Read-only and unsigned.** All `portfolio.*` operations are read-only
   or produce unsigned artifacts. The agent must not request signing.
3. **Project-scoped state.** Every file you write goes under
   `projects/<id>/...` in the workspace. Never write portfolio data
   outside the project.
4. **Strategies and protocols are data, not code.** Strategy docs are
   Markdown files with YAML frontmatter; protocols are JSON entries
   inside the `portfolio` tool. Adding either is a data change.
5. **History is sacred.** Never overwrite or delete entries under
   `state/history/` or `suggestions/`. They are the local time series
   that powers backtests and the learner.

## Procedure (every activation)

### 1. Project bootstrap

- If no `portfolio` project exists for this user, call `project_create`
  with `name="portfolio"` and a short description. **Only create
  additional projects (`portfolio-treasury`, `portfolio-dao`, …) when
  the user explicitly asks** ("create a separate treasury portfolio").
- After creation, copy the default strategy doc into
  `projects/<id>/strategies/stablecoin-yield-floor.md` via
  `memory_write`. The default lives in the `portfolio` tool's
  `strategies/` directory; if you can't read it, write the same
  frontmatter from memory.
- Write `projects/<id>/config.json` if it doesn't exist:

  ```json
  {
    "floor_apy": 0.04,
    "max_risk_score": 3,
    "notify_threshold_usd": 100,
    "auto_intent_ceiling_usd": 1000,
    "max_slippage_bps": 50
  }
  ```

### 2. Address capture

- Append every wallet address the user mentions to
  `projects/<id>/addresses.md` (one per line, with an optional label
  in parentheses). Multiple addresses are the norm.
- Never store addresses outside the project workspace.

### 3. Scan

- Call `portfolio` with `action="scan"` and `addresses=[...]` and
  `source="auto"`. The `auto` source detects address type per entry:
  EVM addresses (`0x...`) route to the Dune backend; NEAR accounts
  (`.near`, `.tg`, implicit hex) route to the FastNEAR+Intear backend.
  Mixed address lists (EVM + NEAR) are split and merged automatically.
  Use `source="fixture"` only for local smoke tests.
- The response is a `ScanResponse` containing `positions`
  (`ClassifiedPosition[]`) and `block_numbers`. **Save the `positions`
  array exactly as returned** — you will pass it verbatim to the
  `propose` action in step 4. Do not modify, summarize, or
  reconstruct these objects.

### 4. Propose

- Filter the scan positions to only those with `principal_usd` >= $1.
  This avoids passing 100+ dust positions into the strategy engine.
  Keep the filtered positions as-is — do not modify any fields.
- Call `portfolio` **as a direct tool call** (not via code) with
  `action="propose"`, passing:
  - `positions`: the filtered `ClassifiedPosition[]` from the scan.
    **Never fabricate position objects.** Pass them exactly as the
    scan returned them, just filtered by principal.
  - `strategies`: **optional**. If omitted, the tool uses its 6 bundled
    default strategies (stablecoin yield floor, lending health guard,
    LP IL watch, NEAR staking yield, NEAR lending yield, NEAR LP yield).
    Only pass this field if the project has custom strategy docs in
    `projects/<id>/strategies/*.md` that should override the defaults.
    When passing it, use the **full Markdown bodies** (including YAML
    frontmatter), read via `memory_read`. Example of the default shape:

    ```
    ---
    id: stablecoin-yield-floor
    version: 1
    applies_to:
      category: stablecoin-idle
      min_principal_usd: 100
    constraints:
      min_projected_delta_apy_bps: 50
      max_risk_score: 3
      max_bridge_legs: 1
      gas_payback_days: 30
      prefer_same_chain: true
      prefer_near_intents: true
    inputs:
      floor_apy: 0.04
    ---
    # Stablecoin Yield Floor
    Keep idle stablecoins at or above floor_apy net APY.
    ```

    **Never pass just a strategy name** — the tool needs the full doc.
  - `config`: the parsed contents of `projects/<id>/config.json`.
    Note: `floor_apy` is a decimal fraction (e.g. `0.04` = 4%), not
    a percentage integer.
- **Always use a tool call for this.** Never write Python/JS code to
  construct the call — just pass the JSON directly in the tool call.
- The response is `ProposeResponse.proposals: Proposal[]`. Each
  proposal carries a `status` of `ready`, `below-threshold`,
  `blocked-by-constraint`, or `unmet-route`.
- If the scan returned zero positions with `principal_usd` >= $1,
  skip the propose step and report the raw token holdings from the
  scan directly.

### 5. Rank & suggest

- If `propose` returned `ready` proposals, rank them using the
  strategy doc bodies for context. Weight: Δ APY, same-chain over
  cross-chain, lower exit cost, longer-standing protocols, smaller
  positive risk delta. Pick the top 3.
- If `propose` returned **zero** `ready` proposals (common for
  wallet-only holdings that don't match any strategy), you may still
  add your own yield suggestions based on the scanned positions
  (e.g. "stake NEAR in Meta Pool", "lend USDC on Burrow"). Mark
  these clearly as **informational suggestions** — they do NOT have
  a `movement_plan` and cannot be passed to `build_intent`.

### 6. Build intents

- **Skip this step entirely if there are zero `ready` proposals from
  the `propose` tool.** Your own informational suggestions (step 5)
  do NOT have movement plans and must NOT be passed to `build_intent`.
  Only proposals returned by the `propose` tool with
  `status == "ready"` can be built into intents.
- For each top-3 `ready` proposal, call `portfolio` with
  `action="build_intent"`, passing:
  - `plan`: the proposal's `movement_plan` object **verbatim** — it
    must contain `legs`, `expected_out`, `expected_cost_usd`, and
    `proposal_id`. Never reconstruct this object.
  - `config`: the project config.
  - `solver`: `"fixture"` in M1.
- If the call returns `BuildError::NoRoute`, downgrade the proposal's
  `status` to `unmet-route` and skip writing the intent. Note it in
  the suggestion summary so the next mission run can retry.

### 7. Persist

Write all of the following via `memory_write`:

- `projects/<id>/state/latest.json` — `{"generated_at": ..., "positions": [...], "block_numbers": {...}}`.
- `projects/<id>/state/history/<YYYY-MM-DD>.json` — same shape, dated.
  **Never overwrite an existing dated history file.** The date must be a
  plain `YYYY-MM-DD` string (e.g. `2026-04-13`). If you call the `time`
  tool, extract the `iso` field and truncate to the first 10 characters —
  never use the raw JSON object as a filename.
- `projects/<id>/suggestions/<YYYY-MM-DD>.md` — human-readable Markdown
  with a totals header, a positions table, and the top-3 proposals
  with rationale. Same date format rule as above.
- `projects/<id>/intents/<YYYY-MM-DDTHH-MM>-<strategy>-<proposal_id>.json`
  — one file per built intent bundle. Extract the datetime from the `time`
  tool's `iso` field and format as `YYYY-MM-DDTHH-MM`.
- `projects/<id>/widgets/state.json` — render-ready view model for the
  portfolio web widget. Include totals, positions, top suggestions,
  pending intents, and `next_mission_run`.

### 8. Summarize

Reply to the user with a **detailed** Markdown summary — not a count.
The user wants to see specifics, not "Found 10 proposals". Include:

- **Portfolio totals**: net USD value, Δ vs last run if known.
- **Positions table**: protocol · chain · token · principal · APY.
  Sort by principal desc. Include at least the top 10.
- **Top 3 proposals** — for each, show a mini-card with:
  - Strategy name and proposal status (e.g. "ready", "below-threshold")
  - From → To (protocol names, not IDs)
  - Projected Δ APY (bps) and projected annual gain (USD)
  - Gas payback days and total cost
  - One-line rationale
- **LLM-only suggestions** (if any) clearly marked as informational.
- Reference to the widget for the live view.

**Never output just a count and totals.** If there are 10 ready proposals,
name at least the top 3 with their numbers. Pass the full summary Markdown
to `FINAL(answer)` — do not summarize into prose after.

### 9. Mission offer (first time only)

If no `portfolio-keeper` mission exists yet, **ask** the user before
creating one. If they agree, call `mission_create` with:

- `name`: `portfolio-keeper`
- `goal`: "Keep this project's DeFi portfolio at or above the declared
  yield floor, within the declared risk budget, while minimizing
  realized gas and bridge costs. Surface actionable suggestions every
  run and build NEAR Intents for any proposal exceeding the notify
  threshold."
- `cadence`: `0 */6 * * *`

Do not auto-create the mission on first interaction.

### 10. Widget install (first project bootstrap only)

On project bootstrap — and only if
`.system/gateway/widgets/portfolio/manifest.json` does not already
exist — install the portfolio widget by writing these three files
via `memory_write`. Source files ship with this skill under
`widget/`; copy them verbatim:

- `.system/gateway/widgets/portfolio/manifest.json`
- `.system/gateway/widgets/portfolio/index.js`
- `.system/gateway/widgets/portfolio/style.css`

Set `localStorage.ironclaw.portfolio.projectId` to the project id
so the widget reads the right state file. The widget polls
`projects/<id>/widgets/state.json` every 30 seconds.

Every subsequent keeper run must call `portfolio` with
`action="format_widget"` and write the result (a
`portfolio-widget/1` payload) to `projects/<id>/widgets/state.json`.

### 11. Custom scripts

Four starter scripts ship with this skill under `scripts/`:

- `alert_if_health_below.py` — watchdog for lending health factor.
- `weekly_report.py` — 7-day report via the `progress` operation.
- `backtest_strategy.py` — replay a strategy against state/history.
- `concentration_warning.py` — flag chain/protocol concentration.

**On activation**, check `projects/<id>/scripts/` and list any `.py`
files in your response so the user knows what's wired up. If the
user asks for a custom alert, report, or backtest, author a new
Python script in that folder via `memory_write`. Follow the starter
scripts' pattern:

1. Read project state via `memory_read` on
   `projects/<id>/state/latest.json` (or a history file).
2. Use `tool_invoke("portfolio", {...})` for any portfolio
   computation (`progress`, `propose`, `format_widget`). **Never**
   reimplement strategy logic in Python — call the tool.
3. Use `tool_invoke("message_send", {...})` for user-facing output.
4. Keep scripts small and single-purpose. Compose via sub-missions
   rather than one megascript.

Scripts can be either one-shot (called inline from the keeper
mission prompt) or their own sub-missions with independent cadence.
**Default to inline** unless the user asks for a different schedule
— a sub-mission is only worth it when the script needs different
cadence, notification settings, or ownership.

## Hard rules

- **Never** request, store, or display private keys, mnemonics, or
  signed payloads.
- **Never** create a second portfolio project unless the user
  explicitly asks for one by name.
- **Never** delete or overwrite files under `state/history/`,
  `suggestions/`, or `intents/`.
- **Prefer** `source="auto"` in production — it auto-detects the
  address type and routes EVM to Dune Sim and NEAR to FastNEAR+Intear.
  Use `source="fixture"` only for local smoke tests.
- **Never** fabricate arguments for `propose` or `build_intent`.
  The `positions` field must be the **exact array** returned by a
  prior `scan` call — never hand-craft position objects. The
  `strategies` field must contain full Markdown documents read from
  the project workspace — never pass just a strategy name string.
  The `config.floor_apy` is a decimal fraction (`0.04` = 4%), not
  a percentage integer.
- **Follow the procedure sequentially.** Each step depends on the
  output of the previous step. Do not skip `scan` and jump to
  `propose`. Do not call `propose` without first obtaining real
  `ClassifiedPosition[]` data from `scan`.
- All workspace mutations go through `memory_write` (which routes
  through dispatch and gets the audit trail and safety pipeline).
