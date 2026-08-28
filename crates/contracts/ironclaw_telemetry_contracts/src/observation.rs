//! Bounded, provider-neutral facts accepted by the telemetry collector.

use std::fmt;

use chrono::{DateTime, Utc};
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_host_api::{ids::UserId, turn::SanitizedFailure};
use serde::{Deserialize, Serialize};

/// Maximum UTF-8 byte length for identifiers introduced by telemetry.
pub const MAX_TELEMETRY_IDENTIFIER_BYTES: usize = 128;

/// Maximum value representable by a signed BIGINT-backed durable counter.
pub const MAX_DURABLE_COUNTER: u64 = i64::MAX as u64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoundedIdentifierError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} must be at most {max} UTF-8 bytes (got {actual})")]
    TooLong {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("{field} must not contain control characters")]
    ControlCharacters { field: &'static str },
}

macro_rules! bounded_identifier {
    ($name:ident, $field:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            fn validate(value: &str) -> Result<(), BoundedIdentifierError> {
                if value.is_empty() {
                    return Err(BoundedIdentifierError::Empty { field: $field });
                }
                if value.len() > MAX_TELEMETRY_IDENTIFIER_BYTES {
                    return Err(BoundedIdentifierError::TooLong {
                        field: $field,
                        max: MAX_TELEMETRY_IDENTIFIER_BYTES,
                        actual: value.len(),
                    });
                }
                if value.chars().any(char::is_control) {
                    return Err(BoundedIdentifierError::ControlCharacters { field: $field });
                }
                Ok(())
            }

            pub fn new(value: impl Into<String>) -> Result<Self, BoundedIdentifierError> {
                let value = value.into();
                Self::validate(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = BoundedIdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::validate(&value)?;
                Ok(Self(value))
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

bounded_identifier!(ProviderId, "provider_id");
bounded_identifier!(EffectiveModelId, "effective_model_id");
bounded_identifier!(AutomationId, "automation_id");
bounded_identifier!(LifecycleEventId, "event_id");
bounded_identifier!(SubjectId, "subject_id");
bounded_identifier!(CollectorInstanceId, "collector_instance_id");

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct FailureCategory(String);

impl FailureCategory {
    fn validate(value: &str) -> Result<(), BoundedIdentifierError> {
        if value.is_empty() {
            return Err(BoundedIdentifierError::Empty {
                field: "failure_category",
            });
        }
        if value.len() > 256 {
            return Err(BoundedIdentifierError::TooLong {
                field: "failure_category",
                max: 256,
                actual: value.len(),
            });
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(BoundedIdentifierError::ControlCharacters {
                field: "failure_category",
            });
        }
        Ok(())
    }

    pub fn new(value: impl Into<String>) -> Result<Self, BoundedIdentifierError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn from_sanitized_failure(
        failure: &SanitizedFailure,
    ) -> Result<Self, BoundedIdentifierError> {
        Self::new(failure.category())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for FailureCategory {
    type Error = BoundedIdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for FailureCategory {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for FailureCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<FailureCategory> for String {
    fn from(value: FailureCategory) -> Self {
        value.0
    }
}

impl TryFrom<SanitizedFailure> for FailureCategory {
    type Error = BoundedIdentifierError;

    fn try_from(failure: SanitizedFailure) -> Result<Self, Self::Error> {
        Self::from_sanitized_failure(&failure)
    }
}

/// Timestamp shared by observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationContext {
    occurred_at: DateTime<Utc>,
}

impl ObservationContext {
    pub const fn new(occurred_at: DateTime<Utc>) -> Self {
        Self { occurred_at }
    }

    pub const fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObservationError {
    #[error(transparent)]
    InvalidIdentifier(#[from] BoundedIdentifierError),
    #[error("failed and recovery-required runs require a sanitized failure category")]
    FailureRequired,
    #[error("only failed and recovery-required runs may carry a failure category")]
    UnexpectedFailure,
    #[error("{field} value {value} exceeds signed BIGINT range")]
    CounterOutOfRange { field: &'static str, value: u64 },
}

/// The origin vocabulary intentionally contains no transport or vendor names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OriginKind {
    Human,
    ParentAgent,
    System,
    Automation,
    Other,
}

/// Terminal outcomes currently represented by the canonical run contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RunOutcome {
    Completed,
    Failed,
    Cancelled,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AutomationKind {
    Cron,
    Once,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LifecycleEventKind {
    MemberAdded,
    MemberRemoved,
    RoutineCreated,
    RoutineEnabled,
    RoutineDisabled,
    RoutineDeleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LifecycleSubjectKind {
    Tenant,
    User,
    Routine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
}

impl ModelUsage {
    pub const fn new(
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
        }
    }

    pub const fn input_tokens(self) -> u64 {
        self.input_tokens
    }

    pub const fn output_tokens(self) -> u64 {
        self.output_tokens
    }

    pub const fn cache_read_input_tokens(self) -> u64 {
        self.cache_read_input_tokens
    }

    pub const fn cache_creation_input_tokens(self) -> u64 {
        self.cache_creation_input_tokens
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSettledObservation {
    context: ObservationContext,
    origin: OriginKind,
    outcome: RunOutcome,
    duration_ms: u64,
    reported_tool_call_count: Option<u64>,
    failure: Option<FailureCategory>,
}

impl RunSettledObservation {
    pub fn new(
        context: ObservationContext,
        origin: OriginKind,
        outcome: RunOutcome,
        duration_ms: u64,
        reported_tool_call_count: Option<u64>,
        failure: Option<SanitizedFailure>,
    ) -> Result<Self, ObservationError> {
        if duration_ms > MAX_DURABLE_COUNTER {
            return Err(ObservationError::CounterOutOfRange {
                field: "duration_ms",
                value: duration_ms,
            });
        }
        if let Some(count) = reported_tool_call_count
            && count > MAX_DURABLE_COUNTER
        {
            return Err(ObservationError::CounterOutOfRange {
                field: "reported_tool_call_count",
                value: count,
            });
        }
        let failure_is_required =
            matches!(outcome, RunOutcome::Failed | RunOutcome::RecoveryRequired);
        let failure = failure
            .as_ref()
            .map(FailureCategory::from_sanitized_failure)
            .transpose()?;
        match (failure_is_required, failure.is_some()) {
            (true, false) => Err(ObservationError::FailureRequired),
            (false, true) => Err(ObservationError::UnexpectedFailure),
            _ => Ok(Self {
                context,
                origin,
                outcome,
                duration_ms,
                reported_tool_call_count,
                failure,
            }),
        }
    }

    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.context.occurred_at()
    }

    pub const fn origin(&self) -> OriginKind {
        self.origin
    }

    pub const fn outcome(&self) -> RunOutcome {
        self.outcome
    }

    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub const fn reported_tool_call_count(&self) -> Option<u64> {
        self.reported_tool_call_count
    }

    pub const fn tool_count_reported(&self) -> bool {
        self.reported_tool_call_count.is_some()
    }

    pub const fn runs_with_reported_tool_calls(&self) -> bool {
        matches!(self.reported_tool_call_count, Some(count) if count > 0)
    }

    pub fn failure(&self) -> Option<&FailureCategory> {
        self.failure.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCallCompletedObservation {
    context: ObservationContext,
    provider_id: ProviderId,
    effective_model_id: EffectiveModelId,
    usage: Option<ModelUsage>,
}

impl ModelCallCompletedObservation {
    pub fn new(
        context: ObservationContext,
        provider_id: ProviderId,
        effective_model_id: EffectiveModelId,
        usage: Option<ModelUsage>,
    ) -> Result<Self, ObservationError> {
        if let Some(usage) = usage {
            for (field, value) in [
                ("input_tokens", usage.input_tokens()),
                ("output_tokens", usage.output_tokens()),
                ("cache_read_input_tokens", usage.cache_read_input_tokens()),
                (
                    "cache_creation_input_tokens",
                    usage.cache_creation_input_tokens(),
                ),
            ] {
                if value > MAX_DURABLE_COUNTER {
                    return Err(ObservationError::CounterOutOfRange { field, value });
                }
            }
        }
        Ok(Self {
            context,
            provider_id,
            effective_model_id,
            usage,
        })
    }

    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.context.occurred_at()
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn effective_model_id(&self) -> &EffectiveModelId {
        &self.effective_model_id
    }

    pub const fn inference_count(&self) -> u64 {
        1
    }

    pub const fn usage_reported(&self) -> bool {
        self.usage.is_some()
    }

    pub fn input_tokens(&self) -> u64 {
        match self.usage {
            Some(usage) => usage.input_tokens(),
            None => 0,
        }
    }

    pub fn output_tokens(&self) -> u64 {
        match self.usage {
            Some(usage) => usage.output_tokens(),
            None => 0,
        }
    }

    pub fn cache_read_input_tokens(&self) -> u64 {
        match self.usage {
            Some(usage) => usage.cache_read_input_tokens(),
            None => 0,
        }
    }

    pub fn cache_creation_input_tokens(&self) -> u64 {
        match self.usage {
            Some(usage) => usage.cache_creation_input_tokens(),
            None => 0,
        }
    }

    pub const fn usage(&self) -> Option<ModelUsage> {
        self.usage
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationSettledObservation {
    context: ObservationContext,
    automation_id: AutomationId,
    automation_kind: AutomationKind,
    outcome: RunOutcome,
}

impl AutomationSettledObservation {
    pub fn new(
        context: ObservationContext,
        automation_id: AutomationId,
        automation_kind: AutomationKind,
        outcome: RunOutcome,
    ) -> Result<Self, ObservationError> {
        Ok(Self {
            context,
            automation_id,
            automation_kind,
            outcome,
        })
    }

    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.context.occurred_at()
    }

    pub fn automation_id(&self) -> &AutomationId {
        &self.automation_id
    }

    pub const fn automation_kind(&self) -> AutomationKind {
        self.automation_kind
    }

    pub const fn outcome(&self) -> RunOutcome {
        self.outcome
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleTransitionObservation {
    subject_user_id: Option<UserId>,
    event_id: LifecycleEventId,
    event_kind: LifecycleEventKind,
    subject_kind: LifecycleSubjectKind,
    subject_id: SubjectId,
    occurred_at: DateTime<Utc>,
}

impl LifecycleTransitionObservation {
    pub fn new(
        subject_user_id: Option<UserId>,
        event_id: LifecycleEventId,
        event_kind: LifecycleEventKind,
        subject_kind: LifecycleSubjectKind,
        subject_id: impl Into<String>,
        occurred_at: DateTime<Utc>,
    ) -> Result<Self, ObservationError> {
        let subject_id = SubjectId::new(subject_id)?;
        Ok(Self {
            subject_user_id,
            event_id,
            event_kind,
            subject_kind,
            subject_id,
            occurred_at,
        })
    }

    pub fn subject_user_id(&self) -> Option<&UserId> {
        self.subject_user_id.as_ref()
    }

    pub fn event_id(&self) -> &LifecycleEventId {
        &self.event_id
    }

    pub const fn event_kind(&self) -> LifecycleEventKind {
        self.event_kind
    }

    pub const fn subject_kind(&self) -> LifecycleSubjectKind {
        self.subject_kind
    }

    pub fn subject_id(&self) -> &SubjectId {
        &self.subject_id
    }

    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelemetryObservation {
    RunSettled(RunSettledObservation),
    ModelCallCompleted(ModelCallCompletedObservation),
    AutomationSettled(AutomationSettledObservation),
    LifecycleTransition(LifecycleTransitionObservation),
}

impl TelemetryObservation {
    pub fn occurred_at(&self) -> DateTime<Utc> {
        match self {
            Self::RunSettled(observation) => observation.occurred_at(),
            Self::ModelCallCompleted(observation) => observation.occurred_at(),
            Self::AutomationSettled(observation) => observation.occurred_at(),
            Self::LifecycleTransition(observation) => observation.occurred_at(),
        }
    }
}

/// A trusted resource scope carried with one observation until aggregation.
///
/// The scope is intentionally owned by this envelope. Observation payloads
/// contain event facts only; tenant and usage attribution always come from the
/// trusted scope supplied by the recorder caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedTelemetryObservation {
    scope: ResourceScope,
    observation: TelemetryObservation,
}

impl ScopedTelemetryObservation {
    pub fn new(scope: ResourceScope, observation: TelemetryObservation) -> Self {
        Self { scope, observation }
    }

    pub fn scope(&self) -> &ResourceScope {
        &self.scope
    }

    pub fn observation(&self) -> &TelemetryObservation {
        &self.observation
    }

    pub fn into_parts(self) -> (ResourceScope, TelemetryObservation) {
        (self.scope, self.observation)
    }

    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.observation.occurred_at()
    }
}
