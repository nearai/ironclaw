use std::path::PathBuf;

use serde::Serialize;

// ─── Status ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub(crate) struct StatusDto {
    pub version: String,
    pub reborn_home: PathBuf,
    pub home_source: &'static str,
    pub profile: String,
    pub config_file: FilePresence,
    pub providers_file: FilePresence,
    pub model_slots: Vec<String>,
    pub drivers: DriversSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FilePresence {
    pub path: PathBuf,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DriversSnapshot {
    pub text_only: ComponentStatus,
    pub planned: ComponentStatus,
    pub subagent_planned: ComponentStatus,
    pub planned_default_profile: ComponentStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ComponentStatus {
    Initialized,
    Failed { reason: String },
}

// ─── Doctor ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DoctorDto {
    pub checks: Vec<DoctorCheck>,
    pub summary: DoctorSummary,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DoctorCheck {
    pub name: String,
    pub category: CheckCategory,
    pub outcome: CheckOutcome,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckCategory {
    Core,
    Drivers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckOutcome {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DoctorSummary {
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
}

// ─── Config ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConfigListDto {
    pub config_file: PathBuf,
    pub entries: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConfigEntry {
    pub key: String,
    pub value: Option<ConfigValue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum ConfigValue {
    String(String),
    Bool(bool),
    Integer(u64),
    Float(f64),
    List(Vec<String>),
}

impl std::fmt::Display for ConfigValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(s) => write!(f, "{s}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Integer(n) => write!(f, "{n}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::List(items) => {
                write!(f, "[{}]", items.join(", "))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConfigGetDto {
    pub key: String,
    pub value: Option<ConfigValue>,
}
