//! Slice-C kernel vocabulary — loop-derived result metadata, gate resume
//! identity, and preserved originating loop refs (arch-simplification §3/§5.3
//! **Stage 1**).
//!
//! These types make [`Resolution`](crate::Resolution) a **non-lossy** carrier for
//! every `CapabilityOutcome` case, so a later stage can delete that overloaded
//! enum (§5.3). Today the `CapabilityOutcome` → `Resolution` mapping drops five
//! classes of field for want of a host_api home (the old "G1/G4 dropped"
//! comments). This module gives each a home:
//!
//! - [`FailureKind`] — the recovery classification on a recoverable failure
//!   (was `CapabilityFailure::error_kind`); it drives retry-vs-terminal.
//! - [`ResultProgress`] / [`TerminateHint`] / [`OutputDigest`] — the loop-derived
//!   completion signals (was `CapabilityResultMessage::{progress, terminate_hint,
//!   output_digest}`, the "G4" fields).
//! - [`ResumeToken`] — the opaque gate-resume identity (was the `resume_token`
//!   inside `approval_resume` / `auth_resume`); the loop echoes it back to resume
//!   a gate. Only the *token* crosses — the raw input/estimate replay payload it
//!   was bundled with stays host-side (charter: no raw input in vocabulary).
//! - [`LoopRef`] — the preserved *originating* loop ref (`result:*` / `gate:*` /
//!   `process:*`), so the loop/evidence layer can still reach state it keyed under
//!   its own ref after the kernel handle (a fresh uuid) is minted.
//!
//! ## Charter
//!
//! Every type here is **plain redacted vocabulary** (host_api charter): a bounded
//! enum, a fixed-width hash value, or a bounded validated safe identifier. None
//! carries a secret, a raw `HostPath`, a backend error string, or a runtime
//! handle. A [`LoopRef`] is a bounded correlation identifier with path delimiters
//! and control characters refused at construction — not free text, and distinct
//! from the kernel record refs ([`GateRef`](crate::GateRef) et al.), which stay
//! opaque uuids precisely so a caller cannot compose one from a string.

use serde::{Deserialize, Serialize};

use crate::HostApiError;

/// Stable digest over a capability's normalized output content — the host_api
/// mirror of `ironclaw_turns`' `ContentDigest` (a Blake3 keyed hash truncated to
/// 8 little-endian bytes). Pure metadata: a fixed-width hash value, never the
/// content itself, so it is safe on the sanitized boundary. Lets progress
/// detection compare outputs without retaining raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutputDigest(u64);

impl OutputDigest {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

/// Typed signal describing whether a completed capability advanced the loop's
/// evidence/state — the host_api mirror of `ironclaw_turns`' `CapabilityProgress`.
/// Lets the loop distinguish a deterministic no-change result from a productive
/// call without inferring progress from prose or token counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultProgress {
    /// Older hosts, or hosts that cannot classify progress yet.
    #[default]
    Unknown,
    /// Produced new evidence or changed host/runtime state. `complete` is an
    /// accepted alias for wire compatibility with the loop enum.
    #[serde(alias = "complete")]
    MadeProgress,
    /// Ran successfully but observed the same state/evidence as before.
    NoChange,
    /// Reached a deterministic non-suspending blocker.
    Blocked,
}

impl ResultProgress {
    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            ResultProgress::Unknown => "unknown",
            ResultProgress::MadeProgress => "made_progress",
            ResultProgress::NoChange => "no_change",
            ResultProgress::Blocked => "blocked",
        }
    }
}

/// Host hint that a completed capability result should end the loop naturally
/// after the current batch — the host_api mirror of the loop's `terminate_hint`
/// bool, modeled as an enum so the two states are named rather than magic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminateHint {
    /// The loop should continue after this result (the default).
    #[default]
    Continue,
    /// The loop should end naturally after the current batch.
    TerminateAfterBatch,
}

impl TerminateHint {
    /// Build from the loop's boolean `terminate_hint`.
    pub fn from_bool(terminate: bool) -> Self {
        if terminate {
            Self::TerminateAfterBatch
        } else {
            Self::Continue
        }
    }

