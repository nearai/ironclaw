//! Route-independent WebUI inbound DTO contract.
//!
//! These DTOs normalize authenticated WebUI callers plus browser request bodies
//! into canonical Reborn commands without depending on WebUI route handlers,
//! product adapters, protocol auth evidence, WASM, or adapter registries.

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    CancelRunRequest, GateRef, IdempotencyKey, SanitizedCancelReason, TurnActor, TurnRunId,
    TurnScope,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CLIENT_ACTION_ID_MAX_BYTES: usize = 256;
const USER_MESSAGE_TEXT_MAX_BYTES: usize = 64 * 1024;
const GATE_REF_MAX_BYTES: usize = 256;
const CREDENTIAL_REF_MAX_BYTES: usize = 512;

/// Authenticated WebUI caller after route auth has already completed.
///
/// This is authority-bearing input supplied by the host/router layer, not by
/// the browser body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebUiAuthenticatedCaller {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

impl WebUiAuthenticatedCaller {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
        }
    }

    pub fn actor(&self) -> TurnActor {
        TurnActor::new(self.user_id.clone())
    }

    pub fn turn_scope(&self, thread_id: ThreadId) -> TurnScope {
        TurnScope::new(
            self.tenant_id.clone(),
            self.agent_id.clone(),
            self.project_id.clone(),
            thread_id,
        )
    }
}

/// Browser body for WebUI create-thread mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiCreateThreadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_thread_id: Option<String>,
}

/// Browser body for WebUI send-message mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiSendMessageRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Browser body for WebUI cancel-run mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiCancelRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Browser query for WebUI list-threads read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiListThreadsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Browser body for WebUI extension-setup interaction.
///
/// This is the v2 entrypoint inventory's "extensions onboarding" row.
/// The native facade exposes the route surface so callers can
/// inventory the API without v1 dependency, but the underlying
/// onboarding controller remains v1 today — concrete impl returns
/// `RebornSetupExtensionStatus::NotImplemented` until a v2-aware
/// extension lifecycle lands.
///
/// `extension_name` is not part of the body — it is bound from the
/// route path as an [`ironclaw_common::ExtensionName`] and threaded
/// through the facade as a typed parameter. The handler/facade
/// boundary validates the path segment so a malformed identifier
/// never crosses into facade-internal request/response state as a
/// raw `String`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiSetupExtensionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Browser body for WebUI gate-resolution mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiResolveGateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    /// Attested-signing proof family for `resolution = "attested"`: one of
    /// `injected_wallet`, `near_redirect`, `wallet_connect`. Mirrors the legacy
    /// monolith `GateResolutionPayload` proof variants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attested_proof_kind: Option<String>,
    /// Lowercase-hex of the approved-tx hash the wallet attests to. Carried as
    /// the untrusted `AttestationClaimRef` on the resume; the authoritative
    /// binding (persisted on gate raise) is what the proof is verified against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attested_approved_tx_hash: Option<String>,
    /// Opaque, provider-specific proof payload (signature, signer, scheme,
    /// public key, scope, state echo, …). Re-decoded by the composition-layer
    /// continuation port; never interpreted by this facade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attested_proof: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebUiCancelReason {
    UserRequested,
    Superseded,
    Timeout,
    OperatorRequested,
    Policy,
}

impl From<WebUiCancelReason> for SanitizedCancelReason {
    fn from(value: WebUiCancelReason) -> Self {
        match value {
            WebUiCancelReason::UserRequested => Self::UserRequested,
            WebUiCancelReason::Superseded => Self::Superseded,
            WebUiCancelReason::Timeout => Self::Timeout,
            WebUiCancelReason::OperatorRequested => Self::OperatorRequested,
            WebUiCancelReason::Policy => Self::Policy,
        }
    }
}

// `Attested` carries an opaque `serde_json::Value` proof payload, which is
// `PartialEq` but not `Eq`; the enum therefore drops the `Eq`/`Hash` derives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum WebUiGateResolution {
    Approved {
        #[serde(default)]
        always: bool,
    },
    Denied,
    /// A host-stored credential reference, not a raw secret/token.
    CredentialProvided {
        credential_ref: String,
    },
    /// An external-wallet / custodial attested-signing proof for a
    /// `BlockedAttested` gate. Carries the opaque proof claim the facade
    /// forwards to the injected `AttestedGateContinuationPort` after the resume
    /// transitions the turn to `AttestedResolved`. The fields are
    /// validated-shape strings/JSON only — no trust is conferred here.
    Attested {
        kind: crate::AttestedProofKind,
        approved_tx_hash_hex: String,
        proof_json: serde_json::Value,
    },
    Cancelled,
}

/// Canonical route-independent WebUI command produced after validation.
// `ResolveGate` embeds `WebUiGateResolution`, whose `Attested` variant carries
// a non-`Eq` proof payload, so this enum drops `Eq` as well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WebUiInboundCommand {
    CreateThread {
        caller: WebUiAuthenticatedCaller,
        client_action_id: IdempotencyKey,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_thread_id: Option<ThreadId>,
    },
    SendMessage {
        scope: TurnScope,
        actor: TurnActor,
        client_action_id: IdempotencyKey,
        content: String,
    },
    CancelRun {
        request: CancelRunRequest,
    },
    ResolveGate {
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: GateRef,
        client_action_id: IdempotencyKey,
        resolution: WebUiGateResolution,
    },
}

