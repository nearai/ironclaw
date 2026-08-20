//! Neutral capability dispatch port contracts.
//!
//! These types describe an already-authorized capability dispatch request and
//! normalized runtime result. Concrete dispatcher/runtime crates implement the
//! behavior; caller-facing workflow crates depend only on this neutral port.

use std::{fmt, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    authorized::Authorized,
    decision::RuntimeCredentialAuthRequirement,
    host_remediation::HostRemediation,
    ids::{CapabilityId, ExtensionId, RunId, SecretHandle, UserId},
    invocation::InvocationOrigin,
    mount::MountView,
    resource::{
        ResourceEstimate, ResourceReceipt, ResourceReservation, ResourceScope, ResourceUsage,
    },
    runtime::RuntimeKind,
};

/// Stable provider error code supplied by a protocol decoder.
///
/// Codes are untrusted metadata: they are bounded here, omitted from debug
/// output with the rest of [`ProviderDiagnostic`], and scrubbed before model
/// visibility.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderErrorCode(String);

impl ProviderErrorCode {
    pub fn new(value: impl Into<String>) -> Self {
        Self(bound_provider_text(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Untrusted provider-authored message retained for the model diagnostic seam.
#[derive(Clone, PartialEq, Eq)]
pub struct UntrustedProviderMessage(String);

impl UntrustedProviderMessage {
    pub fn new(value: impl Into<String>) -> Self {
        Self(bound_provider_text(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Passive protocol diagnostic shared by authentication and ordinary provider
/// rejection paths.
///
/// This value is not safe for logs, persistence, UI, or model output. The loop
/// projection owns secret scrubbing and injection fencing.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderDiagnostic {
    pub code: Option<ProviderErrorCode>,
    pub message: Option<UntrustedProviderMessage>,
    pub retry_after: Option<Duration>,
}

impl fmt::Debug for ProviderDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderDiagnostic(<redacted>)")
    }
}

/// Truncate `text` to at most `max_bytes` bytes, stepping back to the
/// nearest UTF-8 char boundary so a multi-byte character is never split.
/// Returns the truncated prefix and whether truncation occurred.
fn truncate_at_char_boundary(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn bound_provider_text(mut value: String) -> String {
    let (truncated, did_truncate) =
        truncate_at_char_boundary(&value, crate::result_meta::MODEL_DIAGNOSTIC_MAX_BYTES);
    if did_truncate {
        let new_len = truncated.len();
        value.truncate(new_len);
    }
    value
}

/// Canonical model-diagnostic rendering for provider metadata.
///
/// The returned string is still untrusted and must pass the host's secret
/// scrubber and prompt-injection fence before model visibility.
pub fn provider_diagnostic_model_cause(diagnostic: &ProviderDiagnostic) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(code) = &diagnostic.code {
        parts.push(format!("provider error code: {}", code.as_str()));
    }
    if let Some(message) = &diagnostic.message {
        parts.push(format!("provider message: {}", message.as_str()));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Internal adapter request produced after a sealed [`Authorized`] witness is
/// consumed by the dispatcher.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub authenticated_actor_user_id: Option<UserId>,
    /// Loop turn-run identity forwarded from `ExecutionContext::run_id`.
    /// `None` for non-loop callers.
    pub run_id: Option<RunId>,
    /// Authoritative invocation origin preserved from the sealed
    /// [`Authorized`] witness for capability-boundary policy.
    pub origin: InvocationOrigin,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: Value,
}

/// Display-only preview metadata for a completed capability result.
///
/// This side channel lets runtime/tool implementations provide renderer-ready
/// material without changing the model-visible capability output shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDisplayOutputPreview {
    pub output_summary: Option<String>,
    /// Raw, unsanitized content — callers MUST sanitize before display or logging.
    /// The canonical sanitization point is the projection layer in
    /// `ironclaw_composition`. New consumers must not read this field
    /// without sanitizing.
    pub output_preview: String,
    pub output_kind: String,
    pub subtitle: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDisplayText {
    pub text: String,
    pub truncated: bool,
}

pub fn truncate_capability_display_text(text: &str, max_bytes: usize) -> CapabilityDisplayText {
    let (truncated, did_truncate) = truncate_at_char_boundary(text, max_bytes);
    CapabilityDisplayText {
        text: truncated.to_string(),
        truncated: did_truncate,
    }
}

/// Normalized dispatch result returned by a runtime dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDispatchResult {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
}

/// Stable input issue code for dispatch validation failures.
///
/// Also the canonical code for the loop/turns-side `CapabilityInputIssue` wire
/// type, so it carries `rename_all = "snake_case"` serde: serialized as
/// `missing_required` / `unexpected_field` / `type_mismatch` / `invalid_value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchInputIssueCode {
    MissingRequired,
    UnexpectedField,
    TypeMismatch,
    InvalidValue,
}

/// Stable input issue for dispatch validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchInputIssue {
    pub path: String,
    pub code: DispatchInputIssueCode,
    pub expected: Option<String>,
    pub received: Option<String>,
    pub schema_path: Option<String>,
}

impl DispatchInputIssue {
    pub fn new(path: impl Into<String>, code: DispatchInputIssueCode) -> Self {
        Self {
            path: path.into(),
            code,
            expected: None,
            received: None,
            schema_path: None,
        }
    }

    pub fn expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn received(mut self, received: impl Into<String>) -> Self {
        self.received = Some(received.into());
        self
    }

    pub fn schema_path(mut self, schema_path: impl Into<String>) -> Self {
        self.schema_path = Some(schema_path.into());
        self
    }
}

/// Stable structured dispatch failure details for dispatch validation failures.
#[derive(Clone, PartialEq, Eq)]
pub enum DispatchFailureDetail {
    InvalidInput {
        issues: Vec<DispatchInputIssue>,
    },
    /// Free-text raw failure cause preserved when the host-authored
    /// `safe_summary` cannot pass the strict loop safe-summary validator
    /// (e.g. it names a concrete path such as `/testbed/replacer.go`, or
    /// carries newlines from a shell error). The summary shown to the model
    /// degrades to the fixed category sentence; this text rides the
    /// model-visible diagnostic detail channel, where secret VALUES are
    /// scrubbed and disallowed control characters are normalized at the loop
    /// boundary before the model observes it.
    Diagnostic {
        text: String,
    },
    /// Host-authored operator remediation — the TRUSTED text channel.
    ///
    /// Distinct from [`Self::Diagnostic`] by PROVENANCE, not content shape:
    /// `Diagnostic` carries an untrusted raw cause (capability output, a
    /// backend error string) that is redacted hard downstream and collapses to
    /// the safe-summary placeholder when it names a path or a URL;
    /// this variant carries a host-authored instruction that must survive
    /// intact. See [`HostRemediation`] for the invariant and the value guard.
    HostRemediation {
        text: HostRemediation,
    },
}

impl fmt::Debug for DispatchFailureDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { issues } => formatter
                .debug_struct("InvalidInput")
                .field("issues", issues)
                .finish(),
            Self::Diagnostic { .. } => formatter
                .debug_struct("Diagnostic")
                .field("text", &"<redacted>")
                .finish(),
            Self::HostRemediation { text } => formatter
                .debug_struct("HostRemediation")
                .field("text", text)
                .finish(),
        }
    }
}

