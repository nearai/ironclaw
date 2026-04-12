//! `dune-replay` indexer source — read recorded Dune Sim JSON
//! responses from disk and run the production parser over them.
//!
//! This is the path the replay test suite uses to verify the parser
//! and the analyzer against shape-accurate fixtures without making
//! HTTP calls. **Not used in production WASM builds**: at runtime in
//! the sandbox `std::fs` access is gated by host capabilities and the
//! recorded files don't ship with the binary anyway.
//!
//! Fixture layout:
//!
//! ```text
//! tools-src/portfolio/fixtures/dune/
//! ├── balances/
//! │   └── <lowercased-address>.json   # Dune balances response shape
//! └── positions/
//!     └── <lowercased-address>.json   # Dune activity/positions shape
//! ```
//!
//! Either file may be missing for a given address (the parser tolerates
//! it). Address lookups are case-insensitive.
//!
//! Recording new fixtures (M3+ workflow):
//!
//! ```bash
//! curl -H "X-Sim-Api-Key: $DUNE_API_KEY" \
//!   "https://api.sim.dune.com/v1/evm/balances/0x...." \
//!   > tools-src/portfolio/fixtures/dune/balances/0x....json
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::types::{ChainSelector, RawPosition, ScanAt};

use super::dune::{parse_balances_response, parse_positions_response};
use super::ScanResult;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/dune")
}

pub fn scan(
    addresses: &[String],
    _chains: &ChainSelector,
    _at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    let root = fixtures_root();
    let mut all_positions: Vec<RawPosition> = Vec::new();
    let mut block_numbers: BTreeMap<String, u64> = BTreeMap::new();

    for address in addresses {
        let key = address.to_lowercase();

        // Balances (required for the address to be considered scanned).
        let balances_path = root.join("balances").join(format!("{key}.json"));
        let balances_json = std::fs::read_to_string(&balances_path).map_err(|e| {
            format!(
                "dune-replay: missing balances fixture {}: {e}",
                balances_path.display()
            )
        })?;
        let mut from_balances = parse_balances_response(&balances_json, address, 0)?;
        for raw in &from_balances {
            block_numbers
                .entry(raw.chain.clone())
                .and_modify(|b| *b = (*b).max(raw.block_number))
                .or_insert(raw.block_number);
        }
        all_positions.append(&mut from_balances);

        // Positions (optional enrichment).
        let positions_path = root.join("positions").join(format!("{key}.json"));
        if positions_path.exists() {
            let positions_json = std::fs::read_to_string(&positions_path).map_err(|e| {
                format!(
                    "dune-replay: read positions fixture {}: {e}",
                    positions_path.display()
                )
            })?;
            let mut from_positions = parse_positions_response(&positions_json, address, 0)?;
            for raw in &from_positions {
                block_numbers
                    .entry(raw.chain.clone())
                    .and_modify(|b| *b = (*b).max(raw.block_number))
                    .or_insert(raw.block_number);
            }
            all_positions.append(&mut from_positions);
        }
    }

    Ok(ScanResult {
        positions: all_positions,
        block_numbers,
    })
}
