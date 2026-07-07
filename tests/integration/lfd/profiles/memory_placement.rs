//! Skeleton profile for the memory-placement product/provider lane.

use async_trait::async_trait;

use super::{LfdProfile, ProfileError};
use crate::case::Case;
use crate::reborn_support::builder::RebornIntegrationHarness;

pub const NAME: &str = "memory_placement";

pub struct MemoryPlacement;

#[async_trait]
impl LfdProfile for MemoryPlacement {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, _case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        Err(ProfileError::Unsupported(
            "memory_placement profile skeleton is not wired yet".to_string(),
        ))
    }
}
