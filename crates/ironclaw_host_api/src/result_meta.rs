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

use crate::{DispatchInputIssueCode, HostApiError, HostRemediation, SafeSummary};

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
        // Borrow when the input allows it: the 18 named variants allocate
        // nothing, and only an `Unknown` tag needs an owned copy.
        let value = std::borrow::Cow::<str>::deserialize(deserializer)?;
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

/// The upper bound on model-visible input issues carried on an
/// [`ModelFailureDiagnostic::InvalidInput`]. Matches the loop's
/// `MODEL_OBSERVATION_INPUT_ISSUES_MAX` storage cap, so the producer can carry
/// every issue the loop retained; the eventual renderer applies its own
/// (smaller) display cap on top. Keeps the diagnostic a *bounded* list.
pub const MAX_MODEL_INPUT_ISSUES: usize = 16;

/// The bounded list of model-visible input issues on an
/// [`ModelFailureDiagnostic::InvalidInput`] — at most
/// [`MAX_MODEL_INPUT_ISSUES`], enforced at construction AND on the wire
/// (`#[serde(try_from = "Vec<ModelInputIssue>")]`), so no producer, persisted
/// row, or direct construction can bypass the documented cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<ModelInputIssue>")]
pub struct ModelInputIssues(Vec<ModelInputIssue>);

impl ModelInputIssues {
    /// Validating constructor: rejects more than [`MAX_MODEL_INPUT_ISSUES`].
    pub fn new(issues: Vec<ModelInputIssue>) -> Result<Self, HostApiError> {
        if issues.len() > MAX_MODEL_INPUT_ISSUES {
            return Err(HostApiError::invalid_id(
                "model_input_issues",
                issues.len().to_string(),
                format!("must carry at most {MAX_MODEL_INPUT_ISSUES} issues"),
            ));
        }
        Ok(Self(issues))
    }

    /// Producer convenience: keep the first [`MAX_MODEL_INPUT_ISSUES`] issues,
    /// dropping the tail (a producer with an oversized set truncates rather
    /// than fails — the cap is a rendering bound, not an error condition on
    /// the producing side).
    pub fn truncating(issues: impl IntoIterator<Item = ModelInputIssue>) -> Self {
        Self(issues.into_iter().take(MAX_MODEL_INPUT_ISSUES).collect())
    }

    pub fn as_slice(&self) -> &[ModelInputIssue] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<Vec<ModelInputIssue>> for ModelInputIssues {
    type Error = HostApiError;

    /// Wire revalidation matches construction (types.md): a persisted/relayed
    /// 17-item payload is rejected on deserialize, never trusted.
    fn try_from(issues: Vec<ModelInputIssue>) -> Result<Self, HostApiError> {
        Self::new(issues)
    }
}

impl std::ops::Index<usize> for ModelInputIssues {
    type Output = ModelInputIssue;

    fn index(&self, index: usize) -> &ModelInputIssue {
        &self.0[index]
    }
}

/// A redacted, model-visible schema-validation issue — the host_api mirror of the
/// loop's `CapabilityInputIssue`. It is the sanitized, wire-boundary view: every
/// free-text field is a bounded, redacted [`SafeSummary`] (no raw payload, path,
/// or secret), and the issue `code` is the stable [`DispatchInputIssueCode`]
/// host_api enum (already the canonical code for the loop-side issue). Distinct
/// from the fuller internal [`DispatchInputIssue`](crate::DispatchInputIssue),
/// which carries raw `String` fields on the dispatch port — this is the redacted
/// projection that may cross the sanitized `Resolution` boundary (a
/// security-boundary mirror, `type-placement.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInputIssue {
    /// The redacted input path the issue is about (e.g. `schedule.kind`).
    pub path: SafeSummary,
    /// The stable, structured issue code.
    pub code: DispatchInputIssueCode,
    /// The redacted expected shape/value, when the producer supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<SafeSummary>,
    /// The redacted received shape/value, when the producer supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received: Option<SafeSummary>,
    /// The redacted schema pointer, when the producer supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_path: Option<SafeSummary>,
}

impl ModelInputIssue {
    /// A bare issue carrying only the redacted path and structured code. Use the
    /// `with_*` setters to attach the optional redacted expected/received/schema
    /// fields (default-backed builder).
    pub fn new(path: SafeSummary, code: DispatchInputIssueCode) -> Self {
        Self {
            path,
            code,
            expected: None,
            received: None,
            schema_path: None,
        }
    }

    /// Attach the redacted expected shape/value.
    pub fn with_expected(mut self, expected: SafeSummary) -> Self {
        self.expected = Some(expected);
        self
    }

