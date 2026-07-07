//! Skeleton profile for the user-voice-model lane.

use async_trait::async_trait;

use super::{LfdProfile, ProfileError};
use crate::case::Case;
use crate::reborn_support::builder::RebornIntegrationHarness;

pub const NAME: &str = "user_voice_model";

pub struct UserVoiceModel;

#[async_trait]
impl LfdProfile for UserVoiceModel {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, _case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        Err(ProfileError::Unsupported(
            "user_voice_model profile skeleton is not wired yet".to_string(),
        ))
    }
}
