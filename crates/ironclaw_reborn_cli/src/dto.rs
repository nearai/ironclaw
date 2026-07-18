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
    /// CLI-token `/login?token=` bootstrap link, present only when a valid
    /// `webui-token` file exists under `reborn_home`. `None` without the
    /// `webui-v2-beta` feature.
    ///
    /// `skip_serializing`: carries a live bearer token in the query string;
    /// `status --json` is diagnostic data pasted into issues/logs and must
    /// never leak it. The text renderer reads this field directly, not
    /// through serde, so the terminal `login_link:` line is unaffected.
    #[serde(skip_serializing)]
    pub login_link: Option<String>,
    /// `Some` when `serve` will authenticate off an active env var rather
    /// than the token file — mutually exclusive with `login_link`. Carries
    /// no secret, so not `skip_serializing`.
    pub login_note: Option<String>,
    /// Whether the OS-managed service is actually running, queried live
    /// (not inferred from file presence). `Unknown` on detection
    /// error/unsupported platform; `status` must never fail over this.
    pub service: ServiceStateDto,
}

/// Live OS-service lifecycle state. Mirrors `commands::service::ServiceState`,
/// redefined here rather than deriving `Serialize` on that type directly
/// because it's gated behind `webui-v2-beta` and this DTO must exist (with
/// an `Unknown` fallback) on every build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceStateDto {
    Running,
    Stopped,
    NotInstalled,
    /// Detection failed, or `webui-v2-beta` isn't compiled in. Distinct
    /// from `NotInstalled`: "we don't know" vs. "we know it isn't installed".
    Unknown,
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
            Self::Float(v) if v.fract() == 0.0 => write!(f, "{v:.1}"),
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
