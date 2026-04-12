//! Indexer stage — fetch raw positions for a wallet.
//!
//! Three sources, selected by the `source` parameter on `scan`:
//!
//! - **`fixture`** (M1) — hand-rolled `RawPosition[]` JSON embedded
//!   in the binary. Used by smoke tests and as the M1 default.
//! - **`dune`** (M2) — production path. Calls Dune Sim REST via
//!   `host::http_request`. Only works inside the WASM sandbox.
//! - **`dune-replay`** (M2) — reads recorded Dune JSON responses from
//!   disk and runs them through the production parser. Used by the
//!   CI replay scenarios.
//!
//! Adding a fourth source is one match arm + a new module.

use std::collections::BTreeMap;

use crate::types::{ChainSelector, RawPosition, ScanAt};

pub mod dune;
mod dune_replay;
mod fixture;

pub struct ScanResult {
    pub positions: Vec<RawPosition>,
    pub block_numbers: BTreeMap<String, u64>,
}

pub fn scan(
    addresses: &[String],
    chains: &ChainSelector,
    at: Option<&ScanAt>,
    source: &str,
) -> Result<ScanResult, String> {
    if addresses.is_empty() {
        return Ok(ScanResult {
            positions: Vec::new(),
            block_numbers: BTreeMap::new(),
        });
    }

    match source {
        "fixture" => fixture::scan(addresses, chains, at),
        "dune" => dune::scan(addresses, chains, at),
        "dune-replay" => dune_replay::scan(addresses, chains, at),
        other => Err(format!("Unknown indexer source: '{other}'")),
    }
}
