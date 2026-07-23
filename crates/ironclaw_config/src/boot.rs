use std::{env, ffi::OsString};

use crate::{
    IRONCLAW_PROFILE_ENV, IronClawConfigError, IronClawHome, IronClawProfile,
    LEGACY_IRONCLAW_PROFILE_ENV,
};

/// Fully resolved boot configuration for the standalone IronClaw binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawBootConfig {
    home: IronClawHome,
    profile: IronClawProfile,
}

impl IronClawBootConfig {
    pub fn new(home: IronClawHome, profile: IronClawProfile) -> Self {
        Self { home, profile }
    }

    pub fn resolve_from_env() -> Result<Self, IronClawConfigError> {
        let home = IronClawHome::resolve_from_env()?;
        let profile = IronClawProfile::from_env_values(
            env::var_os(IRONCLAW_PROFILE_ENV),
            env::var_os(LEGACY_IRONCLAW_PROFILE_ENV),
        )?;
        Ok(Self { home, profile })
    }

    pub fn resolve_from_env_parts(
        ironclaw_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        profile: Option<OsString>,
    ) -> Result<Self, IronClawConfigError> {
        let home = IronClawHome::resolve_from_env_parts(ironclaw_home, home, userprofile)?;
        let profile = IronClawProfile::from_env_value(profile)?;
        Ok(Self { home, profile })
    }

    pub fn resolve_from_env_parts_with_legacy(
        ironclaw_home: Option<OsString>,
        legacy_ironclaw_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        profile: Option<OsString>,
        legacy_profile: Option<OsString>,
    ) -> Result<Self, IronClawConfigError> {
        let home = IronClawHome::resolve_from_env_parts_with_legacy(
            ironclaw_home,
            legacy_ironclaw_home,
            home,
            userprofile,
        )?;
        let profile = IronClawProfile::from_env_values(profile, legacy_profile)?;
        Ok(Self { home, profile })
    }

    pub fn home(&self) -> &IronClawHome {
        &self.home
    }

    pub fn profile(&self) -> IronClawProfile {
        self.profile
    }

    pub fn into_parts(self) -> (IronClawHome, IronClawProfile) {
        (self.home, self.profile)
    }
}