/// Stable, redacted runtime failure categories surfaced through the dispatch port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDispatchErrorKind {
    Backend,
    Client,
    Executor,
    ExitFailure,
    ExtensionRuntimeMismatch,
    FilesystemDenied,
    Guest,
    InputEncode,
    InvalidResult,
    Manifest,
    Memory,
    MethodMissing,
    /// Transport-level network failure (unreachable, reset, timeout) —
    /// distinct from [`NetworkDenied`](Self::NetworkDenied), which is an egress
    /// policy denial. Transport faults retry; policy denials never do.
    Network,
    NetworkDenied,
    OperationFailed,
    OutputDecode,
    OutputTooLarge,
    PolicyDenied,
    Resource,
    SecretDenied,
    UndeclaredCapability,
    UnsupportedRunner,
    Unknown,
}

/// Fixed user-facing fallback for [`RuntimeDispatchErrorKind::InputEncode`].
///
/// This exact sentence is whitelisted by downstream safety validators after
/// they reject arbitrary summaries mentioning raw tool input.
pub const INPUT_ENCODE_HUMAN_SUMMARY: &str = "the tool input could not be encoded";

/// Lossless injection into the unified [`FailureKind`](crate::result_meta::FailureKind):
/// every precise mechanism name survives 1:1 — this replaces the retired
/// 22→12 coarsening fold that destroyed 17 names (and with them, every
/// remediation hint) on the way to the loop. The single non-identity edge is
/// `Unknown` → `Unclassified`: `Unknown` is the dispatch lane's redaction
/// bucket for failures it cannot classify, and the unified vocabulary's
/// explicit `Unclassified` sink surfaces those model-visibly without retry —
/// an unclassifiable failure may be permanent, so routing it to the retryable
/// `Internal` bucket (the retired fold's mapping) burned retry budget on
/// calls that could never succeed.
impl From<RuntimeDispatchErrorKind> for crate::result_meta::FailureKind {
    fn from(kind: RuntimeDispatchErrorKind) -> Self {
        match kind {
            RuntimeDispatchErrorKind::Backend => Self::Backend,
            RuntimeDispatchErrorKind::Client => Self::Client,
            RuntimeDispatchErrorKind::Executor => Self::Executor,
            RuntimeDispatchErrorKind::ExitFailure => Self::ExitFailure,
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch => Self::ExtensionRuntimeMismatch,
            RuntimeDispatchErrorKind::FilesystemDenied => Self::FilesystemDenied,
            RuntimeDispatchErrorKind::Guest => Self::Guest,
            RuntimeDispatchErrorKind::InputEncode => Self::InputEncode,
            RuntimeDispatchErrorKind::InvalidResult => Self::InvalidResult,
            RuntimeDispatchErrorKind::Manifest => Self::Manifest,
            RuntimeDispatchErrorKind::Memory => Self::Memory,
            RuntimeDispatchErrorKind::MethodMissing => Self::MethodMissing,
            RuntimeDispatchErrorKind::Network => Self::Network,
            RuntimeDispatchErrorKind::NetworkDenied => Self::NetworkDenied,
            RuntimeDispatchErrorKind::OperationFailed => Self::OperationFailed,
            RuntimeDispatchErrorKind::OutputDecode => Self::OutputDecode,
            RuntimeDispatchErrorKind::OutputTooLarge => Self::OutputTooLarge,
            RuntimeDispatchErrorKind::PolicyDenied => Self::PolicyDenied,
            RuntimeDispatchErrorKind::Resource => Self::Resource,
            RuntimeDispatchErrorKind::SecretDenied => Self::SecretDenied,
            RuntimeDispatchErrorKind::UndeclaredCapability => Self::UndeclaredCapability,
            RuntimeDispatchErrorKind::UnsupportedRunner => Self::UnsupportedRunner,
            RuntimeDispatchErrorKind::Unknown => Self::Unclassified,
        }
    }
}