impl WebUiCreateThreadRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let requested_thread_id = self
            .requested_thread_id
            .map(|value| parse_thread_id_value("requested_thread_id", value))
            .transpose()?;

        Ok(WebUiInboundCommand::CreateThread {
            caller,
            client_action_id,
            requested_thread_id,
        })
    }
}

impl WebUiSendMessageRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let content = required_text(
            "content",
            self.content,
            USER_MESSAGE_TEXT_MAX_BYTES,
            TextMode::MessageContent,
        )?;

        Ok(WebUiInboundCommand::SendMessage {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            client_action_id,
            content,
        })
    }
}

impl WebUiCancelRunRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;
        let reason = parse_cancel_reason(self.reason)?;

        Ok(WebUiInboundCommand::CancelRun {
            request: CancelRunRequest {
                scope: caller.turn_scope(thread_id),
                actor: caller.actor(),
                run_id,
                reason,
                idempotency_key: client_action_id,
            },
        })
    }
}

impl WebUiResolveGateRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;
        let gate_ref = parse_gate_ref(self.gate_ref)?;
        let resolution = parse_gate_resolution(
            self.resolution,
            self.always,
            self.credential_ref,
            self.attested_proof_kind,
            self.attested_approved_tx_hash,
            self.attested_proof,
        )?;

        Ok(WebUiInboundCommand::ResolveGate {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            run_id,
            gate_ref,
            client_action_id,
            resolution,
        })
    }
}

/// Stable validation error code for WebUI inbound DTOs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebUiInboundValidationCode {
    MissingField,
    Blank,
    TooLong,
    InvalidControlCharacter,
    InvalidId,
    InvalidValue,
}

/// Stable validation error shape for WebUI clients and facade tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("invalid WebUI inbound field {field}: {code:?}")]
pub struct WebUiInboundValidationError {
    pub field: String,
    pub code: WebUiInboundValidationCode,
}

impl WebUiInboundValidationError {
    pub fn new(field: &'static str, code: WebUiInboundValidationCode) -> Self {
        Self {
            field: field.to_string(),
            code,
        }
    }
}

fn parse_client_action_id(
    value: Option<String>,
) -> Result<IdempotencyKey, WebUiInboundValidationError> {
    let value = required_text(
        "client_action_id",
        value,
        CLIENT_ACTION_ID_MAX_BYTES,
        TextMode::Token,
    )?;
    IdempotencyKey::new(value).map_err(|_| {
        WebUiInboundValidationError::new("client_action_id", WebUiInboundValidationCode::InvalidId)
    })
}

fn parse_thread_id(value: Option<String>) -> Result<ThreadId, WebUiInboundValidationError> {
    let value = required_text("thread_id", value, 256, TextMode::Token)?;
    parse_thread_id_value("thread_id", value)
}

fn parse_thread_id_value(
    field: &'static str,
    value: String,
) -> Result<ThreadId, WebUiInboundValidationError> {
    ThreadId::new(value)
        .map_err(|_| WebUiInboundValidationError::new(field, WebUiInboundValidationCode::InvalidId))
}

fn parse_run_id(value: Option<String>) -> Result<TurnRunId, WebUiInboundValidationError> {
    let value = required_text("run_id", value, 64, TextMode::Token)?;
    Uuid::parse_str(&value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| {
            WebUiInboundValidationError::new("run_id", WebUiInboundValidationCode::InvalidId)
        })
}

fn parse_gate_ref(value: Option<String>) -> Result<GateRef, WebUiInboundValidationError> {
    let value = required_text("gate_ref", value, GATE_REF_MAX_BYTES, TextMode::Token)?;
    GateRef::new(value).map_err(|_| {
        WebUiInboundValidationError::new("gate_ref", WebUiInboundValidationCode::InvalidId)
    })
}

fn parse_cancel_reason(
    value: Option<String>,
) -> Result<SanitizedCancelReason, WebUiInboundValidationError> {
    let Some(value) = value else {
        return Ok(SanitizedCancelReason::UserRequested);
    };
    validate_text_value("reason", &value, 64, TextMode::Token)?;
    match value.as_str() {
        "user_requested" => Ok(SanitizedCancelReason::UserRequested),
        "superseded" => Ok(SanitizedCancelReason::Superseded),
        "timeout" => Ok(SanitizedCancelReason::Timeout),
        "operator_requested" => Ok(SanitizedCancelReason::OperatorRequested),
        "policy" => Ok(SanitizedCancelReason::Policy),
        _ => Err(WebUiInboundValidationError::new(
            "reason",
            WebUiInboundValidationCode::InvalidValue,
        )),
    }
}