    /// Attach the redacted received shape/value.
    pub fn with_received(mut self, received: SafeSummary) -> Self {
        self.received = Some(received);
        self
    }

    /// Attach the redacted schema pointer.
    pub fn with_schema_path(mut self, schema_path: SafeSummary) -> Self {
        self.schema_path = Some(schema_path);
        self
    }
}

/// The model-visible structured diagnostic a recoverable failure carries so the
/// model can correct a bad tool call — the redacted host_api mirror of the loop's
/// `CapabilityFailureDetail`. Rides [`ToolVerdict::RecoverableFailure`](crate::ToolVerdict)
/// so the loop can render the correction hint without reading host storage (§5.3
/// flip prep).
///
/// Both arms are plain redacted vocabulary (host_api charter): enums, a
/// [`DispatchInputIssueCode`], and bounded [`SafeSummary`] values — never a raw
/// cause, secret, host path, or backend error string. The free-text `Diagnostic`
/// arm is deliberately stricter than the loop's lenient diagnostic channel (which
/// permits paths): at this sanitized boundary the text must satisfy the full
/// [`SafeSummary`] redaction contract, so a path-shaped diagnostic is redacted
/// rather than carried raw.
///
/// Internally tagged (`kind`) to mirror the loop enum's shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelFailureDiagnostic {
    /// The tool input failed schema validation; carries the bounded, redacted
    /// structured issues the model corrects from.
    InvalidInput { issues: ModelInputIssues },
    /// A bounded, redacted free-text cause.
    Diagnostic { text: SafeSummary },
    /// Host-authored operator remediation — the TRUSTED text channel.
    ///
    /// Separate from [`Self::Diagnostic`] by PROVENANCE: `Diagnostic` holds an
    /// untrusted cause squeezed through the full [`SafeSummary`] contract (so a
    /// URL- or path-shaped value degrades to the placeholder), while this arm
    /// holds a host-authored instruction that must reach the model intact. Only
    /// host code constructs the payload — see [`HostRemediation`].
    HostRemediation { text: HostRemediation },
}

impl ModelFailureDiagnostic {
    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            ModelFailureDiagnostic::InvalidInput { .. } => "invalid_input",
            ModelFailureDiagnostic::Diagnostic { .. } => "diagnostic",
            ModelFailureDiagnostic::HostRemediation { .. } => "host_remediation",
        }
    }

    /// The model-visible free text carried by whichever text arm is present —
    /// the accessor a renderer wants, so a new text arm cannot be silently
    /// dropped by a call site that only knew about `Diagnostic`.
    pub fn model_visible_text(&self) -> Option<&str> {
        // pub-api-exempt: consumed by ironclaw_reborn_composition's host_remediation_contract full-path test
        match self {
            ModelFailureDiagnostic::Diagnostic { text } => Some(text.as_str()),
            ModelFailureDiagnostic::HostRemediation { text } => Some(text.as_str()),
            ModelFailureDiagnostic::InvalidInput { .. } => None,
        }
    }

    /// The structured schema issues, present exactly on
    /// [`ModelFailureDiagnostic::InvalidInput`].
    pub fn issues(&self) -> Option<&[ModelInputIssue]> {
        match self {
            ModelFailureDiagnostic::InvalidInput { issues } => Some(issues.as_slice()),
            ModelFailureDiagnostic::Diagnostic { .. }
            | ModelFailureDiagnostic::HostRemediation { .. } => None,
        }
    }

    /// The redacted free-text cause, present exactly on
    /// [`ModelFailureDiagnostic::Diagnostic`].
    pub fn diagnostic_text(&self) -> Option<&SafeSummary> {
        match self {
            ModelFailureDiagnostic::Diagnostic { text } => Some(text),
            ModelFailureDiagnostic::InvalidInput { .. }
            | ModelFailureDiagnostic::HostRemediation { .. } => None,
        }
    }
}