/// Lossless injection for the dispatch-control-plane siblings — each has its
/// own named variant in the unified vocabulary (they were previously coarsened
/// into `InvalidOutput`/`MissingRuntime`/`Authorization`/`Backend`).
impl From<DispatchFailureKind> for crate::result_meta::FailureKind {
    fn from(kind: DispatchFailureKind) -> Self {
        match kind {
            DispatchFailureKind::UnknownCapability => Self::UnknownCapability,
            DispatchFailureKind::UnknownProvider => Self::UnknownProvider,
            DispatchFailureKind::RuntimeMismatch => Self::RuntimeMismatch,
            DispatchFailureKind::MissingRuntimeBackend => Self::MissingRuntimeBackend,
            DispatchFailureKind::UnsupportedRuntime => Self::UnsupportedRunner,
            DispatchFailureKind::AuthRequired => Self::AuthRequired,
            DispatchFailureKind::Runtime(kind) => kind.into(),
        }
    }
}

impl RuntimeDispatchErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backend => "Backend",
            Self::Client => "Client",
            Self::Executor => "Executor",
            Self::ExitFailure => "ExitFailure",
            Self::ExtensionRuntimeMismatch => "ExtensionRuntimeMismatch",
            Self::FilesystemDenied => "FilesystemDenied",
            Self::Guest => "Guest",
            Self::InputEncode => "InputEncode",
            Self::InvalidResult => "InvalidResult",
            Self::Manifest => "Manifest",
            Self::Memory => "Memory",
            Self::MethodMissing => "MethodMissing",
            Self::Network => "Network",
            Self::NetworkDenied => "NetworkDenied",
            Self::OperationFailed => "OperationFailed",
            Self::OutputDecode => "OutputDecode",
            Self::OutputTooLarge => "OutputTooLarge",
            Self::PolicyDenied => "PolicyDenied",
            Self::Resource => "Resource",
            Self::SecretDenied => "SecretDenied",
            Self::UndeclaredCapability => "UndeclaredCapability",
            Self::UnsupportedRunner => "UnsupportedRunner",
            Self::Unknown => "Unknown",
        }
    }

    /// Human-readable, user-facing summary for this redacted failure kind.
    ///
    /// Used when a dispatch failure carries no host-authored `safe_summary`, so
    /// surfaces show a plain sentence instead of the stable category token
    /// (`as_str`). Fixed host-authored text — no raw content interpolated.
    pub const fn human_summary(self) -> &'static str {
        match self {
            Self::Backend => "the tool's backend failed",
            Self::Client => "the tool client failed to run",
            Self::Executor => "the tool executor failed",
            Self::ExitFailure => "the tool exited with an error",
            Self::ExtensionRuntimeMismatch => "the tool runtime did not match its extension",
            Self::FilesystemDenied => "the tool was denied filesystem access",
            Self::Guest => "the tool reported an internal error",
            Self::InputEncode => INPUT_ENCODE_HUMAN_SUMMARY,
            Self::InvalidResult => "the tool returned an invalid result",
            Self::Manifest => "the tool manifest is invalid",
            Self::Memory => "the tool exceeded its memory limit",
            Self::MethodMissing => "the tool method is not available",
            Self::Network => "a network error interrupted the tool",
            Self::NetworkDenied => "the tool was denied network access",
            Self::OperationFailed => "the tool operation failed",
            Self::OutputDecode => "the tool output could not be decoded",
            Self::OutputTooLarge => "the tool output was too large",
            Self::PolicyDenied => "the tool call was denied by policy",
            Self::Resource => "the tool ran out of resources",
            Self::SecretDenied => "the tool was denied access to a required secret",
            Self::UndeclaredCapability => "the tool used an undeclared capability",
            Self::UnsupportedRunner => "the tool runner is not supported",
            Self::Unknown => "the tool failed for an unknown reason",
        }
    }

    /// Sanitizer-compatible event/audit token for this redacted failure kind.
    pub const fn event_kind(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Client => "client",
            Self::Executor => "executor",
            Self::ExitFailure => "exit_failure",
            Self::ExtensionRuntimeMismatch => "extension.runtime_mismatch",
            Self::FilesystemDenied => "filesystem_denied",
            Self::Guest => "guest",
            Self::InputEncode => "input_encode",
            Self::InvalidResult => "invalid_result",
            Self::Manifest => "manifest",
            Self::Memory => "memory",
            Self::MethodMissing => "method_missing",
            Self::Network => "network",
            Self::NetworkDenied => "network_denied",
            Self::OperationFailed => "operation_failed",
            Self::OutputDecode => "output_decode",
            Self::OutputTooLarge => "output_too_large",
            Self::PolicyDenied => "policy_denied",
            Self::Resource => "resource",
            Self::SecretDenied => "secret_denied",
            Self::UndeclaredCapability => "undeclared_capability",
            Self::UnsupportedRunner => "unsupported_runner",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for RuntimeDispatchErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable, redacted dispatch failure categories surfaced above the neutral dispatch port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchFailureKind {
    UnknownCapability,
    UnknownProvider,
    RuntimeMismatch,
    MissingRuntimeBackend,
    UnsupportedRuntime,
    AuthRequired,
    Runtime(RuntimeDispatchErrorKind),
}

impl DispatchFailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCapability => "UnknownCapability",
            Self::UnknownProvider => "UnknownProvider",
            Self::RuntimeMismatch => "RuntimeMismatch",
            Self::MissingRuntimeBackend => "MissingRuntimeBackend",
            Self::UnsupportedRuntime => "UnsupportedRuntime",
            Self::AuthRequired => "AuthRequired",
            Self::Runtime(kind) => kind.as_str(),
        }
    }

    /// Human-readable, user-facing summary for this redacted failure kind.
    /// Fixed host-authored text — no raw content interpolated.
    pub const fn human_summary(self) -> &'static str {
        match self {
            Self::UnknownCapability => "the requested tool was not found",
            Self::UnknownProvider => "the tool provider was not found",
            Self::RuntimeMismatch => "the tool runtime did not match",
            Self::MissingRuntimeBackend => "the tool runtime backend is unavailable",
            Self::UnsupportedRuntime => "the tool runtime is not supported",
            Self::AuthRequired => "the tool requires authentication",
            Self::Runtime(kind) => kind.human_summary(),
        }
    }
}

