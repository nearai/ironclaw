// M1 scaffold: several types/methods are reserved for M2+ stages and
// are intentionally unused right now. Tightening to per-item allows
// once the surface stops moving.
#![allow(dead_code)]

//! Portfolio WASM tool for IronClaw.
//!
//! Single tool with multiple operations:
//!
//! - `scan` — discover positions across chains for one or more addresses
//!   and classify them against the embedded protocol registry.
//! - `propose` — given classified positions and a set of strategy docs,
//!   produce ranked rebalancing proposals (deterministic constraint
//!   filter; LLM does the final ranking via the skill playbook).
//! - `build_intent` — translate a movement plan into an unsigned NEAR
//!   Intent bundle, applying bounded checks before returning.
//! - `backtest` — run deterministic long-only spot strategy tests over
//!   caller-provided OHLCV candles before any intent is built.
//! - `backtest_suite` — rank several strategy candidates over the same
//!   candle episode before any strategy is selected for paper intent work.
//! - `backtest_walkforward` — fixed-strategy walk-forward across N
//!   contiguous, non-overlapping fold windows; reports per-fold
//!   IS/OOS gap and an aggregate robustness verdict.
//! - `backtest_montecarlo` — seeded resample/permute over a per-trade
//!   return series; reports return / drawdown / terminal-equity
//!   distributions and loss probabilities.
//! - `backtest_grid` — cartesian sweep over strategy parameter axes
//!   plus optional sizing/stop/take-profit options; ranked through
//!   `backtest_suite`.
//! - `validate_episode` — structural check for the shared episode
//!   format (candles + optional solver fixture + optional news
//!   context).
//! - `replay_episode` — validate then replay an episode through
//!   `backtest_suite`, surfacing the embedded solver fixture and
//!   news context.
//! - `backtest_dca` — replay a fixed-cadence dollar-cost-averaging
//!   buy schedule with optional symmetric price-band variants.
//! - `plan_dca_schedule` — emit a recurring DCA schedule (cron + per
//!   period plan + `build_intent` template + risk gates) for a
//!   NEAR-Intents-supported destination asset; unsigned only.
//! - `compile_intent_prompt` — turn a natural-language trade prompt
//!   ("DCA $100 weekly into NEAR for 6 months") into a structured
//!   recommended action plus extracted fields, assumptions,
//!   clarifications, and gates. Deterministic pattern matching, no
//!   LLM call.
//! - `validate_strategy` — composed base backtest + walk-forward +
//!   Monte Carlo against caller-tunable thresholds; returns an
//!   `approved` / `watch` / `rejected` verdict with a per-gate
//!   pass/fail table.
//! - `format_strategy_doc` — Markdown formatter for any of the
//!   backtest-family schemas; produces a journal-ready document the
//!   agent persists after a run.
//! - `plan_paid_research` — rank paid/free research sources, budget a
//!   premium-source query, and prepare attribution/payment gates.
//! - `plan_dripstack_browse` — model DripStack's guided topic →
//!   publication → article purchase flow without fetching paid content.
//! - `fetch_dripstack_catalog` — fetch DripStack's free publication or
//!   publication-post metadata routes for guided browse.
//! - `prepare_dripstack_paid_fetch` — prepare the explicit
//!   confirmation/receipt boundary for a single paid DripStack article.
//! - `plan_near_intents_trial` — prepare a nominal-NEAR paper/quote
//!   rehearsal with strategy gates and unsigned intent build requests.
//! - `format_intents_widget` — build the NEAR Intents trading console
//!   view model consumed by the project widget.
//!
//! The agent never holds private keys. All output is read-only or
//! unsigned.
//!
//! Internal layout (one module per stage):
//!
//! ```text
//! src/
//! ├── lib.rs              // WIT bindings, action dispatch, schema
//! ├── types/              // shared types between stages
//! ├── indexer/            // scan: fetch + normalize raw positions
//! ├── analyzer/           // classify raw positions via protocols/*.json
//! ├── strategy/           // propose: parse strategy docs + filter
//! ├── intents/            // build_intent: solver call + bounded checks
//! ├── backtest.rs         // paper strategy replay/ranking over OHLCV candles
//! ├── research.rs         // paid research source budgeting/attribution
//! ├── trial.rs            // nominal NEAR rehearsal planning
//! └── widget.rs           // web widget state formatters
//! ```

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

