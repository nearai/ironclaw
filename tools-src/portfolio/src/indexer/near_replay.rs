//! `near-replay` indexer source — read recorded FastNEAR + Intear
//! JSON responses from disk and run the production parser over them.
//!
//! Fixture layout:
//!
//! ```text
//! tools-src/portfolio/fixtures/near/
//! ├── <account_id>.json       # FastNEAR /v1/account/{id}/full response
//! └── intear_tokens.json      # Intear /tokens response (shared across all accounts)
//! ```
//!
//! Recording new fixtures:
//!
//! ```bash
//! curl "https://api.fastnear.com/v1/account/root.near/full" \
//!   > tools-src/portfolio/fixtures/near/root.near.json
//! curl "https://prices.intear.tech/tokens" \
//!   > tools-src/portfolio/fixtures/near/intear_tokens.json
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::types::{ChainSelector, RawPosition, ScanAt};

use super::near::parse_fastnear_response;
use super::ScanResult;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/near")
}

/// Cache the Intear prices file (~235 KB) shared across addresses.
fn intear_json() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(|| {
        let root = fixtures_root();
        // Try new name first, fall back to old name for existing fixtures
        let path = root.join("intear_prices.json");
        if path.exists() {
            return std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!("near-replay: read Intear fixture {}: {e}", path.display())
            });
        }
        let legacy = root.join("intear_tokens.json");
        std::fs::read_to_string(&legacy).unwrap_or_else(|e| {
            panic!(
                "near-replay: missing Intear fixture {} or {}: {e}",
                path.display(),
                legacy.display()
            )
        })
    })
}

pub fn scan(
    addresses: &[String],
    _chains: &ChainSelector,
    _at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    let root = fixtures_root();
    let intear = intear_json();
    let mut all_positions: Vec<RawPosition> = Vec::new();
    let mut block_numbers: BTreeMap<String, u64> = BTreeMap::new();

    for address in addresses {
        let key = address.to_ascii_lowercase();
        let fastnear_path = root.join(format!("{key}.json"));
        let fastnear_json = std::fs::read_to_string(&fastnear_path).map_err(|e| {
            format!(
                "near-replay: missing FastNEAR fixture {}: {e}",
                fastnear_path.display()
            )
        })?;

        let mut positions = parse_fastnear_response(&fastnear_json, intear, address, 0)?;
        for raw in &positions {
            if raw.block_number > 0 {
                block_numbers
                    .entry(raw.chain.clone())
                    .and_modify(|b| *b = (*b).max(raw.block_number))
                    .or_insert(raw.block_number);
            }
        }
        all_positions.append(&mut positions);
    }

    Ok(ScanResult {
        positions: all_positions,
        block_numbers,
    })
}
