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
///
/// NEAR account rules (enforced here to avoid shipping garbage to the
/// FastNEAR `/v1/account/{id}/full` endpoint):
/// - 2..=64 characters
/// - lowercase ASCII letters, digits, `_`, `-`, `.`
/// - no leading/trailing `.` or `-` or `_`, no consecutive `.`
/// - OR a 64-char lowercase hex string (implicit account)
///
/// See https://nomicon.io/DataStructures/Account for the full spec.
fn is_near_address(address: &str) -> bool {
    // EVM addresses are 0x-prefixed hex — explicitly not NEAR.
    if address.starts_with("0x") || address.starts_with("0X") {
        return false;
    }
    // NEAR implicit accounts are 64-char lowercase hex.
    if address.len() == 64
        && address
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return true;
    }
    is_valid_near_named_account(address)
}

fn is_evm_address(address: &str) -> bool {
    let Some(hex) = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
    else {
        return false;
    };
    hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_valid_near_named_account(s: &str) -> bool {
    let len = s.len();
    if !(2..=64).contains(&len) {
        return false;
    }
    let bytes = s.as_bytes();
    // Must start and end with an alphanumeric lowercase char.
    let valid_edge = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !valid_edge(bytes[0]) || !valid_edge(bytes[len - 1]) {
        return false;
    }
    let mut prev_sep = false;
    for &b in bytes {
        let is_sep = matches!(b, b'-' | b'_' | b'.');
        let is_alnum = b.is_ascii_lowercase() || b.is_ascii_digit();
        if !is_sep && !is_alnum {
            return false;
        }
        if is_sep && prev_sep {
            // No `..`, `__`, `.-`, etc. runs of separators.
            return false;
        }
        prev_sep = is_sep;
    }
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
        } else if is_evm_address(addr) {
            evm_addrs.push(addr.clone());
        } else {
            return Err(format!(
                "address '{addr}' is neither a valid EVM address (0x + 40 hex) nor a valid NEAR account id"
            ));
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
        // 64-char lowercase hex without 0x prefix = NEAR implicit account
        assert!(is_near_address(
            "98793cd91a3f870fb126f66285808c7e094afcfc4eda8a970f6648cdf0dbd6de"
        ));
    }

    #[test]
    fn is_near_address_rejects_uppercase_implicit() {
        // Uppercase hex is not a valid implicit account id.
        assert!(!is_near_address(
            "98793CD91A3F870FB126F66285808C7E094AFCFC4EDA8A970F6648CDF0DBD6DE"
        ));
    }

    #[test]
    fn is_near_address_rejects_garbage() {
        // Regression: previously these all returned true and got
        // shipped to FastNEAR as `/v1/account/{garbage}/full`.
        assert!(!is_near_address(""));
        assert!(!is_near_address(" "));
        assert!(!is_near_address("  spaces  "));
        assert!(!is_near_address("drop table"));
        assert!(!is_near_address("🦀"));
        assert!(!is_near_address("UPPERCASE"));
        assert!(!is_near_address(".leading-dot"));
        assert!(!is_near_address("trailing-dot."));
        assert!(!is_near_address("double..dot"));
        assert!(!is_near_address("a"));
        assert!(!is_near_address(&"x".repeat(65)));
        assert!(!is_near_address("has/slash"));
        assert!(!is_near_address("has@at"));
        assert!(!is_near_address("../etc/passwd"));
    }

    #[test]
    fn is_near_address_accepts_edge_named_accounts() {
        assert!(is_near_address("ab"));
        assert!(is_near_address("a-b.c"));
        assert!(is_near_address("test_account.near"));
        assert!(is_near_address(&"a".repeat(64)));
    }

    #[test]
    fn is_evm_address_accepts_valid() {
        assert!(is_evm_address("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"));
        assert!(is_evm_address("0x0000000000000000000000000000000000000000"));
    }

    #[test]
    fn is_evm_address_rejects_wrong_length() {
        assert!(!is_evm_address("0xabcd"));
        assert!(!is_evm_address(&format!("0x{}", "a".repeat(41))));
    }

    #[test]
    fn is_evm_address_rejects_no_prefix() {
        // A 40-char hex string without 0x prefix is not an EVM address.
        assert!(!is_evm_address(&"a".repeat(40)));
    }

    #[test]
    fn scan_auto_rejects_garbage_address() {
        let addrs = vec!["not a wallet".to_string()];
        let res = scan_auto(&addrs, &ChainSelector::default(), None);
        match res {
            Err(msg) => assert!(msg.contains("neither a valid"), "got: {msg}"),
            Ok(_) => panic!("expected error for garbage address"),
        }
    }
}
