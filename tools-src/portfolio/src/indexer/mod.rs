//! Indexer stage — fetch raw positions for a wallet.
//!
//! Sources, selected by the `source` parameter on `scan`:
//!
//! - **`fixture`** (M1) — hand-rolled `RawPosition[]` JSON embedded
//!   in the binary. Used by smoke tests and as the M1 default.
//! - **`dune`** (M2) — EVM production path. Calls Dune Sim REST via
//!   `host::http_request`. Only works inside the WASM sandbox.
//! - **`dune-replay`** (M2) — reads recorded Dune JSON responses from
//!   disk and runs them through the production parser. Used by the
//!   CI replay scenarios.
//! - **`near`** — NEAR production path. Calls FastNEAR + Intear APIs.
//!   Only works inside the WASM sandbox.
//! - **`auto`** — auto-detect per address: NEAR accounts (containing
//!   `.near`, `.tg`, or no `0x` prefix) go to `near`, EVM addresses
//!   (`0x...`) go to `dune`. Mixed address lists are split and merged.

use std::collections::BTreeMap;

use crate::types::{ChainSelector, RawPosition, ScanAt};

pub mod dune;
mod dune_replay;
mod fixture;
pub mod near;
mod near_replay;

pub struct ScanResult {
    pub positions: Vec<RawPosition>,
    pub block_numbers: BTreeMap<String, u64>,
}

/// Returns true if the address looks like a NEAR account rather than
/// an EVM address.
fn is_near_address(address: &str) -> bool {
    // EVM addresses are 0x-prefixed hex
    if address.starts_with("0x") || address.starts_with("0X") {
        return false;
    }
    // NEAR implicit accounts are 64-char hex (no 0x prefix)
    if address.len() == 64 && address.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    // Named NEAR accounts: contain a dot or are plain alphanumeric
    true
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
        "near" => near::scan(addresses, chains, at),
        "near-replay" => near_replay::scan(addresses, chains, at),
        "auto" => scan_auto(addresses, chains, at),
        other => Err(format!("Unknown indexer source: '{other}'")),
    }
}

/// Auto-detect address type and route to the appropriate backend.
fn scan_auto(
    addresses: &[String],
    chains: &ChainSelector,
    at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    let mut near_addrs = Vec::new();
    let mut evm_addrs = Vec::new();

    for addr in addresses {
        if is_near_address(addr) {
            near_addrs.push(addr.clone());
        } else {
            evm_addrs.push(addr.clone());
        }
    }

    let mut all_positions: Vec<RawPosition> = Vec::new();
    let mut block_numbers: BTreeMap<String, u64> = BTreeMap::new();

    if !evm_addrs.is_empty() {
        let evm_result = dune::scan(&evm_addrs, chains, at)?;
        for raw in &evm_result.positions {
            block_numbers
                .entry(raw.chain.clone())
                .and_modify(|b| *b = (*b).max(raw.block_number))
                .or_insert(raw.block_number);
        }
        all_positions.extend(evm_result.positions);
    }

    if !near_addrs.is_empty() {
        let near_result = near::scan(&near_addrs, chains, at)?;
        for raw in &near_result.positions {
            if raw.block_number > 0 {
                block_numbers
                    .entry(raw.chain.clone())
                    .and_modify(|b| *b = (*b).max(raw.block_number))
                    .or_insert(raw.block_number);
            }
        }
        all_positions.extend(near_result.positions);
    }

    Ok(ScanResult {
        positions: all_positions,
        block_numbers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_near_address_detects_named_accounts() {
        assert!(is_near_address("root.near"));
        assert!(is_near_address("alice.near"));
        assert!(is_near_address("relay.tg"));
        assert!(is_near_address("illia.near"));
    }

    #[test]
    fn is_near_address_detects_evm() {
        assert!(!is_near_address(
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
        ));
        assert!(!is_near_address(
            "0x0000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn is_near_address_detects_implicit_accounts() {
        // 64-char hex without 0x prefix = NEAR implicit account
        assert!(is_near_address(
            "98793cd91a3f870fb126f66285808c7e094afcfc4eda8a970f6648cdf0dbd6de"
        ));
    }
}
