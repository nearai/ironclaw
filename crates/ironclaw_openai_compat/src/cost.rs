//! IronClaw cost extension for OpenAI-compatible usage objects.
//!
//! OpenAI's Chat Completions and Responses APIs report token `usage` but no
//! monetary cost. IronClaw adds a namespaced `cost` object so callers can see
//! the USD spend of a response without maintaining their own price table.
//!
//! Amounts are decimal strings (e.g. `"0.000042"`), not JSON floats: per-token
//! prices are tiny and float rounding would corrupt them. The route crate keeps
//! no `rust_decimal` dependency — host composition formats the `Decimal` prices
//! into these strings when it fills the usage object.

use serde::{Deserialize, Serialize};

/// Computed USD cost for a completed response. Attached to the surface's usage
/// object as the `cost` field. `currency` is always [`OpenAiCompatCost::USD`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompatCost {
    /// Cost of the (uncached) input tokens.
    pub input_cost_usd: String,
    /// Cost of the cached-input tokens (a subset of the input tokens, priced at
    /// the provider's cache-read discount). `"0"` when nothing was cached.
    pub cached_input_cost_usd: String,
    /// Cost of the output tokens.
    pub output_cost_usd: String,
    /// Sum of the three components above.
    pub total_cost_usd: String,
    /// Currency of every amount above. Always `"USD"`.
    pub currency: String,
}

impl OpenAiCompatCost {
    /// The only currency IronClaw prices in.
    pub const USD: &'static str = "USD";
}
