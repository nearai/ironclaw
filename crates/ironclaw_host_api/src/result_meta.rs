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

/// Declares the closed [`FailureKind`] vocabulary exactly once: the enum, its
/// wire tags ([`FailureKind::as_str`] and the primary [`FailureKind::from_tag`]
/// arms), and [`FailureKind::ALL`] all expand from this single list. The
/// compiler owns completeness — a variant cannot exist without its tag or its
/// `ALL` membership, so downstream exhaustiveness pins that iterate `ALL`
/// cannot silently stop covering a newly added kind.
macro_rules! declare_failure_kinds {
    ($( $(#[$meta:meta])* $variant:ident => $tag:literal ),+ $(,)?) => {
        /// The single failure vocabulary — one closed enum naming every way an
        /// operation can fail, carried unchanged from the mint site to the loop.
        ///
        /// This unifies what were five overlapping enums (`RuntimeDispatchErrorKind`'s
        /// precise mechanism names, the loop's `CapabilityFailureKind`, host_runtime's
        /// `RuntimeFailureKind`, and the loop-private `CapabilityErrorClass`) into one
        /// list plus projection functions. Layers above the mint site ask questions
        /// ([`fate`](Self::fate), [`is_retryable`](Self::is_retryable)) instead of
        /// re-declaring the domain — re-declared domains drift, and the drift is
        /// where recoverability died (#6284).
        ///
        /// Deliberately NOT `#[non_exhaustive]` and deliberately **closed** (no open
        /// `Unknown(String)` escape hatch): every producer knows what it is minting.
        /// The one sanctioned sink for values the system genuinely cannot classify
        /// is the explicit [`Unclassified`](Self::Unclassified) variant — surfaced,
        /// never retried. Downstream matches are exhaustive, so a new variant fails
        /// to compile until every projection deliberately classifies it — that
        /// compile error *is* the recoverability review.
        ///
        /// Wire tags are lowercase snake_case ([`as_str`](Self::as_str)) — they must
        /// pass `ironclaw_events`' `is_safe_error_kind` (first byte lowercase ASCII) or
        /// the event layer silently rewrites the tag to `Unclassified`. Historical tags
        /// from the retired coarse vocabularies remain readable via
        /// [`from_tag`](Self::from_tag) aliases.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum FailureKind {
            $( $(#[$meta])* $variant, )+
        }

        impl FailureKind {
            /// Every variant, for conformance tests (tag/validator round-trips).
            /// Compiler-derived from the single declaration list — a variant
            /// cannot exist outside `ALL`.
            pub const ALL: &'static [FailureKind] = &[ $(Self::$variant),+ ];

            /// The stable wire tag. Lowercase snake_case — every tag here passes the
            /// event layer's `is_safe_error_kind` validator (pinned by test).
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( Self::$variant => $tag, )+
                }
            }

            /// The primary tag decode: exactly the tags [`as_str`](Self::as_str)
            /// emits. Aliases and the fallback live in
            /// [`from_tag`](Self::from_tag).
            fn from_primary_tag(tag: &str) -> Option<Self> {
                match tag {
                    $( $tag => Some(Self::$variant), )+
                    _ => None,
                }
            }
        }
    };
}

declare_failure_kinds! {
    // ── Model-visible: the model did something wrong and can correct it ──
    /// Invoked a method the capability does not expose.
    MethodMissing => "method_missing",
    /// Invoked a capability the extension never declared in its manifest.
    UndeclaredCapability => "undeclared_capability",
    /// Invoked a capability no installed extension provides.
    UnknownCapability => "unknown_capability",
    /// Named a provider that does not exist.
    UnknownProvider => "unknown_provider",
    /// The input failed to encode against the tool's declared schema.
    InputEncode => "input_encode",
    /// The capability ran and reported a domain failure ("no such file").
    OperationFailed => "operation_failed",
    /// The result exceeded the output size cap.
    OutputTooLarge => "output_too_large",
    /// Hit a resource quota/limit the model could work around.
    Resource => "resource",
    /// Blocked by policy — permanently forbidden, not a gate.
    PolicyDenied => "policy_denied",
    /// Network egress denied by policy. Never retryable: policy does not
    /// change between attempts (retrying burns budget on a call that cannot
    /// succeed).
    NetworkDenied => "network_denied",
    /// Filesystem path access refused.
    FilesystemDenied => "filesystem_denied",
    /// Secret/credential access refused.
    SecretDenied => "secret_denied",
    /// Authorization failed (general case; the two `*Denied` variants above
    /// are specific causes).
    Authorization => "authorization",
    /// The capability surface changed between disclosure and invocation —
    /// re-issue the call against the fresh surface.
    StaleSurface => "stale_surface",
    /// The user declined the gate. Model-visible so the loop can pursue
    /// another approach; wire tag is load-bearing in the WebUI frontend.
    GateDeclined => "gate_declined",

    // ── Retry quietly: the world hiccuped, the model did nothing wrong ──
    /// Transport-level network failure (timeout, connection reset) — distinct
    /// from [`NetworkDenied`](Self::NetworkDenied), which is policy.
    Network => "network",
    /// Temporary fault; try again.
    Transient => "transient",
    /// The backing service is down right now.
    Unavailable => "unavailable",
    /// An upstream/backend service errored.
    Backend => "backend",
    /// Host internal fault, retryable.
    Internal => "internal",

    // ── Model-visible: the extension itself is broken ──
    /// Extension guest code crashed/trapped.
    Guest => "guest",
    /// A sandboxed process exited with failure.
    ExitFailure => "exit_failure",
    /// The tool returned output that could not be decoded.
    OutputDecode => "output_decode",
    /// The result violated the declared result schema.
    InvalidResult => "invalid_result",
    /// The extension exceeded its memory limit.
    Memory => "memory",

    // ── Model-visible with user-setup remediation: configuration faults ──
    /// The extension manifest is invalid.
    Manifest => "manifest",
    /// The extension was built for a different runtime than it was routed to.
    ExtensionRuntimeMismatch => "extension_runtime_mismatch",
    /// Dispatch routed to the wrong runtime lane.
    RuntimeMismatch => "runtime_mismatch",
    /// The required runtime backend is not installed.
    MissingRuntimeBackend => "missing_runtime_backend",
    /// The runtime kind is not supported in this deployment.
    UnsupportedRunner => "unsupported_runner",
    /// The runtime for this capability is absent.
    MissingRuntime => "missing_runtime",
    /// Host-side client machinery fault.
    Client => "client",
    /// Host-side executor machinery fault.
    Executor => "executor",

    // ── Model-visible: the system could not classify the failure ──
    /// The failure could not be classified: an unrecognized wire tag (a future
    /// build's kind, a legacy open-set tag) or a lane's redacted unknown
    /// bucket. Deliberately NOT retryable — an unclassifiable failure may be
    /// permanent, so it surfaces to the model instead of silently consuming
    /// retry budget. Distinct from [`Internal`](Self::Internal), which is a
    /// *classified* retryable host fault.
    Unclassified => "unclassified",

    // ── Park: waiting on a human or a gate ──
    /// A credential is required — park the run, launch the auth flow, resume.
    AuthRequired => "auth_required",

    // ── Terminal: the only honest run-ender in this vocabulary ──
    /// Someone stopped the run. Nothing failed.
    Cancelled => "cancelled",
}

/// What the loop does with a [`FailureKind`] — the single fate decision,
/// decided once beside the enum instead of re-derived per layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureFate {
    /// Retry with backoff; the model is not consulted until budget exhausts.
    Retry,
    /// Surface to the model as a tool error it can act on; the run continues.
    ModelVisible,
    /// Park the run on a gate (auth/approval) and resume when resolved.
    Park,
    /// End the run. Reserved for genuine invariants (cancellation).
    Terminal,
}