impl std::fmt::Display for DispatchFailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Runtime dispatch failures surfaced through the neutral host API port.
#[derive(Error)]
pub enum DispatchError {
    #[error("unknown capability {capability}")]
    UnknownCapability { capability: CapabilityId },
    #[error("capability {capability} provider {provider} is not registered")]
    UnknownProvider {
        capability: CapabilityId,
        provider: ExtensionId,
    },
    #[error(
        "capability {capability} descriptor runtime {descriptor_runtime:?} does not match package runtime {package_runtime:?}"
    )]
    RuntimeMismatch {
        capability: CapabilityId,
        descriptor_runtime: RuntimeKind,
        package_runtime: RuntimeKind,
    },
    #[error("runtime backend {runtime:?} is not configured")]
    MissingRuntimeBackend { runtime: RuntimeKind },
    #[error(
        "runtime {runtime:?} is recognized but not supported by this dispatcher yet for capability {capability}"
    )]
    UnsupportedRuntime {
        capability: CapabilityId,
        runtime: RuntimeKind,
    },
    #[error("capability {capability} has no sealed dispatch authorization")]
    MissingAuthorization { capability: CapabilityId },
    #[error("authorized dispatch witness for capability {capability} has expired")]
    AuthorizationExpired { capability: CapabilityId },
    #[error("process dispatch is missing durable authorization for capability {capability}")]
    MissingProcessAuthorization { capability: CapabilityId },
    /// Authentication is required to dispatch this capability.
    ///
    /// `required_secrets` names the credentials the caller must stage.  The
    /// field is intentionally absent from the `Debug` output to avoid leaking
    /// secret-handle identifiers into logs.
    #[error("capability {capability} dispatch requires authentication")]
    AuthRequired {
        capability: CapabilityId,
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
        model_visible_cause: Option<Box<ProviderDiagnostic>>,
    },
    /// Provider/protocol rejection shared across runtime lanes. The lane is
    /// diagnostic metadata; callers branch on `kind`, not implementation.
    #[error("provider dispatch rejected: {kind}")]
    Rejected {
        runtime: Option<RuntimeKind>,
        kind: DispatchFailureKind,
        diagnostic: Option<ProviderDiagnostic>,
        detail: Option<DispatchFailureDetail>,
    },
    /// MCP dispatch failure. `model_visible_cause` carries the raw backend cause —
    /// it is NOT yet display/model-safe: secret VALUES are scrubbed downstream
    /// at the model-visible Diagnostic seam (`scrub_model_visible_detail`),
    /// and display surfaces run their own redaction. Do not log or surface it
    /// directly.
    #[error("MCP dispatch failed: {kind}")]
    Mcp {
        kind: RuntimeDispatchErrorKind,
        model_visible_cause: Option<String>,
    },
    /// Script dispatch failure. Same `model_visible_cause` contract as [`Self::Mcp`]:
    /// raw cause, scrubbed downstream — not directly displayable.
    #[error("script dispatch failed: {kind}")]
    Script {
        kind: RuntimeDispatchErrorKind,
        model_visible_cause: Option<String>,
    },
    /// WASM guest dispatch failure. `model_visible_cause` carries the best available
    /// cause: the stable, host-sanitized error code a structured guest error
    /// declared (e.g. a Slack `channel_not_found`) when present, otherwise the
    /// raw error text (secret VALUES are scrubbed downstream at the
    /// model-visible Diagnostic seam), so the failure keeps its actionable
    /// cause instead of collapsing to the kind's generic sentence.
    #[error("WASM dispatch failed: {kind}")]
    Wasm {
        kind: RuntimeDispatchErrorKind,
        model_visible_cause: Option<String>,
    },
    #[error("first-party dispatch failed: {kind}")]
    FirstParty {
        kind: RuntimeDispatchErrorKind,
        safe_summary: Option<String>,
        detail: Option<DispatchFailureDetail>,
    },
}

