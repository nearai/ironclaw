//! Cost-table types consumed by [`crate::GovernorBackedAccountant`].
//!
//! A [`ModelCostTable`] resolves a [`ModelProfileId`] to a [`ModelCost`]
//! (per-token USD prices + model max-output tokens). Implementations
//! bridge the `LlmProvider::cost_per_token()` family from
//! `ironclaw_llm` into the loop layer without re-exporting LLM crate
//! types. This crate ships two: [`ZeroCostTable`] for free/local
//! providers and tests, and [`StaticModelCostTable`] for composition-
//! driven lookups.

use std::collections::HashMap;

use ironclaw_turns::run_profile::ModelProfileId;
use rust_decimal::Decimal;

/// Static cost-per-token + max-output-tokens table for a single model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelCost {
    /// Input USD per token. `Decimal::ZERO` for free/local models.
    pub input_per_token: Decimal,
    /// Output USD per token. `Decimal::ZERO` for free/local models.
    pub output_per_token: Decimal,
    /// Model's max output tokens — used for worst-case pre-call estimate.
    /// `0` is treated as "unknown" and falls back to
    /// [`ModelCostTable::DEFAULT_MAX_OUTPUT_TOKENS`].
    pub max_output_tokens: u64,
}

/// Resolves [`ModelProfileId`] → [`ModelCost`]. Implementations bridge
/// the `LlmProvider::cost_per_token()` family from `ironclaw_llm` into
/// the loop layer without re-exporting LLM crate types.
pub trait ModelCostTable: Send + Sync + std::fmt::Debug {
    fn cost_for(&self, model: &ModelProfileId) -> Option<ModelCost>;
}

impl dyn ModelCostTable {
    /// Conservative fallback when a model's max_output_tokens is unknown.
    /// 8 KiB tokens covers most chat completions; reservations release
    /// the overshoot in `reconcile`.
    pub const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 8_192;
}

/// Constant cost table used in tests and as a safe baseline for
/// free/local providers. Every model returns `(0, 0, 0)` so reservation
/// succeeds with a zero-USD estimate.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZeroCostTable;

impl ModelCostTable for ZeroCostTable {
    fn cost_for(&self, _model: &ModelProfileId) -> Option<ModelCost> {
        Some(ModelCost {
            input_per_token: Decimal::ZERO,
            output_per_token: Decimal::ZERO,
            max_output_tokens: 0,
        })
    }
}

/// Static `(ModelProfileId → ModelCost)` lookup. Composition layers
/// populate this from their model-route registry (provider model name →
/// known per-token price via `ironclaw_llm::costs::model_cost`) so the
/// accountant can compute actual USD spend on every reconcile.
///
/// Profiles missing from the table fall back to `None`, which the
/// accountant treats as zero-cost (free/local). That matches the safety
/// direction we want: an unknown provider must not silently overstate
/// spend.
#[derive(Debug, Default, Clone)]
pub struct StaticModelCostTable {
    costs: HashMap<ModelProfileId, ModelCost>,
}

impl StaticModelCostTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_entry(mut self, profile: ModelProfileId, cost: ModelCost) -> Self {
        self.costs.insert(profile, cost);
        self
    }

    pub fn insert(&mut self, profile: ModelProfileId, cost: ModelCost) {
        self.costs.insert(profile, cost);
    }

    pub fn len(&self) -> usize {
        self.costs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.costs.is_empty()
    }
}

impl ModelCostTable for StaticModelCostTable {
    fn cost_for(&self, model: &ModelProfileId) -> Option<ModelCost> {
        self.costs.get(model).copied()
    }
}
