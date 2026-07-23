//! IronClaw composition root-glue cluster — assorted composition-root helpers
//! (product-live adapter bundle, communication context, default system prompt,
//! psychographic profile) grouped behind one internal module. The crate root
//! re-exports the same public items so the public API is unchanged.

pub(crate) mod communication_context;
pub(crate) mod default_system_prompt;
pub(crate) mod product_live_adapters;
pub(crate) mod profile;