impl fmt::Debug for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCapability { capability } => f
                .debug_struct("UnknownCapability")
                .field("capability", capability)
                .finish(),
            Self::UnknownProvider {
                capability,
                provider,
            } => f
                .debug_struct("UnknownProvider")
                .field("capability", capability)
                .field("provider", provider)
                .finish(),
            Self::RuntimeMismatch {
                capability,
                descriptor_runtime,
                package_runtime,
            } => f
                .debug_struct("RuntimeMismatch")
                .field("capability", capability)
                .field("descriptor_runtime", descriptor_runtime)
                .field("package_runtime", package_runtime)
                .finish(),
            Self::MissingRuntimeBackend { runtime } => f
                .debug_struct("MissingRuntimeBackend")
                .field("runtime", runtime)
                .finish(),
            Self::UnsupportedRuntime {
                capability,
                runtime,
            } => f
                .debug_struct("UnsupportedRuntime")
                .field("capability", capability)
                .field("runtime", runtime)
                .finish(),
            Self::MissingAuthorization { capability } => f
                .debug_struct("MissingAuthorization")
                .field("capability", capability)
                .finish(),
            Self::AuthorizationExpired { capability } => f
                .debug_struct("AuthorizationExpired")
                .field("capability", capability)
                .finish(),
            Self::MissingProcessAuthorization { capability } => f
                .debug_struct("MissingProcessAuthorization")
                .field("capability", capability)
                .finish(),
            // `required_secrets` handle names are omitted from Debug output to
            // prevent leaking secret identifiers into logs and error chains.
            Self::AuthRequired {
                capability,
                required_secrets,
                credential_requirements,
                ..
            } => f
                .debug_struct("AuthRequired")
                .field("capability", capability)
                .field(
                    "required_secrets",
                    &format!("[{} handle(s) redacted]", required_secrets.len()),
                )
                .field(
                    "credential_requirements",
                    &format!(
                        "[{} requirement(s) redacted]",
                        credential_requirements.len()
                    ),
                )
                .finish(),
            Self::Rejected {
                runtime,
                kind,
                detail,
                ..
            } => f
                .debug_struct("Rejected")
                .field("runtime", runtime)
                .field("kind", kind)
                .field("detail", detail)
                .field("diagnostic", &"<redacted>")
                .finish(),
            Self::Mcp { kind, .. } => f.debug_struct("Mcp").field("kind", kind).finish(),
            Self::Script { kind, .. } => f.debug_struct("Script").field("kind", kind).finish(),
            Self::Wasm { kind, .. } => f.debug_struct("Wasm").field("kind", kind).finish(),
            Self::FirstParty { kind, .. } => {
                f.debug_struct("FirstParty").field("kind", kind).finish()
            }
        }
    }
}