/// Shared validator for bounded safe-identifier tags (the `FailureKind::Unknown`
/// tag): non-empty, bounded, and restricted to a safe identifier charset so no
/// raw payload/path/secret can ride along.
fn validate_safe_tag(
    kind: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), HostApiError> {
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
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
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

    /// The documented 16-item cap is enforced by the TYPE, not just one
    /// producer's `.take(16)`: direct construction and a 17-item wire payload
    /// are both rejected, and the truncating producer constructor keeps the
    /// first 16 (2026-07-19 ironloopai review finding on #6273).
    #[test]
    fn model_input_issues_cap_is_enforced_at_construction_and_on_the_wire() {
        let issue = ModelInputIssue {
            code: DispatchInputIssueCode::TypeMismatch,
            path: SafeSummary::new("schedule.kind").unwrap(),
            expected: None,
            received: None,
            schema_path: None,
        };
        let seventeen: Vec<ModelInputIssue> = (0..=MAX_MODEL_INPUT_ISSUES)
            .map(|_| issue.clone())
            .collect();
        assert!(ModelInputIssues::new(seventeen.clone()).is_err());
        assert_eq!(
            ModelInputIssues::truncating(seventeen.clone()).len(),
            MAX_MODEL_INPUT_ISSUES
        );
        // A hostile 17-item wire payload is rejected on deserialize.
        let wire = serde_json::to_value(&seventeen).unwrap();
        assert!(serde_json::from_value::<ModelInputIssues>(wire).is_err());
        // At-cap round-trips.
        let at_cap =
            ModelInputIssues::new((0..MAX_MODEL_INPUT_ISSUES).map(|_| issue.clone()).collect())
                .unwrap();
        let back: ModelInputIssues =
            serde_json::from_value(serde_json::to_value(&at_cap).unwrap()).unwrap();
        assert_eq!(back, at_cap);
    }

    #[test]
    fn output_digest_is_transparent_on_the_wire() {
        let digest = OutputDigest::new(0x0102_0304_0506_0708);
        let json = serde_json::to_value(digest).unwrap();
        assert_eq!(json, serde_json::json!(0x0102_0304_0506_0708u64));
        assert_eq!(
            serde_json::from_value::<OutputDigest>(json)
                .unwrap()
                .value(),
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
        assert_eq!(
            TerminateHint::from_bool(true),
            TerminateHint::TerminateAfterBatch
        );
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
            assert_eq!(
                FailureKind::from_tag(tag),
                kind,
                "from_tag round-trip: {tag}"
            );
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
    fn model_failure_diagnostic_invalid_input_roundtrips_structured_issues() {
        // A recoverable failure's InvalidInput diagnostic carries the structured,
        // redacted schema issues the model corrects from — code (a host_api enum)
        // plus bounded redacted path/expected/received/schema_path.
        let issue = ModelInputIssue::new(
            SafeSummary::new("schedule.kind").unwrap(),
            DispatchInputIssueCode::TypeMismatch,
        )
        .with_expected(SafeSummary::new("integer").unwrap())
        .with_received(SafeSummary::new("string").unwrap())
        .with_schema_path(SafeSummary::new("properties.schedule").unwrap());
        let diagnostic = ModelFailureDiagnostic::InvalidInput {
            issues: ModelInputIssues::truncating([issue.clone()]),
        };
        let back: ModelFailureDiagnostic =
            serde_json::from_value(serde_json::to_value(&diagnostic).unwrap()).unwrap();
        assert_eq!(back, diagnostic);
        // The structured issue survives with its code and every redacted field.
        match back {
            ModelFailureDiagnostic::InvalidInput { issues } => {
                assert_eq!(issues.len(), 1);
                assert_eq!(issues[0], issue);
                assert_eq!(issues[0].code, DispatchInputIssueCode::TypeMismatch);
                assert_eq!(issues[0].path.as_str(), "schedule.kind");
                assert_eq!(
                    issues[0].expected.as_ref().map(SafeSummary::as_str),
                    Some("integer")
                );
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn model_failure_diagnostic_free_text_roundtrips() {
        let diagnostic = ModelFailureDiagnostic::Diagnostic {
            text: SafeSummary::new("backend returned an error").unwrap(),
        };
        let wire = serde_json::to_value(&diagnostic).unwrap();
        // Internally tagged like the loop's CapabilityFailureDetail mirror.
        assert_eq!(
            wire,
            serde_json::json!({ "kind": "diagnostic", "text": "backend returned an error" })
        );
        let back: ModelFailureDiagnostic = serde_json::from_value(wire).unwrap();
        assert_eq!(back, diagnostic);
        assert_eq!(
            back.diagnostic_text().map(SafeSummary::as_str),
            Some("backend returned an error")
        );
    }

    #[test]
    fn model_failure_diagnostic_rejects_an_unsafe_free_text_on_the_wire() {
        // A hostile persisted diagnostic (path-shaped) cannot rehydrate: the
        // SafeSummary redaction contract fires on deserialize.
        let json = serde_json::json!({ "kind": "diagnostic", "text": "leaked /etc/passwd" });
        assert!(serde_json::from_value::<ModelFailureDiagnostic>(json).is_err());
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
