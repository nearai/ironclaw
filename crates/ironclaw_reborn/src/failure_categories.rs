/// Failure category identifier for model provider credit exhaustion.
/// Exposed for cross-crate consumers that project this category to a user-facing message.
pub const MODEL_CREDITS_EXHAUSTED_CATEGORY: &str = "model_credits_exhausted";
pub(crate) const MODEL_CREDITS_EXHAUSTED_SUMMARY: &str = "model provider account is out of credits";