mod analyzer;
mod backtest;
mod dca;
mod episode;
mod format;
mod indexer;
mod intents;
mod lab;
mod nl;
mod research;
mod strategy;
mod trial;
mod types;
mod validate;
mod widget;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod replay_tests;

use types::{ChainSelector, IntentBundle, MovementPlan, ProjectConfig, Proposal, ScanAt};

struct PortfolioTool;

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum PortfolioAction {
    /// Discover and classify positions for one or more addresses.
    #[serde(rename = "scan")]
    Scan {
        addresses: Vec<String>,
        #[serde(default = "default_chains")]
        chains: ChainSelector,
        #[serde(default)]
        at: Option<ScanAt>,
        /// Data source. M1: only "fixture". M2: "dune".
        #[serde(default = "default_source")]
        source: String,
    },

    /// Generate ranked rebalancing proposals from classified positions.
    #[serde(rename = "propose")]
    Propose {
        positions: serde_json::Value,
        /// Raw markdown strategy docs (with YAML frontmatter). If omitted
        /// or empty, falls back to the bundled default strategies
        /// (stablecoin-yield-floor, lending-health-guard,
        /// lp-impermanent-loss-watch, near-staking-yield,
        /// near-lending-yield, near-lp-yield).
        #[serde(default)]
        strategies: Vec<String>,
        /// Project configuration (floor_apy, risk caps, slippage).
        /// Optional — defaults to the standard config if omitted.
        #[serde(default)]
        config: ProjectConfig,
    },

    /// Translate a movement plan into an unsigned NEAR Intent bundle.
    #[serde(rename = "build_intent")]
    BuildIntent {
        plan: MovementPlan,
        /// Project configuration. Optional — defaults to standard config.
        #[serde(default)]
        config: ProjectConfig,
        /// Solver source. M1: only "fixture". M4: "near-intents".
        #[serde(default = "default_solver")]
        solver: String,
    },

    /// Format classified positions + proposals as a Markdown
    /// suggestion doc. Deterministic and snapshot-testable.
    #[serde(rename = "format_suggestion")]
    FormatSuggestion(format::FormatSuggestionInput),

    /// Compute the mission progress metric (realized net APY over
    /// the last 7 history snapshots vs the config's floor_apy).
    #[serde(rename = "progress")]
    Progress(format::ProgressInput),

    /// Run deterministic long-only spot backtests over caller-provided
    /// OHLCV candles. Used by the Intents Trading Agent before a
    /// candidate trade is eligible for intent construction.
    #[serde(rename = "backtest")]
    Backtest(backtest::BacktestInput),

    /// Run and rank several deterministic long-only spot strategy
    /// candidates over the same OHLCV episode. Used to build a
    /// menu of choices instead of blindly evaluating one strategy.
    #[serde(rename = "backtest_suite")]
    BacktestSuite(backtest::BacktestSuiteInput),

    /// Run a fixed-strategy walk-forward across N contiguous,
    /// non-overlapping fold windows, splitting each fold into
    /// in-sample (train) and out-of-sample (test) ranges with an
    /// optional embargo. Reports per-fold IS/OOS gap and an
    /// aggregate robustness verdict. Does not refit parameters.
    #[serde(rename = "backtest_walkforward")]
    BacktestWalkForward(lab::WalkForwardInput),

    /// Run a deterministic Monte Carlo on a per-trade return series
    /// (typically `trades[].return_pct` from a prior backtest). Uses
    /// a seeded xorshift64* PRNG to resample or permute the trade
    /// order and reports return / drawdown / terminal-equity
    /// distributions plus loss probabilities.
    #[serde(rename = "backtest_montecarlo")]
    BacktestMonteCarlo(lab::MonteCarloInput),

    /// Cartesian sweep over strategy parameter axes plus optional
    /// sizing/stop/take-profit options. Materializes candidates,
    /// runs `backtest_suite`, and returns the ranked grid plus a
    /// flat cell table. Enforces a `max_cells` cap so the WASM
    /// sandbox can't be DOSed by an oversized request.
    #[serde(rename = "backtest_grid")]
    BacktestGrid(lab::GridInput),

    /// Validate a multi-surface episode (candles + optional solver
    /// fixture + optional news context) and return a deterministic
    /// summary. No backtest is run.
    #[serde(rename = "validate_episode")]
    ValidateEpisode(episode::ValidateEpisodeInput),

    /// Validate an episode and replay it through `backtest_suite`,
    /// surfacing the embedded solver fixture and news context so the
    /// caller can route them into `build_intent` and the analyst
    /// memos.
    #[serde(rename = "replay_episode")]
    ReplayEpisode(episode::ReplayEpisodeInput),

    /// Replay a fixed-cadence dollar-cost-averaging schedule over a
    /// candle series. Reports lots, breakeven, mark-to-market, and
    /// alpha vs lump-sum buy-and-hold. Optional symmetric price band
    /// turns the schedule into "skip when stretched, double-up when
    /// discounted" without breaking determinism.
    #[serde(rename = "backtest_dca")]
    BacktestDca(dca::DcaBacktestInput),

    /// Plan a recurring DCA schedule into a NEAR-Intents-supported
    /// destination asset. Emits a cron expression, per-period plans,
    /// risk gates, and a `build_intent` template the caller uses per
    /// period. All output is unsigned; signing remains user-only.
    #[serde(rename = "plan_dca_schedule")]
    PlanDcaSchedule(dca::DcaScheduleInput),

    /// Compile a natural-language trade prompt into a structured
    /// recommended action (one of `plan_dca_schedule`, `build_intent`,
    /// `backtest_suite`, `plan_paid_research`, `format_intents_widget`,
    /// or `noop`). Deterministic pattern matching, no LLM call. The
    /// caller invokes the recommended action separately after risk
    /// gating; this compiler never executes anything.
    #[serde(rename = "compile_intent_prompt")]
    CompileIntentPrompt(nl::CompileInput),

    /// Composed strategy validation: runs the base backtest plus
    /// optional walk-forward and Monte Carlo passes against a single
    /// candidate, applies caller-tunable eligibility thresholds, and
    /// returns an `approved` / `watch` / `rejected` verdict with a
    /// per-gate pass/fail table.
    #[serde(rename = "validate_strategy")]
    ValidateStrategy(validate::ValidateStrategyInput),

    /// Format any of the new backtest-family responses (base
    /// backtest, walk-forward, Monte Carlo, grid, DCA backtest, DCA
    /// schedule, validation) as a journal-ready Markdown document.
    /// Deterministic; the caller persists the output.
    #[serde(rename = "format_strategy_doc")]
    FormatStrategyDoc(validate::FormatStrategyDocInput),

    /// Plan a premium-source research query before fetching paywalled
    /// content. This ranks candidate sources, enforces a budget, and
    /// returns attribution/payment gates for MPP, x402, and NEAR
    /// Intents-style rails. It never fetches paid content or signs.
    #[serde(rename = "plan_paid_research")]
    PlanPaidResearch(research::PaidResearchPlanInput),

    /// Plan the DripStack guided-browse flow. Free catalog/post-title
    /// metadata goes in; a selected article becomes a paid-source
    /// candidate for `plan_paid_research`. It never buys or fetches
    /// article bodies.
    #[serde(rename = "plan_dripstack_browse")]
    PlanDripstackBrowse(research::DripstackBrowseInput),

    /// Fetch DripStack's free catalog routes. This may return
    /// publication metadata or post-title metadata; it does not fetch
    /// paid article bodies.
    #[serde(rename = "fetch_dripstack_catalog")]
    FetchDripstackCatalog(research::DripstackCatalogInput),

    /// Prepare the paid article fetch boundary for one DripStack post:
    /// confirmation, 402 challenge probe, and receipt-backed retry
    /// headers. It never creates a payment receipt or reads article
    /// content by itself.
    #[serde(rename = "prepare_dripstack_paid_fetch")]
    PrepareDripstackPaidFetch(research::DripstackPaidFetchInput),

    /// Plan a nominal-NEAR trial run. This returns setup guardrails,
    /// strategy menu defaults, and a paper/live-quote `build_intent`
    /// request without signing or moving funds.
    #[serde(rename = "plan_near_intents_trial")]
    PlanNearIntentsTrial(trial::NearTrialPlanInput),

    /// Build the render-ready view model the web widget consumes.
    /// Writes to `projects/<id>/widgets/state.json`.
    #[serde(rename = "format_widget")]
    FormatWidget(widget::FormatWidgetInput),

    /// Build the render-ready view model the Intents Trading Agent
    /// project widget consumes. The caller persists it under
    /// `projects/intents-trading-agent/widgets/state.json`.
    #[serde(rename = "format_intents_widget")]
    FormatIntentsWidget(widget::FormatIntentsTradingWidgetInput),
}