/// Stable two-variant error for staged credential operations.
///
/// Both the host-runtime staging layer (`ProductAuthCredentialStageError`) and the
/// per-extension staging traits (e.g. `GsuiteCredentialStageError`) map 1:1 to this
/// type so that no mechanical conversion glue is needed across crate boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStageError {
    /// Credential is missing, expired, or revoked — user must re-authenticate.
    AuthRequired,
    /// Internal staging failure not attributable to the user's credentials.
    Backend,
}

impl DispatchError {
    pub const fn failure_kind(&self) -> DispatchFailureKind {
        match self {
            Self::UnknownCapability { .. } => DispatchFailureKind::UnknownCapability,
            Self::UnknownProvider { .. } => DispatchFailureKind::UnknownProvider,
            Self::RuntimeMismatch { .. } => DispatchFailureKind::RuntimeMismatch,
            Self::MissingRuntimeBackend { .. } => DispatchFailureKind::MissingRuntimeBackend,
            Self::UnsupportedRuntime { .. } => DispatchFailureKind::UnsupportedRuntime,
            Self::MissingAuthorization { .. }
            | Self::AuthorizationExpired { .. }
            | Self::MissingProcessAuthorization { .. } => {
                DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::PolicyDenied)
            }
            Self::AuthRequired { .. } => DispatchFailureKind::AuthRequired,
            Self::Rejected { kind, .. } => *kind,
            Self::Mcp { kind, .. }
            | Self::Script { kind, .. }
            | Self::Wasm { kind, .. }
            | Self::FirstParty { kind, .. } => DispatchFailureKind::Runtime(*kind),
        }
    }

    /// Stable event-token string for the error, suitable for telemetry and structured logging.
    ///
    /// This is the single canonical source for dispatch error event tokens; crates should
    /// call this method rather than maintaining a parallel local `match` over `DispatchError`.
    pub fn event_kind(&self) -> &'static str {
        match self {
            Self::UnknownCapability { .. } => "unknown_capability",
            Self::UnknownProvider { .. } => "unknown_provider",
            Self::RuntimeMismatch { .. } => "runtime_mismatch",
            Self::MissingRuntimeBackend { .. } => "missing_runtime_backend",
            Self::UnsupportedRuntime { .. } => "unsupported_runtime",
            Self::MissingAuthorization { .. } => "missing_authorization",
            Self::AuthorizationExpired { .. } => "authorization_expired",
            Self::MissingProcessAuthorization { .. } => "missing_process_authorization",
            Self::AuthRequired { .. } => "auth_required",
            Self::Rejected { kind, .. } => match kind {
                DispatchFailureKind::Runtime(kind) => kind.event_kind(),
                _ => "provider_rejected",
            },
            Self::Mcp { kind, .. }
            | Self::Script { kind, .. }
            | Self::Wasm { kind, .. }
            | Self::FirstParty { kind, .. } => kind.event_kind(),
        }
    }
}

