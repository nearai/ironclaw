use std::{env, ffi::OsString};

use crate::{
    IRONCLAW_PROFILE_ENV, REBORN_PROFILE_ENV, RebornConfigError, RebornHome, RebornProfile,
};

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
        let profile = RebornProfile::from_env_values(
            env::var_os(IRONCLAW_PROFILE_ENV),
            env::var_os(REBORN_PROFILE_ENV),
        )?;
        Ok(Self { home, profile })
    }

    pub fn resolve_from_env_parts(
        ironclaw_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        profile: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        let home = RebornHome::resolve_from_env_parts(ironclaw_home, home, userprofile)?;
        let profile = RebornProfile::from_env_value(profile)?;
        Ok(Self { home, profile })
    }

    pub fn resolve_from_env_parts_with_legacy(
        ironclaw_home: Option<OsString>,
        legacy_reborn_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        profile: Option<OsString>,
        legacy_profile: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        let home = RebornHome::resolve_from_env_parts_with_legacy(
            ironclaw_home,
            legacy_reborn_home,
            home,
            userprofile,
        )?;
        let profile = RebornProfile::from_env_values(profile, legacy_profile)?;
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
