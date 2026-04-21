//! Intents stage — translate a `MovementPlan` into an unsigned NEAR
//! Intent bundle.
//!
//! Three solver sources, selected by the `solver` parameter on
//! `build_intent`:
//!
//! - **`fixture`** (M1) — synthesizes a single-leg bundle from the
//!   plan. No HTTP, no fixtures. Used by smoke tests.
//! - **`near-intents`** (M4) — production path. Calls the NEAR
//!   Intents solver relay via `host::http_request`. Only works
//!   inside the WASM sandbox.
//! - **`replay`** (M4) — reads a recorded solver response from
//!   `fixtures/solver/<key>.json` and parses it through the
//!   production code path. Used by CI replay scenarios and the
//!   `hostile/solver-bad-quote` scenario.
//!
//! Bounded checks (`bounded.rs`) run on every source. A failing
//! check is a hard error — the skill marks the proposal `unmet-route`
//! and the mission logs it for the next run.

use crate::types::{BoundedChecks, IntentBundle, MovementPlan, ProjectConfig};

mod bounded;
mod bundling;
mod fixture;
pub mod solver;

pub fn build(
    plan: &MovementPlan,
    config: &ProjectConfig,
    solver_name: &str,
) -> Result<IntentBundle, String> {
    let bundle = match solver_name {
        "fixture" => fixture::build(plan, config)?,
        "near-intents" => {
            let quote = solver::fetch_quote(plan, config.max_slippage_bps)?;
            build_from_solver_response(plan, quote, config)?
        }
        "replay" => {
            let quote = solver::load_quote_fixture(plan)?;
            build_from_solver_response(plan, quote, config)?
        }
        other => return Err(format!("Unknown intent solver: '{other}'")),
    };

    bounded::check(&bundle, plan, config)?;
    Ok(bundle)
}

fn build_from_solver_response(
    _plan: &MovementPlan,
    response: solver::SolverQuoteResponse,
    config: &ProjectConfig,
) -> Result<IntentBundle, String> {
    let legs = solver::response_to_legs(&response);
    let legs = bundling::order_legs(legs)?;
    let min_out_per_leg = legs.iter().map(|l| l.min_out.clone()).collect();

    Ok(IntentBundle {
        id: response.quote_id.clone(),
        legs,
        total_cost_usd: response.total_cost_usd.clone(),
        bounded_checks: BoundedChecks {
            min_out_per_leg,
            max_slippage_bps: config.max_slippage_bps,
            solver_quote_version: response.quote_version.clone(),
        },
        expires_at: response.expires_at_unix,
        signer_placeholder: "<signed-by-user>".to_string(),
        schema_version: "portfolio-intent/1".to_string(),
    })
}