impl FailureKind {
    /// The one fate decision for this kind. Exhaustive and wildcard-free:
    /// adding a variant refuses to compile until its fate is chosen here.
    pub fn fate(self) -> FailureFate {
        match self {
            // The world hiccuped — retry quietly.
            Self::Network
            | Self::Transient
            | Self::Unavailable
            | Self::Backend
            | Self::Internal => FailureFate::Retry,
            // Waiting on a human/credential gate.
            Self::AuthRequired => FailureFate::Park,
            // The only sanctioned run-ender here.
            Self::Cancelled => FailureFate::Terminal,
            // Everything else the model sees and survives — including config
            // faults (the model relays the setup step to the user) and policy
            // denials (the model routes around them).
            Self::MethodMissing
            | Self::UndeclaredCapability
            | Self::UnknownCapability
            | Self::UnknownProvider
            | Self::InputEncode
            | Self::OperationFailed
            | Self::OutputTooLarge
            | Self::Resource
            | Self::PolicyDenied
            | Self::NetworkDenied
            | Self::FilesystemDenied
            | Self::SecretDenied
            | Self::Authorization
            | Self::StaleSurface
            | Self::GateDeclined
            | Self::Guest
            | Self::ExitFailure
            | Self::OutputDecode
            | Self::InvalidResult
            | Self::Memory
            | Self::Manifest
            | Self::ExtensionRuntimeMismatch
            | Self::RuntimeMismatch
            | Self::MissingRuntimeBackend
            | Self::UnsupportedRunner
            | Self::MissingRuntime
            | Self::Client
            | Self::Executor
            // Unclassifiable is NOT retryable: it may be permanent, and
            // quiet retries would burn budget on a call that cannot succeed.
            | Self::Unclassified => FailureFate::ModelVisible,
        }
    }

    /// Whether the host should quietly retry before surfacing anything.
    pub fn is_retryable(self) -> bool {
        matches!(self.fate(), FailureFate::Retry)
    }