fn default_chains() -> ChainSelector {
    ChainSelector::default()
}

fn default_source() -> String {
    "auto".to_string()
}

/// Default solver for `build_intent`. Must match a value `intents::build`
/// understands — "auto" is not a valid solver name, so `BuildIntent`
/// can't reuse `default_source()`.
fn default_solver() -> String {
    "fixture".to_string()
}

/// Bundled default strategy docs used when `propose` is called without
/// an explicit `strategies` array. Covers both EVM and NEAR yield.
fn default_strategies() -> Vec<String> {
    vec![
        include_str!("../strategies/stablecoin-yield-floor.md").to_string(),
        include_str!("../strategies/lending-health-guard.md").to_string(),
        include_str!("../strategies/lp-impermanent-loss-watch.md").to_string(),
        include_str!("../strategies/near-staking-yield.md").to_string(),
        include_str!("../strategies/near-lending-yield.md").to_string(),
        include_str!("../strategies/near-lp-yield.md").to_string(),
    ]
}

#[derive(Debug, Serialize)]
struct ScanResponse {
    positions: Vec<types::ClassifiedPosition>,
    /// Echo back the source used so callers can confirm.
    source: String,
    /// Block heights observed per chain.
    block_numbers: std::collections::BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
struct ProposeResponse {
    proposals: Vec<Proposal>,
}

#[derive(Debug, Serialize)]
struct BuildIntentResponse {
    bundle: IntentBundle,
}

impl exports::near::agent::tool::Guest for PortfolioTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Cross-chain DeFi portfolio analyzer. Discovers positions across chains, \
         classifies them against an embedded protocol registry, generates ranked \
         rebalancing proposals from declarative strategy docs, and builds unsigned \
         NEAR Intent bundles. Operations: scan, propose, build_intent, progress, \
         backtest, backtest_suite, backtest_walkforward, backtest_montecarlo, \
         backtest_grid, backtest_dca, plan_dca_schedule, compile_intent_prompt, \
         validate_strategy, format_strategy_doc, validate_episode, replay_episode, \
         plan_paid_research, \
         plan_dripstack_browse, fetch_dripstack_catalog, prepare_dripstack_paid_fetch, \
         plan_near_intents_trial, format_suggestion, format_widget, format_intents_widget. \
         Read-only and unsigned — the agent never holds private keys."
            .to_string()
    }
}

