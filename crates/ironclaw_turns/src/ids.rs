use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnId(Uuid);

impl TurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TurnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnRunId(Uuid);

impl TurnRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TurnRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for TurnRunId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityActivityId(Uuid);

impl CapabilityActivityId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for CapabilityActivityId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CapabilityActivityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnCheckpointId(Uuid);

impl TurnCheckpointId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnCheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnLeaseToken(Uuid);

impl TurnLeaseToken {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TurnLeaseToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnRunnerId(Uuid);

impl TurnRunnerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TurnRunnerId {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! bounded_ref {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_ref($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! loop_ref {
    ($name:ident, $kind:literal, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_loop_ref($kind, $prefix, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_ref!(AcceptedMessageRef, "accepted_message_ref");
bounded_ref!(SourceBindingRef, "source_binding_ref");
bounded_ref!(ReplyTargetBindingRef, "reply_target_binding_ref");
bounded_ref!(GateRef, "gate_ref");
bounded_ref!(IdempotencyKey, "idempotency_key");
bounded_ref!(RunProfileRequest, "run_profile_request");
bounded_ref!(RunProfileId, "run_profile_id");
loop_ref!(LoopExitId, "loop_exit_id", "exit:");
loop_ref!(LoopMessageRef, "loop_message_ref", "msg:");
loop_ref!(LoopResultRef, "loop_result_ref", "result:");
loop_ref!(LoopGateRef, "loop_gate_ref", "gate:");
loop_ref!(LoopUsageSummaryRef, "loop_usage_summary_ref", "usage:");
loop_ref!(LoopDiagnosticRef, "loop_diagnostic_ref", "diag:");

// GateRef and LoopGateRef carry the same validated `gate:<id>` string by
// design (the model-visible `LoopGateRef` is constructed from the host-side
// `GateRef`). Cross-type equality enforces that invariant at the type
// system instead of via ad-hoc `.as_str()` string compares in callers.
impl PartialEq<LoopGateRef> for GateRef {
    fn eq(&self, other: &LoopGateRef) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<GateRef> for LoopGateRef {
    fn eq(&self, other: &GateRef) -> bool {
        self.as_str() == other.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::{GateRef, LoopGateRef, TurnRunId};

    #[test]
    fn gate_ref_eq_loop_gate_ref_matches_exact_gate_string() {
        let gate_ref = GateRef::new("gate:subagent-test").unwrap();
        let loop_gate_ref = LoopGateRef::new("gate:subagent-test").unwrap();
        let other_loop_gate_ref = LoopGateRef::new("gate:subagent-other").unwrap();
        let other_gate_ref = GateRef::new("gate:subagent-other").unwrap();

        assert_eq!(gate_ref, loop_gate_ref);
        assert_eq!(loop_gate_ref, gate_ref);
        assert_ne!(gate_ref, other_loop_gate_ref);
        assert_ne!(loop_gate_ref, other_gate_ref);
    }

    #[test]
    fn turn_run_id_parse_round_trips_display_string() {
        let run_id = TurnRunId::new();
        let parsed = TurnRunId::parse(&run_id.to_string()).expect("parse run id");
        assert_eq!(parsed, run_id);
        assert!("not-a-uuid".parse::<TurnRunId>().is_err());
    }
}

impl RunProfileId {
    pub fn default_profile() -> Self {
        Self::from_trusted_static("default")
    }

    pub fn interactive_default() -> Self {
        Self::from_trusted_static("interactive_default")
    }

    pub fn long_running_mission() -> Self {
        Self::from_trusted_static("long_running_mission")
    }

    pub fn is_interactive_default(&self) -> bool {
        self == &Self::interactive_default()
    }

    pub(crate) fn from_trusted_static(value: &'static str) -> Self {
        debug_assert!(validate_ref("run_profile_id", value).is_ok());
        Self(value.to_string())
    }

    pub fn from_request(request: &RunProfileRequest) -> Self {
        Self(request.as_str().to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunProfileVersion(u64);

impl RunProfileVersion {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

fn validate_ref(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 256 {
        return Err(format!("{kind} must be at most 256 bytes"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} must not contain control characters"));
    }
    Ok(())
}

fn validate_loop_ref(kind: &'static str, prefix: &'static str, value: &str) -> Result<(), String> {
    validate_ref(kind, value)?;
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(format!("{kind} must start with {prefix}"));
    };
    if suffix.is_empty() {
        return Err(format!("{kind} must include an opaque id after {prefix}"));
    }
    if !suffix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(format!(
            "{kind} opaque id must contain only ASCII letters, digits, _, -, or ."
        ));
    }
    Ok(())
}
