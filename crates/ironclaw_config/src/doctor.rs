use std::path::Path;

use crate::{IronClawBootConfig, IronClawHome, IronClawProfile};

/// Side-effect-free doctor snapshot for the standalone IronClaw binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawDoctorReport {
    home: IronClawHome,
    profile: IronClawProfile,
}

impl IronClawDoctorReport {
    pub fn from_config(config: IronClawBootConfig) -> Self {
        let (home, profile) = config.into_parts();
        Self { home, profile }
    }

    pub fn home_path(&self) -> &Path {
        self.home.path()
    }

    pub fn home_source_label(&self) -> &'static str {
        self.home.source_label()
    }

    pub fn profile(&self) -> IronClawProfile {
        self.profile
    }

    pub fn v1_state(&self) -> &'static str {
        "not-used"
    }
}
