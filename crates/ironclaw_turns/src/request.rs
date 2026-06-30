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
pub enum GateResumeDisposition {
    /// The user explicitly declined the gate (auth OR approval). The executor
    /// surfaces this to the model as a non-retryable authorization failure rather
    /// than re-dispatching the gate.
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
    BlockedExternalToolGate,
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
            Self::BlockedExternalToolGate => Some(TurnStatus::BlockedExternalTool),
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
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<GateResumeDisposition>,
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

    fn make_resume_request(disposition: Option<GateResumeDisposition>) -> ResumeTurnRequest {
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
            resume_disposition: disposition,
        }
    }

    #[test]
    fn gate_resume_disposition_denied_round_trips() {
        let disposition = GateResumeDisposition::Denied;
        let json = serde_json::to_string(&disposition).expect("serialize");
        assert!(json.contains("denied"), "wire value is snake_case: {json}");
        let decoded: GateResumeDisposition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(disposition, decoded);
    }

    #[test]
    fn external_tool_precondition_maps_to_status_and_round_trips() {
        let precondition = ResumeTurnPrecondition::BlockedExternalToolGate;
        assert_eq!(
            precondition.required_status(),
            Some(TurnStatus::BlockedExternalTool)
        );
        // Wire-stable snake_case contract for the new precondition.
        let json = serde_json::to_string(&precondition).expect("serialize");
        assert_eq!(json, "\"blocked_external_tool_gate\"");
        let decoded: ResumeTurnPrecondition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(precondition, decoded);
    }

    /// Asserts the serde contract on `ResumeTurnRequest.resume_disposition`:
    ///
    /// 1. **Backward-compat deserialize**: a JSON object for a full
    ///    `ResumeTurnRequest` that omits the `auth_resume_disposition` key must
    ///    deserialize with `resume_disposition == None`.
    /// 2. **Absent-field serialize**: when `resume_disposition` is `None`,
    ///    serializing the struct must NOT emit the `auth_resume_disposition` key.
    /// 3. **Some round-trip**: when `resume_disposition` is `Some(Denied)`,
    ///    the key is present in serialized JSON as the snake_case string `"denied"`
    ///    and round-trips correctly.
    #[test]
    fn resume_turn_request_resume_disposition_serde_contract() {
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
            deserialized.resume_disposition, None,
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
        let disposition = GateResumeDisposition::Denied;
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
            "resume_disposition must serialize under the legacy key 'auth_resume_disposition' as 'denied': {disposition_value}"
        );
        let roundtrip: ResumeTurnRequest =
            serde_json::from_value(some_json).expect("deserialize Some request");
        assert_eq!(
            roundtrip.resume_disposition,
            Some(disposition),
            "resume_disposition must round-trip correctly"
        );
    }

    /// Proves that a `ResumeTurnRequest` serialized with the OLD wire key
    /// `"auth_resume_disposition"` (from before the rename to `resume_disposition`)
    /// still deserializes into the new `resume_disposition` field, and that a
    /// missing field defaults to `None`.
    ///
    /// Strategy: round-trip through `make_resume_request` (gives us a valid
    /// struct), then overwrite/inject the key in the JSON map to simulate
    /// what pre-rename code would have written.
    #[test]
    fn resume_disposition_deserializes_legacy_auth_key() {
        // --- Part 1: old key "auth_resume_disposition" lands on resume_disposition ---
        // Start from a valid serialized request (no disposition set).
        let base = make_resume_request(None);
        let mut json_value = serde_json::to_value(&base).expect("serialize base");
        // Inject the old wire key with the "denied" variant value — as pre-rename
        // code would have written it.
        json_value.as_object_mut().expect("object").insert(
            "auth_resume_disposition".to_string(),
            serde_json::json!("denied"),
        );

        let deserialized: ResumeTurnRequest =
            serde_json::from_value(json_value).expect("legacy JSON must deserialize");
        assert_eq!(
            deserialized.resume_disposition,
            Some(GateResumeDisposition::Denied),
            "legacy 'auth_resume_disposition' key must deserialize into resume_disposition"
        );

        // --- Part 2: completely missing field defaults to None ---
        let base_none = make_resume_request(None);
        let mut json_none = serde_json::to_value(&base_none).expect("serialize base_none");
        // Remove the key entirely (it won't be present for None due to skip_serializing_if,
        // but remove defensively in case the serializer ever changes).
        json_none
            .as_object_mut()
            .expect("object")
            .remove("auth_resume_disposition");

        let deserialized_none: ResumeTurnRequest =
            serde_json::from_value(json_none).expect("missing-field JSON must deserialize");
        assert_eq!(
            deserialized_none.resume_disposition, None,
            "missing auth_resume_disposition key must default to None"
        );
    }
}