    /// Whether the loop should end after the current batch.
    pub fn should_terminate(&self) -> bool {
        matches!(self, TerminateHint::TerminateAfterBatch)
    }
}

/// The recovery classification of a recoverable failure — the host_api mirror of
/// `ironclaw_turns`' `CapabilityFailureKind`. This is the class that drives
/// retry-vs-terminal handling; it is a bounded *taxonomy* (`network`, `backend`,
/// `authorization`, …), never a raw backend error string — the raw cause stays
/// host-side. An open `Unknown` escape hatch keeps a newer producer's unrecognized
/// tag representable (forward compatibility), mirroring the loop enum.
///
/// Deliberately NOT `#[non_exhaustive]`: the `Unknown` variant is the open-set
/// escape hatch, and the manual `as_str`/`from_tag` route every value through it,
/// so downstream classifiers can match exhaustively (a new *named* variant fails
/// to compile until it is deliberately classified).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FailureKind {
    Authorization,
    Backend,
    Cancelled,
    Dispatcher,
    GateDeclined,
    InvalidInput,
    InvalidOutput,
    MissingRuntime,
    Network,
    OperationFailed,
    OutputTooLarge,
    PolicyDenied,
    Process,
    Resource,
    Transient,
    Unavailable,
    Internal,
    Permanent,
    /// A tag outside the closed set above (forward compatibility). Bounded and
    /// validated at construction; never a raw error string.
    Unknown(FailureKindValue),
}

/// A validated, bounded tag for [`FailureKind::Unknown`]. Safe-identifier
/// charset, so it can never carry a raw payload/path/secret.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FailureKindValue(String);

impl FailureKindValue {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_safe_tag("failure_kind", &value, 128)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FailureKind {
    /// Construct an open-set [`FailureKind::Unknown`] from a validated tag.
    pub fn unknown(value: impl Into<String>) -> Result<Self, HostApiError> {
        FailureKindValue::new(value).map(Self::Unknown)
    }

    /// The stable wire tag (matches the loop enum's `as_str`, so a value maps
    /// losslessly across the two vocabularies).
    pub fn as_str(&self) -> &str {
        match self {
            FailureKind::Authorization => "authorization",
            FailureKind::Backend => "backend",
            FailureKind::Cancelled => "cancelled",
            FailureKind::Dispatcher => "dispatcher",
            FailureKind::GateDeclined => "gate_declined",
            FailureKind::InvalidInput => "invalid_input",
            FailureKind::InvalidOutput => "invalid_output",
            FailureKind::MissingRuntime => "missing_runtime",
            FailureKind::Network => "network",
            FailureKind::OperationFailed => "operation_failed",
            FailureKind::OutputTooLarge => "output_too_large",
            FailureKind::PolicyDenied => "policy_denied",
            FailureKind::Process => "process",
            FailureKind::Resource => "resource",
            FailureKind::Transient => "transient",
            FailureKind::Unavailable => "unavailable",
            FailureKind::Internal => "internal",
            FailureKind::Permanent => "permanent",
            FailureKind::Unknown(value) => value.as_str(),
        }
    }

    /// Reconstruct a `FailureKind` from a wire tag. A known tag yields its named
    /// variant; anything else buckets into a validated [`FailureKind::Unknown`].
    /// Total: an unvalidatable tag (never produced by the loop's own validator)
    /// falls back to [`FailureKind::Internal`].
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "authorization" => Self::Authorization,
            "backend" => Self::Backend,
            "cancelled" => Self::Cancelled,
            "dispatcher" => Self::Dispatcher,
            "gate_declined" => Self::GateDeclined,
            "invalid_input" => Self::InvalidInput,
            "invalid_output" => Self::InvalidOutput,
            "missing_runtime" => Self::MissingRuntime,
            "network" => Self::Network,
            "operation_failed" => Self::OperationFailed,
            "output_too_large" => Self::OutputTooLarge,
            "policy_denied" => Self::PolicyDenied,
            "process" => Self::Process,
            "resource" => Self::Resource,
            "transient" => Self::Transient,
            "unavailable" => Self::Unavailable,
            "internal" => Self::Internal,
            "permanent" => Self::Permanent,
            other => Self::unknown(other).unwrap_or(Self::Internal),
        }
    }
}