fn parse_gate_resolution(
    resolution: Option<String>,
    always: Option<bool>,
    credential_ref: Option<String>,
    attested_proof_kind: Option<String>,
    attested_approved_tx_hash: Option<String>,
    attested_proof: Option<serde_json::Value>,
) -> Result<WebUiGateResolution, WebUiInboundValidationError> {
    let resolution = required_text("resolution", resolution, 64, TextMode::Token)?;
    match resolution.as_str() {
        "approved" => Ok(WebUiGateResolution::Approved {
            always: always.unwrap_or(false),
        }),
        "denied" => Ok(WebUiGateResolution::Denied),
        "credential_provided" => Ok(WebUiGateResolution::CredentialProvided {
            credential_ref: required_text(
                "credential_ref",
                credential_ref,
                CREDENTIAL_REF_MAX_BYTES,
                TextMode::Token,
            )?,
        }),
        "attested" => parse_attested_resolution(
            attested_proof_kind,
            attested_approved_tx_hash,
            attested_proof,
        ),
        "cancelled" => Ok(WebUiGateResolution::Cancelled),
        _ => Err(WebUiInboundValidationError::new(
            "resolution",
            WebUiInboundValidationCode::InvalidValue,
        )),
    }
}

/// Maximum byte length of the lowercase-hex approved-tx-hash claim. A 32-byte
/// hash is 64 hex chars; allow the optional `0x` prefix the browser may send.
const ATTESTED_HASH_HEX_MAX_BYTES: usize = 66;

fn parse_attested_resolution(
    attested_proof_kind: Option<String>,
    attested_approved_tx_hash: Option<String>,
    attested_proof: Option<serde_json::Value>,
) -> Result<WebUiGateResolution, WebUiInboundValidationError> {
    let kind_text = required_text(
        "attested_proof_kind",
        attested_proof_kind,
        64,
        TextMode::Token,
    )?;
    let kind = match kind_text.as_str() {
        "injected_wallet" => crate::AttestedProofKind::InjectedWallet,
        "near_redirect" => crate::AttestedProofKind::NearRedirect,
        "wallet_connect" => crate::AttestedProofKind::WalletConnect,
        _ => {
            return Err(WebUiInboundValidationError::new(
                "attested_proof_kind",
                WebUiInboundValidationCode::InvalidValue,
            ));
        }
    };
    let approved_tx_hash_raw = required_text(
        "attested_approved_tx_hash",
        attested_approved_tx_hash,
        ATTESTED_HASH_HEX_MAX_BYTES,
        TextMode::Token,
    )?;
    // Canonicalize to the port's `bound_hex` form: strip the optional `0x`
    // prefix we explicitly tolerate, require exactly 64 ASCII-hex digits, and
    // lowercase. Without this, a documented `0x`-prefixed (or uppercase) hash
    // would be carried verbatim into the `AttestationClaimRef` and then fail the
    // resume port's byte-exact comparison against the canonical bound hash —
    // rejecting otherwise-valid proofs.
    let approved_tx_hash_hex = parse_approved_tx_hash_hex(&approved_tx_hash_raw)?;
    let proof_json = attested_proof.ok_or_else(|| {
        WebUiInboundValidationError::new("attested_proof", WebUiInboundValidationCode::MissingField)
    })?;
    if !proof_json.is_object() {
        return Err(WebUiInboundValidationError::new(
            "attested_proof",
            WebUiInboundValidationCode::InvalidValue,
        ));
    }
    Ok(WebUiGateResolution::Attested {
        kind,
        approved_tx_hash_hex,
        proof_json,
    })
}

/// Canonicalize the attested approved-tx-hash hex to the form the resume port
/// compares against (lowercase, no `0x`, exactly 64 hex digits = 32 bytes).
/// Fail-closed on anything that is not a well-formed 32-byte hex hash.
fn parse_approved_tx_hash_hex(value: &str) -> Result<String, WebUiInboundValidationError> {
    let stripped = value.strip_prefix("0x").unwrap_or(value);
    if stripped.len() != 64 || !stripped.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WebUiInboundValidationError::new(
            "attested_approved_tx_hash",
            WebUiInboundValidationCode::InvalidValue,
        ));
    }
    Ok(stripped.to_ascii_lowercase())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMode {
    Token,
    MessageContent,
}

fn required_text(
    field: &'static str,
    value: Option<String>,
    max_bytes: usize,
    mode: TextMode,
) -> Result<String, WebUiInboundValidationError> {
    let value = value.ok_or_else(|| {
        WebUiInboundValidationError::new(field, WebUiInboundValidationCode::MissingField)
    })?;
    validate_text_value(field, &value, max_bytes, mode)?;
    Ok(value)
}

fn validate_text_value(
    field: &'static str,
    value: &str,
    max_bytes: usize,
    mode: TextMode,
) -> Result<(), WebUiInboundValidationError> {
    if value.trim().is_empty() {
        return Err(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::Blank,
        ));
    }
    if value.len() > max_bytes {
        return Err(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::TooLong,
        ));
    }
    let has_invalid_control = value.chars().any(|c| match mode {
        TextMode::Token => c == '\0' || c.is_control(),
        TextMode::MessageContent => c == '\0' || (c.is_control() && c != '\n' && c != '\t'),
    });
    if has_invalid_control {
        return Err(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::InvalidControlCharacter,
        ));
    }
    Ok(())
}
