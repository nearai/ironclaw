//! Skeleton profile for the custom-build-tools lane.

use async_trait::async_trait;

use super::{LfdProfile, ProfileError};
use crate::case::Case;
use crate::reborn_support::builder::RebornIntegrationHarness;

pub const NAME: &str = "custom_build_tools";

pub struct CustomBuildTools;

#[async_trait]
impl LfdProfile for CustomBuildTools {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, _case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        Err(ProfileError::Unsupported(
            "custom_build_tools profile skeleton is not wired yet".to_string(),
        ))
    }
}
