//! Reborn composition root-glue cluster: the default system prompt and
//! psychographic profile, plus a feature-gated ProductLive test fixture.

pub(crate) mod default_system_prompt;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod product_live_adapters;
pub(crate) mod profile;
