use std::{env, ffi::OsString};

use crate::{REBORN_PROFILE_ENV, RebornConfigError, RebornHome, RebornProfile};

/// Fully resolved boot configuration for the standalone Reborn binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornBootConfig {
    home: RebornHome,
    profile: RebornProfile,
}

impl RebornBootConfig {
    pub fn new(home: RebornHome, profile: RebornProfile) -> Self {
        Self { home, profile }
    }

    pub fn resolve_from_env() -> Result<Self, RebornConfigError> {
        let home = RebornHome::resolve_from_env()?;
        let profile = RebornProfile::from_env_value(env::var_os(REBORN_PROFILE_ENV))?;
        Ok(Self { home, profile })
    }

    pub fn resolve_from_env_parts(
        reborn_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        profile: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        let home = RebornHome::resolve_from_env_parts(reborn_home, home, userprofile)?;
        let profile = RebornProfile::from_env_value(profile)?;
        Ok(Self { home, profile })
    }

    pub fn home(&self) -> &RebornHome {
        &self.home
    }

    pub fn profile(&self) -> RebornProfile {
        self.profile
    }

    pub fn into_parts(self) -> (RebornHome, RebornProfile) {
        (self.home, self.profile)
    }
}
