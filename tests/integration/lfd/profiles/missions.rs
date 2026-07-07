//! Skeleton profile for the missions lane.

use async_trait::async_trait;

use super::{LfdProfile, ProfileError};
use crate::case::Case;
use crate::reborn_support::builder::RebornIntegrationHarness;

pub const NAME: &str = "missions";

pub struct Missions;

#[async_trait]
impl LfdProfile for Missions {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, _case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        Err(ProfileError::Unsupported(
            "missions profile skeleton is not wired yet".to_string(),
        ))
    }
}
