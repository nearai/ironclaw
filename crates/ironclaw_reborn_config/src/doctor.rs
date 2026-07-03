use std::path::Path;

use crate::{RebornBootConfig, RebornHome, RebornProfile};

/// Side-effect-free doctor snapshot for the standalone Reborn binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornDoctorReport {
    home: RebornHome,
    profile: RebornProfile,
}

impl RebornDoctorReport {
    pub fn from_config(config: RebornBootConfig) -> Self {
        let (home, profile) = config.into_parts();
        Self { home, profile }
    }

    pub fn home_path(&self) -> &Path {
        self.home.path()
    }

    pub fn home_source_label(&self) -> &'static str {
        self.home.source_label()
    }

    pub fn profile(&self) -> RebornProfile {
        self.profile
    }

    pub fn v1_state(&self) -> &'static str {
        "not-used"
    }
}