impl std::fmt::Display for FailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for FailureKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FailureKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_tag(&value))
    }
}

/// An opaque, redacted gate-resume identity — the host_api mirror of the loop's
/// `CapabilityResumeToken`. Produced when a gate is raised and echoed back by the
/// loop to resume it; the host reconstitutes the original execution context (input
/// replay, estimate, prior-approval lease) from its own storage keyed by this
/// token. Only the token crosses the boundary: bounded and control-free, it
/// carries identity, never the raw input/estimate it was bundled with.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResumeToken(String);

impl ResumeToken {
    /// Maximum length in bytes — matches the loop's `CapabilityResumeToken` bound,
    /// so any loop-minted token is representable losslessly.
    pub const MAX_BYTES: usize = 128;

    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        if value.is_empty() {
            return Err(HostApiError::invalid_id(
                "resume_token",
                value,
                "must not be empty",
            ));
        }
        if value.len() > Self::MAX_BYTES {
            return Err(HostApiError::invalid_id(
                "resume_token",
                value,
                format!("must be at most {} bytes", Self::MAX_BYTES),
            ));
        }
        if value.chars().any(|c| c == '\0' || c.is_control()) {
            return Err(HostApiError::invalid_id(
                "resume_token",
                "<redacted>",
                "must not contain NUL/control characters",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ResumeToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ResumeToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ResumeToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// The preserved *originating* loop ref (`result:*` / `gate:*` / `process:*`) a
/// kernel handle was minted for. The kernel record refs ([`GateRef`](crate::GateRef),
/// [`ResultRef`](crate::ResultRef), [`ProcessRef`](crate::ProcessRef)) are opaque
/// uuids by design, so they cannot carry the loop's own ref identity; without
/// this, state the loop keyed under its ref (e.g. output staged by the result
/// writer) becomes unreachable once the handle is minted. `LoopRef` carries that
/// originating ref alongside the kernel handle so it stays reachable through the
/// migration window.
///
/// It is a **bounded, redacted correlation identifier**, not free text: control
/// characters and path delimiters (`/`, `\`, `..`) are refused at construction, so
/// it can hold no raw path — and it is a *distinct* type from the kernel refs, so
/// it can never be mistaken for one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopRef(String);

impl LoopRef {
    /// Maximum length in bytes — matches the widest loop ref bound
    /// (`LoopProcessRef`, 256), so any loop ref is representable losslessly.
    pub const MAX_BYTES: usize = 256;

    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        if value.is_empty() {
            return Err(HostApiError::invalid_id(
                "loop_ref",
                value,
                "must not be empty",
            ));
        }
        if value.len() > Self::MAX_BYTES {
            return Err(HostApiError::invalid_id(
                "loop_ref",
                value,
                format!("must be at most {} bytes", Self::MAX_BYTES),
            ));
        }
        if value.chars().any(|c| c == '\0' || c.is_control()) {
            return Err(HostApiError::invalid_id(
                "loop_ref",
                "<redacted>",
                "must not contain NUL/control characters",
            ));
        }
        if value.contains('/') || value.contains('\\') || value.contains("..") {
            return Err(HostApiError::invalid_id(
                "loop_ref",
                value,
                "must not contain path separators or parent-directory markers",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LoopRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for LoopRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LoopRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Shared validator for bounded safe-identifier tags (the `FailureKind::Unknown`
/// tag): non-empty, bounded, and restricted to a safe identifier charset so no
/// raw payload/path/secret can ride along.
fn validate_safe_tag(kind: &'static str, value: &str, max_bytes: usize) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_id(kind, value, "must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            format!("must be at most {max_bytes} bytes"),
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "must contain only ASCII letters, digits, _, -, ., or :",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_digest_is_transparent_on_the_wire() {
        let digest = OutputDigest::new(0x0102_0304_0506_0708);
        let json = serde_json::to_value(digest).unwrap();
        assert_eq!(json, serde_json::json!(0x0102_0304_0506_0708u64));
        assert_eq!(
            serde_json::from_value::<OutputDigest>(json).unwrap().value(),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn result_progress_snake_case_and_complete_alias() {
        for (progress, tag) in [
            (ResultProgress::Unknown, "unknown"),
            (ResultProgress::MadeProgress, "made_progress"),
            (ResultProgress::NoChange, "no_change"),
            (ResultProgress::Blocked, "blocked"),
        ] {
            assert_eq!(
                serde_json::to_value(progress).unwrap(),
                serde_json::Value::String(tag.to_string())
            );
            assert_eq!(progress.kind(), tag);
        }
        assert_eq!(ResultProgress::default(), ResultProgress::Unknown);
        // The loop enum's `complete` alias still decodes (wire compatibility).
        assert_eq!(
            serde_json::from_value::<ResultProgress>(serde_json::json!("complete")).unwrap(),
            ResultProgress::MadeProgress
        );
    }

    #[test]
    fn terminate_hint_bool_bridge_and_wire() {
        assert_eq!(TerminateHint::from_bool(true), TerminateHint::TerminateAfterBatch);
        assert_eq!(TerminateHint::from_bool(false), TerminateHint::Continue);
        assert!(TerminateHint::TerminateAfterBatch.should_terminate());
        assert!(!TerminateHint::Continue.should_terminate());
        assert_eq!(TerminateHint::default(), TerminateHint::Continue);
        assert_eq!(
            serde_json::to_value(TerminateHint::TerminateAfterBatch).unwrap(),
            serde_json::Value::String("terminate_after_batch".to_string())
        );
    }

    #[test]
    fn failure_kind_tags_round_trip_for_every_named_variant() {
        let named = [
            FailureKind::Authorization,
            FailureKind::Backend,
            FailureKind::Cancelled,
            FailureKind::Dispatcher,
            FailureKind::GateDeclined,
            FailureKind::InvalidInput,
            FailureKind::InvalidOutput,
            FailureKind::MissingRuntime,
            FailureKind::Network,
            FailureKind::OperationFailed,
            FailureKind::OutputTooLarge,
            FailureKind::PolicyDenied,
            FailureKind::Process,
            FailureKind::Resource,
            FailureKind::Transient,
            FailureKind::Unavailable,
            FailureKind::Internal,
            FailureKind::Permanent,
        ];
        for kind in named {
            let tag = kind.as_str();
            assert_eq!(FailureKind::from_tag(tag), kind, "from_tag round-trip: {tag}");
            let wire = serde_json::to_value(&kind).unwrap();
            assert_eq!(wire, serde_json::Value::String(tag.to_string()));
            assert_eq!(serde_json::from_value::<FailureKind>(wire).unwrap(), kind);
        }
    }

    #[test]
    fn failure_kind_unknown_tag_is_preserved_not_dropped() {
        // A newer producer's tag survives round-trip through Unknown.
        let kind = FailureKind::from_tag("quota_exceeded");
        assert_eq!(kind, FailureKind::unknown("quota_exceeded").unwrap());
        assert_eq!(kind.as_str(), "quota_exceeded");
        let back: FailureKind =
            serde_json::from_value(serde_json::to_value(&kind).unwrap()).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn resume_token_bounded_control_free_and_round_trips() {
        let token = ResumeToken::new("resume-abc.123").unwrap();
        assert_eq!(token.as_str(), "resume-abc.123");
        let back: ResumeToken =
            serde_json::from_value(serde_json::to_value(&token).unwrap()).unwrap();
        assert_eq!(back, token);
        assert!(ResumeToken::new("").is_err());
        assert!(ResumeToken::new("has\nnewline").is_err());
        assert!(ResumeToken::new("x".repeat(ResumeToken::MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn loop_ref_preserves_the_loop_charset_but_refuses_paths() {
        for value in ["result:child-1", "gate:approval-req_9", "process:pid-1"] {
            let loop_ref = LoopRef::new(value).unwrap();
            assert_eq!(loop_ref.as_str(), value);
            let back: LoopRef =
                serde_json::from_value(serde_json::to_value(&loop_ref).unwrap()).unwrap();
            assert_eq!(back, loop_ref);
        }
        // Charter: no raw paths / control chars.
        assert!(LoopRef::new("result:/etc/passwd").is_err());
        assert!(LoopRef::new("result:..\\escape").is_err());
        assert!(LoopRef::new("result:a\0b").is_err());
        assert!(LoopRef::new("").is_err());
    }
}