    /// Reconstruct from a wire tag. Total over history: every tag any retired
    /// vocabulary ever emitted maps to its nearest surviving variant
    /// (`.claude/rules/types.md` — migrations preserve every historical value);
    /// an unrecognized tag falls back to [`Unclassified`](Self::Unclassified),
    /// the non-retryable model-visible sink — an unknown tag may name a
    /// permanent failure, so it must not silently consume retry budget.
    pub fn from_tag(tag: &str) -> Self {
        if let Some(kind) = Self::from_primary_tag(tag) {
            return kind;
        }
        match tag {
            // Historical tags from the retired coarse vocabularies.
            "invalid_input" => Self::InputEncode,
            "invalid_output" => Self::OutputDecode,
            "process" => Self::ExitFailure,
            "dispatcher" => Self::Internal,
            "permanent" => Self::OperationFailed,
            "auth_denied" => Self::Authorization,
            // Unrecognized (legacy open-set tags from the retired `Unknown`
            // funnel, or a future build's kind): the explicit non-retryable
            // unclassified sink.
            _ => Self::Unclassified,
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
    fn failure_kind_tags_round_trip_for_every_variant() {
        for &kind in FailureKind::ALL {
            let tag = kind.as_str();
            assert_eq!(
                FailureKind::from_tag(tag),
                kind,
                "from_tag round-trip: {tag}"
            );
            let wire = serde_json::to_value(kind).unwrap();
            assert_eq!(wire, serde_json::Value::String(tag.to_string()));
            assert_eq!(serde_json::from_value::<FailureKind>(wire).unwrap(), kind);
        }
    }

    #[test]
    fn failure_kind_all_is_complete_and_distinct() {
        // ALL covers every variant exactly once. Membership is compiler-owned:
        // `declare_failure_kinds!` expands the enum, the tags, and `ALL` from
        // one list, so a variant cannot exist outside `ALL`. This test pins
        // the remaining runtime property — tag distinctness.
        let tags: std::collections::HashSet<&str> =
            FailureKind::ALL.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(tags.len(), FailureKind::ALL.len());
    }

    #[test]
    fn failure_kind_historical_tags_stay_readable() {
        // Every tag a retired vocabulary ever emitted maps to a surviving
        // variant (types.md: migrations preserve every historical value).
        let historical = [
            ("invalid_input", FailureKind::InputEncode),
            ("invalid_output", FailureKind::OutputDecode),
            ("process", FailureKind::ExitFailure),
            ("dispatcher", FailureKind::Internal),
            ("permanent", FailureKind::OperationFailed),
            ("auth_denied", FailureKind::Authorization),
        ];
        for (tag, expected) in historical {
            assert_eq!(FailureKind::from_tag(tag), expected, "historical: {tag}");
        }
        // Legacy open-set tags (the retired `Unknown` funnel) and future
        // builds' tags fall back to the explicit non-retryable `Unclassified`
        // sink instead of the retryable host-fault bucket — an unknown tag
        // may name a permanent failure, so it must not burn retry budget.
        assert_eq!(
            FailureKind::from_tag("quota_exceeded"),
            FailureKind::Unclassified
        );
    }

    #[test]
    fn failure_kind_fates_pin_the_recoverability_contract() {
        // The epic's invariant (#6284): exactly one variant may end a run.
        let terminal: Vec<FailureKind> = FailureKind::ALL
            .iter()
            .copied()
            .filter(|kind| kind.fate() == FailureFate::Terminal)
            .collect();
        assert_eq!(terminal, vec![FailureKind::Cancelled]);
        // Policy denials never retry (a denied call cannot succeed on retry).
        assert!(!FailureKind::NetworkDenied.is_retryable());
        assert!(!FailureKind::PolicyDenied.is_retryable());
        // Transport faults do retry.
        assert!(FailureKind::Network.is_retryable());
        // The unclassifiable sink surfaces model-visibly and never retries —
        // it may be permanent, so quiet retries would burn budget.
        assert_eq!(FailureKind::Unclassified.fate(), FailureFate::ModelVisible);
        assert!(!FailureKind::Unclassified.is_retryable());
        assert_eq!(FailureKind::Unclassified.as_str(), "unclassified");
        // The frontend's load-bearing literal survives the merge.
        assert_eq!(FailureKind::GateDeclined.as_str(), "gate_declined");
    }

    #[test]
    fn failure_kind_tags_pass_the_event_layer_shape() {
        // ironclaw_events' `is_safe_error_kind` silently rewrites tags that do
        // not start with lowercase ASCII to "Unclassified" — a lossy write.
        // Pin the shape here (host_api cannot depend on ironclaw_events).
        for kind in FailureKind::ALL {
            let tag = kind.as_str();
            assert!(
                tag.chars().next().is_some_and(|c| c.is_ascii_lowercase()),
                "tag must start lowercase or the event layer drops it: {tag}"
            );
            assert!(
                tag.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "tag must stay in the safe charset: {tag}"
            );
        }
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
