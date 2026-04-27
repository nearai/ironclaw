//! Fixture indexer backend.
//!
//! Reads canned `RawPosition[]` responses from a JSON file embedded at
//! compile time. The fixture file is a JSON object keyed by lowercased
//! wallet address — every address in the scan request is looked up
//! independently and the results are concatenated.
//!
//! Used for M1 smoke tests, replay scenarios, and for any future test
//! that needs deterministic indexer output. The fixture is part of the
//! WASM binary, so the host doesn't need to grant workspace read.
//!
//! Adding a new fixture: edit `fixtures.json` and add an entry under
//! the lowercased address. Unknown addresses return an empty list (not
//! an error) so smoke-empty-wallet is just "any address not in the
//! file".

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::types::{ChainSelector, RawPosition, ScanAt};

use super::ScanResult;

const FIXTURE_DATA: &str = include_str!("fixtures.json");

#[derive(Debug, Deserialize)]
struct FixtureFile {
    #[serde(default)]
    addresses: BTreeMap<String, Vec<RawPosition>>,
}

pub fn scan(
    addresses: &[String],
    _chains: &ChainSelector,
    _at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    let file: FixtureFile = serde_json::from_str(FIXTURE_DATA)
        .map_err(|e| format!("Embedded fixture file is invalid JSON: {e}"))?;

    let mut positions = Vec::new();
    let mut block_numbers: BTreeMap<String, u64> = BTreeMap::new();

    for addr in addresses {
        let key = addr.to_ascii_lowercase();
        if let Some(entries) = file.addresses.get(&key) {
            for raw in entries {
                let chain = raw.chain.clone();
                block_numbers
                    .entry(chain)
                    .and_modify(|b| *b = (*b).max(raw.block_number))
                    .or_insert(raw.block_number);
                positions.push(raw.clone());
            }
        }
    }

    Ok(ScanResult {
        positions,
        block_numbers,
    })
}
