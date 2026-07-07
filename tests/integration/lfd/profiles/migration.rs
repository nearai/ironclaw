//! Skeleton profile for the old-architecture migration/cleanup lane.

use async_trait::async_trait;

use super::{LfdProfile, ProfileError};
use crate::case::Case;
use crate::reborn_support::builder::RebornIntegrationHarness;

pub const NAME: &str = "migration";

pub struct Migration;

#[async_trait]
impl LfdProfile for Migration {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, _case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        Err(ProfileError::Unsupported(
            "migration profile skeleton is not wired yet".to_string(),
        ))
    }
}
