//! The human-readable transaction view shown at the gate.
//!
//! [`render`] is derived from the *same* [`crate::fields::project`] projection
//! that [`crate::canonical::canonical_signing_bytes`] consumes, so the view and
//! the signed bytes can never diverge (see `crate::fields`).

use serde::{Deserialize, Serialize};

use crate::decoded_tx::{DecodedTransaction, RenderingSchemaVersion};
use crate::fields::project;

/// One displayed label/value pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedField {
    /// Human-readable field label.
    pub label: String,
    /// Human-readable field value.
    pub value: String,
}

/// The structured, human-facing rendering of a decoded transaction.
///
/// This is the concrete render type one layer above the opaque
/// `ironclaw_signing_provider::RenderedTx` forward declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedTx {
    /// Schema version this render was produced under (bound into the hash).
    pub schema_version: RenderingSchemaVersion,
    /// Chain tag (e.g. `evm`, `solana`, `near`).
    pub chain: String,
    /// Chain/network identity (e.g. `eip155:1`, `solana:mainnet-beta`).
    pub chain_network: String,
    /// Transaction-type label.
    pub tx_type: String,
    /// Every signing-relevant field, in canonical order.
    pub fields: Vec<RenderedField>,
}

impl RenderedTx {
    /// Whether the render surfaces a field whose human value contains `needle`.
    /// Used by render-coverage tests to assert every consumed field is visible.
    pub fn mentions_value(&self, needle: &str) -> bool {
        self.fields.iter().any(|f| f.value.contains(needle))
    }

    /// Whether the render carries a field with the given label.
    pub fn has_label(&self, label: &str) -> bool {
        self.fields.iter().any(|f| f.label == label)
    }
}

/// Render a decoded transaction into its human-facing view.
///
/// The renderer surfaces EVERY signing-relevant field that
/// [`crate::canonical::canonical_signing_bytes`] consumes — there are no silent
/// fields, because both walk [`crate::fields::project`].
pub fn render(tx: &DecodedTransaction, schema_version: RenderingSchemaVersion) -> RenderedTx {
    let fields = project(tx)
        .into_iter()
        .map(|f| RenderedField {
            label: f.label.to_string(),
            value: f.value,
        })
        .collect();
    RenderedTx {
        schema_version,
        chain: tx.chain_tag().to_string(),
        chain_network: tx.chain_network(),
        tx_type: tx.tx_type_label(),
        fields,
    }
}