/// Interface for already-authorized runtime dispatch.
#[async_trait]
pub trait CapabilityDispatcher: Send + Sync {
    /// Dispatches one already-authorized JSON capability request and must not perform caller-facing authorization or approval resolution.
    async fn dispatch_json(
        &self,
        authorized: Authorized,
    ) -> Result<CapabilityDispatchResult, DispatchError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_failure_kind_human_summary_is_plain_language_not_category_token() {
        // The user-facing summary must read as a sentence, never the stable
        // category token (which is for routing/metrics/audit only).
        let runtime = DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::InputEncode);
        assert_eq!(runtime.human_summary(), INPUT_ENCODE_HUMAN_SUMMARY);
        assert_ne!(runtime.human_summary(), runtime.as_str());
        assert_eq!(
            DispatchFailureKind::UnknownCapability.human_summary(),
            "the requested tool was not found"
        );
        // Every variant maps to non-empty, lowercase-initial plain text.
        for kind in [
            RuntimeDispatchErrorKind::Backend,
            RuntimeDispatchErrorKind::InputEncode,
            RuntimeDispatchErrorKind::OperationFailed,
            RuntimeDispatchErrorKind::NetworkDenied,
            RuntimeDispatchErrorKind::Unknown,
        ] {
            let summary = kind.human_summary();
            assert!(!summary.is_empty());
            assert_ne!(summary, kind.as_str());
        }
    }

    #[test]
    fn dispatch_input_issue_builder_methods_round_trip_optional_fields() {
        let issue = DispatchInputIssue::new("schedule.kind", DispatchInputIssueCode::TypeMismatch)
            .expected("string")
            .received("number")
            .schema_path("/properties/schedule/oneOf/0/properties/kind");

        assert_eq!(issue.path, "schedule.kind");
        assert_eq!(issue.code, DispatchInputIssueCode::TypeMismatch);
        assert_eq!(issue.expected.as_deref(), Some("string"));
        assert_eq!(issue.received.as_deref(), Some("number"));
        assert_eq!(
            issue.schema_path.as_deref(),
            Some("/properties/schedule/oneOf/0/properties/kind")
        );
    }

    #[test]
    fn provider_diagnostic_is_redacted_and_utf8_byte_bounded() {
        let message = UntrustedProviderMessage::new(format!(
            "a{}",
            "é".repeat(crate::result_meta::MODEL_DIAGNOSTIC_MAX_BYTES)
        ));
        let diagnostic = ProviderDiagnostic {
            code: None,
            message: Some(message.clone()),
            retry_after: None,
        };

        assert!(message.as_str().len() <= crate::result_meta::MODEL_DIAGNOSTIC_MAX_BYTES);
        assert!(message.as_str().is_char_boundary(message.as_str().len()));
        assert!(!format!("{diagnostic:?}").contains('é'));
    }
}
