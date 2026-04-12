// M1 scaffold: several types/methods are reserved for M2+ stages and
// are intentionally unused right now. Tightening to per-item allows
// once the surface stops moving.
#![allow(dead_code)]

//! Portfolio WASM tool for IronClaw.
//!
//! Single tool with three operations:
//!
//! - `scan` — discover positions across chains for one or more addresses
//!   and classify them against the embedded protocol registry.
//! - `propose` — given classified positions and a set of strategy docs,
//!   produce ranked rebalancing proposals (deterministic constraint
//!   filter; LLM does the final ranking via the skill playbook).
//! - `build_intent` — translate a movement plan into an unsigned NEAR
//!   Intent bundle, applying bounded checks before returning.
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
//! └── intents/            // build_intent: solver call + bounded checks
//! ```

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

mod analyzer;
mod format;
mod indexer;
mod intents;
mod strategy;
mod types;
mod widget;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod replay_tests;

use types::{ChainSelector, IntentBundle, MovementPlan, Proposal, ProjectConfig, ScanAt};

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
        /// Raw markdown strategy docs (with YAML frontmatter).
        strategies: Vec<String>,
        config: ProjectConfig,
    },

    /// Translate a movement plan into an unsigned NEAR Intent bundle.
    #[serde(rename = "build_intent")]
    BuildIntent {
        plan: MovementPlan,
        config: ProjectConfig,
        /// Solver source. M1: only "fixture". M4: "near-intents".
        #[serde(default = "default_source")]
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

    /// Build the render-ready view model the web widget consumes.
    /// Writes to `projects/<id>/widgets/state.json`.
    #[serde(rename = "format_widget")]
    FormatWidget(widget::FormatWidgetInput),
}

fn default_chains() -> ChainSelector {
    ChainSelector::default()
}

fn default_source() -> String {
    "fixture".to_string()
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
         NEAR Intent bundles. Three operations: scan, propose, build_intent. \
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
            let positions: Vec<types::ClassifiedPosition> = serde_json::from_value(positions)
                .map_err(|e| format!("Invalid positions: {e}"))?;
            let proposals = strategy::propose(&positions, &strategies, &config)?;
            let response = ProposeResponse { proposals };
            serde_json::to_string(&response)
                .map_err(|e| format!("Serialize propose response: {e}"))
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
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize progress response: {e}"))
        }
        PortfolioAction::FormatWidget(input) => {
            let output = widget::format_widget(input);
            serde_json::to_string(&output)
                .map_err(|e| format!("Serialize format_widget response: {e}"))
        }
    }
}

const SCHEMA: &str = include_str!("schema.json");
