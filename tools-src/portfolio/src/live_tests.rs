//! Live integration tests that hit real external APIs.
//!
//! All tests in this module are `#[ignore]` by default. They require:
//!
//!   - `DUNE_API_KEY` environment variable
//!   - Network access to `api.sim.dune.com`
//!
//! Run with:
//!
//! ```bash
//! DUNE_API_KEY=... cargo test -p portfolio-tool -- --ignored
//! ```
//!
//! These tests validate that the Dune API response shape hasn't drifted
//! from what the M2 parser expects, and that the full pipeline
//! (indexer parse → analyzer → strategy) produces coherent output
//! against live data.

use crate::analyzer;
use crate::indexer::dune;
use crate::strategy;
use crate::types::ProjectConfig;

const VITALIK_ADDRESS: &str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

fn dune_api_key() -> String {
    std::env::var("DUNE_API_KEY").expect(
        "DUNE_API_KEY not set; live tests require a real Dune Sim API key",
    )
}

fn dune_get(url: &str, api_key: &str) -> Result<String, String> {
    let response = ureq::get(url)
        .set("X-Sim-Api-Key", api_key)
        .set("Accept", "application/json")
        .set("User-Agent", "IronClaw-Portfolio-Tool-LiveTest/0.1")
        .call()
        .map_err(|e| format!("Dune HTTP error: {e}"))?;

    response
        .into_string()
        .map_err(|e| format!("Dune response read error: {e}"))
}

#[test]
#[ignore]
fn live_dune_balances_parse() {
    let key = dune_api_key();
    let url = format!(
        "https://api.sim.dune.com/v1/evm/balances/{VITALIK_ADDRESS}"
    );
    let json = dune_get(&url, &key).expect("Dune balances API call");

    let raw = dune::parse_balances_response(&json, VITALIK_ADDRESS, 1_700_000_000)
        .expect("parse_balances_response against live data");

    assert!(
        !raw.is_empty(),
        "Vitalik's wallet should have at least one balance"
    );

    for pos in &raw {
        assert!(!pos.chain.is_empty(), "chain must not be empty");
        assert!(
            !pos.token_balances.is_empty(),
            "each position should have at least one token balance"
        );
        assert!(
            !pos.token_balances[0].symbol.is_empty(),
            "token symbol must not be empty"
        );
    }

    eprintln!(
        "live_dune_balances_parse: parsed {} positions across {} chains",
        raw.len(),
        raw.iter()
            .map(|p| p.chain.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
}

#[test]
#[ignore]
fn live_dune_positions_parse() {
    let key = dune_api_key();
    let url = format!(
        "https://api.sim.dune.com/v1/evm/activity/{VITALIK_ADDRESS}"
    );

    match dune_get(&url, &key) {
        Ok(json) => {
            let raw = dune::parse_positions_response(&json, VITALIK_ADDRESS, 1_700_000_000)
                .expect("parse_positions_response against live data");

            eprintln!(
                "live_dune_positions_parse: parsed {} DeFi positions",
                raw.len()
            );

            for pos in &raw {
                assert!(!pos.chain.is_empty(), "chain must not be empty");
                assert!(!pos.protocol_id.is_empty(), "protocol_id must not be empty");
            }
        }
        Err(e) => {
            eprintln!(
                "live_dune_positions_parse: activity endpoint returned error \
                 (non-fatal, balances-only flow still works): {e}"
            );
        }
    }
}

#[test]
#[ignore]
fn live_full_pipeline_scan_classify_propose() {
    let key = dune_api_key();
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Step 1: Fetch balances from Dune
    let balances_url = format!(
        "https://api.sim.dune.com/v1/evm/balances/{VITALIK_ADDRESS}"
    );
    let balances_json = dune_get(&balances_url, &key).expect("Dune balances API call");
    let mut raw_positions =
        dune::parse_balances_response(&balances_json, VITALIK_ADDRESS, now_secs)
            .expect("parse balances");

    // Step 1b: Enrich with positions (best-effort)
    let positions_url = format!(
        "https://api.sim.dune.com/v1/evm/activity/{VITALIK_ADDRESS}"
    );
    if let Ok(positions_json) = dune_get(&positions_url, &key) {
        if let Ok(mut from_positions) =
            dune::parse_positions_response(&positions_json, VITALIK_ADDRESS, now_secs)
        {
            raw_positions.append(&mut from_positions);
        }
    }

    assert!(
        !raw_positions.is_empty(),
        "should have at least one raw position from Dune"
    );

    // Step 2: Classify through analyzer
    let raw_count = raw_positions.len();
    let classified = analyzer::classify(raw_positions).expect("analyzer::classify");

    eprintln!(
        "live_full_pipeline: {} raw → {} classified positions",
        raw_count,
        classified.len()
    );

    // Step 3: Run strategy proposals
    let strategies = vec![
        include_str!("../strategies/stablecoin-yield-floor.md").to_string(),
        include_str!("../strategies/lending-health-guard.md").to_string(),
        include_str!("../strategies/lp-impermanent-loss-watch.md").to_string(),
    ];
    let config = ProjectConfig::default();

    let proposals = strategy::propose(&classified, &strategies, &config)
        .expect("strategy::propose");

    eprintln!(
        "live_full_pipeline: {} proposals generated ({} ready)",
        proposals.len(),
        proposals.iter().filter(|p| p.status == "ready").count()
    );

    // Validate proposal structure
    for p in &proposals {
        assert!(!p.id.is_empty(), "proposal id must not be empty");
        assert!(!p.strategy_id.is_empty(), "strategy_id must not be empty");
        assert!(
            ["ready", "below-threshold", "blocked-by-constraint", "unmet-route"]
                .contains(&p.status.as_str()),
            "unexpected proposal status: {}",
            p.status
        );
    }
}

#[test]
#[ignore]
fn live_dune_replay_fixture_record() {
    let key = dune_api_key();
    let address = VITALIK_ADDRESS.to_lowercase();

    let balances_url = format!(
        "https://api.sim.dune.com/v1/evm/balances/{address}"
    );
    let balances_json = dune_get(&balances_url, &key).expect("Dune balances API call");

    let fixtures_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/dune");
    let balances_dir = fixtures_root.join("balances");
    std::fs::create_dir_all(&balances_dir).expect("create balances dir");
    std::fs::write(
        balances_dir.join(format!("{address}.json")),
        &balances_json,
    )
    .expect("write balances fixture");

    let positions_url = format!(
        "https://api.sim.dune.com/v1/evm/activity/{address}"
    );
    if let Ok(positions_json) = dune_get(&positions_url, &key) {
        let positions_dir = fixtures_root.join("positions");
        std::fs::create_dir_all(&positions_dir).expect("create positions dir");
        std::fs::write(
            positions_dir.join(format!("{address}.json")),
            &positions_json,
        )
        .expect("write positions fixture");
    }

    eprintln!(
        "live_dune_replay_fixture_record: recorded fixtures for {address} \
         under {}/",
        fixtures_root.display()
    );
}
