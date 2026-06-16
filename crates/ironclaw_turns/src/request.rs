use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    AcceptedMessageRef, GateRef, IdempotencyKey, ProductTurnContext, ReplyTargetBindingRef,
    RunProfileRequest, SanitizedCancelReason, SourceBindingRef, TurnActor, TurnRunId, TurnScope,
    TurnStatus,
};

pub type TurnTimestamp = DateTime<Utc>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthResumeDisposition {
    /// The user explicitly declined the auth gate. The executor surfaces this to
    /// the model as an authorization failure rather than re-dispatching the gate.
    ///
    /// New variants (e.g. `Deferred`) may be added here as needs arise.
    Denied,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeTurnPrecondition {
    // Default preserves the historical WebUI resume behavior. Auth
    // continuations opt into BlockedAuthGate explicitly.
    #[default]
    AnyBlockedGate,
    BlockedApprovalGate,
    BlockedAuthGate,
    BlockedResourceGate,
    BlockedDependentRunGate,
}

impl ResumeTurnPrecondition {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::AnyBlockedGate)
    }

    pub fn required_status(&self) -> Option<TurnStatus> {
        match self {
            Self::AnyBlockedGate => None,
            Self::BlockedApprovalGate => Some(TurnStatus::BlockedApproval),
            Self::BlockedAuthGate => Some(TurnStatus::BlockedAuth),
            Self::BlockedResourceGate => Some(TurnStatus::BlockedResource),
            Self::BlockedDependentRunGate => Some(TurnStatus::BlockedDependentRun),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitTurnRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub requested_run_profile: Option<RunProfileRequest>,
    pub idempotency_key: IdempotencyKey,
    pub received_at: TurnTimestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_run_id: Option<TurnRunId>,
    /// Persisted lineage fields for compatibility with stored turn requests.
    /// New child-run callers should use `SubmitChildRunRequest` so the store
    /// derives these fields atomically with spawn-tree reservation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<TurnRunId>,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_tree_root_run_id: Option<TurnRunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
}

/// Request shape for callers that are creating a child run from an existing
/// parent. The coordinator derives the persisted lineage fields on
/// `SubmitTurnRequest` from the parent record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitChildRunRequest {
    pub parent_scope: TurnScope,
    pub parent_run_id: TurnRunId,
    pub child_scope: TurnScope,
    pub actor: TurnActor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub requested_run_profile: Option<RunProfileRequest>,
    pub idempotency_key: IdempotencyKey,
    pub received_at: TurnTimestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_run_id: Option<TurnRunId>,
    pub spawn_tree_descendant_cap: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeTurnRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub gate_resolution_ref: GateRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub idempotency_key: IdempotencyKey,
    #[serde(default, skip_serializing_if = "ResumeTurnPrecondition::is_default")]
    pub precondition: ResumeTurnPrecondition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_resume_disposition: Option<AuthResumeDisposition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRunRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub reason: SanitizedCancelReason,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRunStateRequest {
    pub scope: TurnScope,
    pub run_id: TurnRunId,
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{TenantId, ThreadId, UserId};

    use super::*;

    fn make_resume_request(disposition: Option<AuthResumeDisposition>) -> ResumeTurnRequest {
        ResumeTurnRequest {
            scope: TurnScope {
                tenant_id: TenantId::from_trusted("tenant:test".to_string()),
                agent_id: None,
                project_id: None,
                thread_id: ThreadId::from_trusted("thread:test".to_string()),
                thread_owner: Default::default(),
            },
            actor: TurnActor::new(UserId::from_trusted("user:test".to_string())),
            run_id: TurnRunId::new(),
            gate_resolution_ref: GateRef::new("gate:test-gate").expect("valid gate ref"),
            source_binding_ref: SourceBindingRef::new("source-binding").expect("valid source ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-target")
                .expect("valid reply target ref"),
            idempotency_key: IdempotencyKey::new("idempotency-key").expect("valid idempotency key"),
            precondition: ResumeTurnPrecondition::default(),
            auth_resume_disposition: disposition,
        }
    }

    #[test]
    fn auth_resume_disposition_denied_round_trips() {
        let disposition = AuthResumeDisposition::Denied;
        let json = serde_json::to_string(&disposition).expect("serialize");
        assert!(json.contains("denied"), "wire value is snake_case: {json}");
        let decoded: AuthResumeDisposition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(disposition, decoded);
    }

    /// Asserts the `#[serde(default, skip_serializing_if = "Option::is_none")]`
    /// contract on `ResumeTurnRequest.auth_resume_disposition`:
    ///
    /// 1. **Backward-compat deserialize**: a JSON object for a full
    ///    `ResumeTurnRequest` that omits the `auth_resume_disposition` key must
    ///    deserialize with `auth_resume_disposition == None`.
    /// 2. **Absent-field serialize**: when `auth_resume_disposition` is `None`,
    ///    serializing the struct must NOT emit the `auth_resume_disposition` key.
    /// 3. **Some round-trip**: when `auth_resume_disposition` is `Some(Denied)`,
    ///    the key is present in serialized JSON as the snake_case string `"denied"`
    ///    and round-trips correctly.
    #[test]
    fn resume_turn_request_auth_resume_disposition_serde_contract() {
        // --- 1. Backward-compat: omitted key → None ---
        let base = make_resume_request(None);
        let mut json_value = serde_json::to_value(&base).expect("serialize base request");
        // Remove the key if it was somehow emitted (it shouldn't be — see assertion 2).
        json_value
            .as_object_mut()
            .expect("object")
            .remove("auth_resume_disposition");
        let deserialized: ResumeTurnRequest = serde_json::from_value(json_value)
            .expect("deserialize without auth_resume_disposition");
        assert_eq!(
            deserialized.auth_resume_disposition, None,
            "omitted auth_resume_disposition key must default to None"
        );

        // --- 2. Absent-field serialize: None → key must not appear ---
        let none_request = make_resume_request(None);
        let none_json = serde_json::to_value(&none_request).expect("serialize None request");
        assert!(
            !none_json
                .as_object()
                .expect("object")
                .contains_key("auth_resume_disposition"),
            "auth_resume_disposition must be absent from JSON when None (skip_serializing_if)"
        );

        // --- 3. Some round-trip: Denied (unit variant) ---
        let disposition = AuthResumeDisposition::Denied;
        let some_request = make_resume_request(Some(disposition.clone()));
        let some_json = serde_json::to_value(&some_request).expect("serialize Some request");
        let disposition_value = some_json
            .as_object()
            .expect("object")
            .get("auth_resume_disposition")
            .expect("auth_resume_disposition key must be present when Some");
        // The enum is a unit variant with rename_all = "snake_case", so Denied → "denied"
        assert_eq!(
            disposition_value,
            &serde_json::Value::String("denied".to_string()),
            "auth_resume_disposition must serialize as snake_case string 'denied': {disposition_value}"
        );
        let roundtrip: ResumeTurnRequest =
            serde_json::from_value(some_json).expect("deserialize Some request");
        assert_eq!(
            roundtrip.auth_resume_disposition,
            Some(disposition),
            "auth_resume_disposition must round-trip correctly"
        );
    }
}