export!(PortfolioTool);

fn execute_inner(params: &str) -> Result<String, String> {
    let action: PortfolioAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    match action {
        PortfolioAction::Scan {
            addresses,
            chains,
            at,
            source,
        } => {
            let scan = indexer::scan(&addresses, &chains, at.as_ref(), &source)?;
            let classified = analyzer::classify(scan.positions)?;
            let response = ScanResponse {
                positions: classified,
                source,
                block_numbers: scan.block_numbers,
            };
            serde_json::to_string(&response).map_err(|e| format!("Serialize scan response: {e}"))
        }
        PortfolioAction::Propose {
            positions,
            strategies,
            config,
        } => {
            // If positions came in as a JSON-encoded string (common LLM
            // mistake: `json.dumps(scan["positions"])` instead of passing
            // the list directly), parse it once more to recover the array.
            let positions = if let Some(s) = positions.as_str() {
                serde_json::from_str(s).map_err(|e| {
                    format!(
                        "Invalid positions: received a JSON string but it failed to parse \
                         as ClassifiedPosition[]: {e}. Pass the positions array directly — \
                         do NOT call json.dumps() before passing."
                    )
                })?
            } else {
                positions
            };
            let positions: Vec<types::ClassifiedPosition> = serde_json::from_value(positions)
                .map_err(|e| {
                    format!(
                        "Invalid positions: {e}. Pass positions as a native JSON array \
                         (list of ClassifiedPosition objects), not a JSON-encoded string. \
                         In Python: `positions=scan['positions']`, not \
                         `positions=json.dumps(scan['positions'])`."
                    )
                })?;
            // If the caller omits strategies (or passes an empty array),
            // fall back to the bundled defaults so `propose` stays
            // useful without requiring the agent to load strategy docs
            // from the workspace on every call.
            let strategies = if strategies.is_empty() {
                default_strategies()
            } else {
                strategies
            };
            let proposals = strategy::propose(&positions, &strategies, &config)?;
            let response = ProposeResponse { proposals };
            serde_json::to_string(&response).map_err(|e| format!("Serialize propose response: {e}"))
        }
        PortfolioAction::BuildIntent {
            plan,
            config,
            solver,
        } => {
            let bundle = intents::build(&plan, &config, &solver)?;
            let response = BuildIntentResponse { bundle };
            serde_json::to_string(&response)
                .map_err(|e| format!("Serialize build_intent response: {e}"))
        }
        PortfolioAction::FormatSuggestion(input) => {
            let output = format::format_suggestion_md(input);
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize format_suggestion response: {e}"))
        }
        PortfolioAction::Progress(input) => {
            let output = format::format_progress(input);
            serde_json::to_string(&output).map_err(|e| format!("Serialize progress response: {e}"))
        }
        PortfolioAction::Backtest(input) => {
            let output = backtest::run(input)?;
            serde_json::to_string(&output).map_err(|e| format!("Serialize backtest response: {e}"))
        }
        PortfolioAction::BacktestSuite(input) => {
            let output = backtest::run_suite(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize backtest_suite response: {e}"))
        }
        PortfolioAction::BacktestWalkForward(input) => {
            let output = lab::run_walkforward(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize backtest_walkforward response: {e}"))
        }
        PortfolioAction::BacktestMonteCarlo(input) => {
            let output = lab::run_montecarlo(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize backtest_montecarlo response: {e}"))
        }
        PortfolioAction::BacktestGrid(input) => {
            let output = lab::run_grid(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize backtest_grid response: {e}"))
        }
        PortfolioAction::ValidateEpisode(input) => {
            let output = episode::validate(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize validate_episode response: {e}"))
        }
        PortfolioAction::ReplayEpisode(input) => {
            let output = episode::replay(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize replay_episode response: {e}"))
        }
        PortfolioAction::BacktestDca(input) => {
            let output = dca::run_backtest(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize backtest_dca response: {e}"))
        }
        PortfolioAction::PlanDcaSchedule(input) => {
            let output = dca::plan_schedule(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize plan_dca_schedule response: {e}"))
        }
        PortfolioAction::CompileIntentPrompt(input) => {
            let output = nl::compile(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize compile_intent_prompt response: {e}"))
        }
        PortfolioAction::ValidateStrategy(input) => {
            let output = validate::run(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize validate_strategy response: {e}"))
        }
        PortfolioAction::FormatStrategyDoc(input) => {
            let output = validate::format(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize format_strategy_doc response: {e}"))
        }
        PortfolioAction::PlanPaidResearch(input) => {
            let output = research::plan(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize plan_paid_research response: {e}"))
        }
        PortfolioAction::PlanDripstackBrowse(input) => {
            let output = research::plan_dripstack_browse(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize plan_dripstack_browse response: {e}"))
        }
        PortfolioAction::FetchDripstackCatalog(input) => {
            let output = research::fetch_dripstack_catalog(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize fetch_dripstack_catalog response: {e}"))
        }
        PortfolioAction::PrepareDripstackPaidFetch(input) => {
            let output = research::prepare_dripstack_paid_fetch(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize prepare_dripstack_paid_fetch response: {e}"))
        }
        PortfolioAction::PlanNearIntentsTrial(input) => {
            let output = trial::plan(input)?;
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize plan_near_intents_trial response: {e}"))
        }
        PortfolioAction::FormatWidget(input) => {
            let output = widget::format_widget(input);
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize format_widget response: {e}"))
        }
        PortfolioAction::FormatIntentsWidget(input) => {
            let output = widget::format_intents_trading_widget(input);
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize format_intents_widget response: {e}"))
        }
    }
}

const SCHEMA: &str = include_str!("schema.json");
