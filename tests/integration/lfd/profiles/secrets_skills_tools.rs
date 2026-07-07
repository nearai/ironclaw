//! Skeleton profile for the skills/tools secret-mediation lane.

use async_trait::async_trait;

use super::{LfdProfile, ProfileError};
use crate::case::Case;
use crate::reborn_support::builder::RebornIntegrationHarness;

pub const NAME: &str = "secrets_skills_tools";

pub struct SecretsSkillsTools;

#[async_trait]
impl LfdProfile for SecretsSkillsTools {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, _case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        Err(ProfileError::Unsupported(
            "secrets_skills_tools profile skeleton is not wired yet".to_string(),
        ))
    }
}
