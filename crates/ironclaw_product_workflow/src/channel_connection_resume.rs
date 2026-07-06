//! Channel-connection gate resume.
//!
//! When a caller activates a connectable inbound channel (Slack, Telegram) and
//! is not yet connected, `extension_activate` parks the turn on the auth-gate
//! rail carrying a `RuntimeCredentialAccountSetup::ChannelPairing { channel }`
//! requirement (`TurnStatus::BlockedAuth`). Unlike an OAuth/manual-token gate,
//! a channel-pairing gate has **no credential_ref and no `AuthFlowRecord`**: it
//! is satisfied out-of-band by redeeming a pairing code that binds the caller's
//! channel identity.
//!
//! This service resumes every run the caller has parked on a channel's pairing
//! gate once that binding exists — pair once, all waiting chats continue. It
//! mirrors the OAuth resume shape (`AuthInteractionService::resume_auth_gate` →
//! `TurnCoordinator::resume_turn` with the `BlockedAuthGate` precondition) but
//! resumes **without a credential_ref** (the credential-less generic resume
//! shape used by `RebornServices::resolve_generic_gate`), because the pairing
//! binding — not a stored credential — is what satisfies the gate.
//!
//! Scope safety: enumeration is strictly bounded to the authenticated caller's
//! `tenant_id` + owner `user_id`. The channel connection binding itself is per
//! `(tenant, user)` (Slack `SlackPersonalBindingPrincipal`), so resuming all of
//! that user's parked runs across their threads/agents/projects is exactly
//! consistent with the connection they just established — and never touches
//! another caller's runs. Failures propagate (no silent `.ok()?` /
//! `unwrap_or_default`) so a backend fault is surfaced, not masked.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_turns::{
    GateRef, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest,
    SourceBindingRef, TurnActor, TurnCoordinator, TurnRunId, TurnScope,
};

use crate::auth_interaction::AuthInteractionRejectionKind;
use crate::error::ProductWorkflowError;

/// Authenticated caller that owns the channel connection and any runs parked on
/// its pairing gate. Deliberately only `tenant_id` + `user_id`: that pair is the
/// security boundary for "belongs to this caller", and the channel connection
/// binding is scoped to exactly that pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConnectionResumeScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

/// One resumable run parked on a channel-pairing auth gate, resolved from
/// durable turn state by the read model. Carries exactly the fields
/// `resume_turn` needs; every one is sourced from the caller-scoped run record,
/// so the service never reconstructs identity by string manipulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPairingBlockedRun {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub gate_ref: GateRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
}

/// Read model that enumerates the caller's runs currently blocked on a
/// channel-pairing auth gate. Implementations MUST restrict the scan to the
/// caller's own `tenant_id` + owner `user_id`; they must never return another
/// caller's runs.
#[async_trait]
pub trait ChannelConnectionResumeReadModel: Send + Sync {
    async fn channel_pairing_blocked_runs(
        &self,
        scope: &ChannelConnectionResumeScope,
        channel: &str,
    ) -> Result<Vec<ChannelPairingBlockedRun>, ProductWorkflowError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeChannelConnectionRequest {
    pub scope: ChannelConnectionResumeScope,
    /// Connectable channel id the pairing satisfied (e.g. `slack`). Matched
    /// case-insensitively against the parked gate's `ChannelPairing` channel.
    pub channel: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeChannelConnectionResponse {
    /// Run ids that transitioned out of the blocked-auth gate. Empty when the
    /// caller had nothing parked on this channel — a valid, non-error outcome.
    pub resumed_runs: Vec<TurnRunId>,
}

/// Channel-agnostic resume for connectable inbound channels. Consumed by the
/// pairing-redeem route after it binds the caller's channel identity.
#[async_trait]
pub trait ChannelConnectionResumeService: Send + Sync {
    async fn resume_channel_connection(
        &self,
        request: ResumeChannelConnectionRequest,
    ) -> Result<ResumeChannelConnectionResponse, ProductWorkflowError>;
}

pub struct DefaultChannelConnectionResumeService {
    read_model: Arc<dyn ChannelConnectionResumeReadModel>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl DefaultChannelConnectionResumeService {
    pub fn new(
        read_model: Arc<dyn ChannelConnectionResumeReadModel>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            read_model,
            turn_coordinator,
        }
    }
}

#[async_trait]
impl ChannelConnectionResumeService for DefaultChannelConnectionResumeService {
    async fn resume_channel_connection(
        &self,
        request: ResumeChannelConnectionRequest,
    ) -> Result<ResumeChannelConnectionResponse, ProductWorkflowError> {
        let channel = normalize_channel(&request.channel);
        let blocked = self
            .read_model
            .channel_pairing_blocked_runs(&request.scope, &channel)
            .await?;
        let mut resumed_runs = Vec::with_capacity(blocked.len());
        for run in blocked {
            // Belt-and-suspenders scope guard: the read model already restricts
            // to the caller, but re-check tenant + owner user here so a future
            // read-model change can never leak a cross-caller resume through this
            // service's `resume_turn` side effect.
            if run.scope.tenant_id != request.scope.tenant_id
                || run.scope.explicit_owner_user_id() != Some(&request.scope.user_id)
            {
                return Err(ProductWorkflowError::AuthInteractionRejected {
                    kind: AuthInteractionRejectionKind::CrossScopeDenied,
                });
            }
            let idempotency_key = resume_idempotency_key(&channel, run.run_id)?;
            let response = self
                .turn_coordinator
                .resume_turn(ResumeTurnRequest {
                    scope: run.scope,
                    actor: run.actor,
                    run_id: run.run_id,
                    gate_resolution_ref: run.gate_ref,
                    source_binding_ref: run.source_binding_ref,
                    reply_target_binding_ref: run.reply_target_binding_ref,
                    idempotency_key,
                    // No credential_ref: a channel-pairing gate is satisfied by
                    // the binding, so resume WITHOUT one — the same shape the
                    // credential-less generic gate resume uses, but pinned to the
                    // BlockedAuth precondition so a stray non-auth run can't be
                    // resumed through this path.
                    precondition: ResumeTurnPrecondition::BlockedAuthGate,
                    resume_disposition: None,
                })
                .await
                .map_err(|error| ProductWorkflowError::TurnResumeDenied { error })?;
            resumed_runs.push(response.run_id);
        }
        Ok(ResumeChannelConnectionResponse { resumed_runs })
    }
}

fn normalize_channel(channel: &str) -> String {
    channel.trim().to_ascii_lowercase()
}

fn resume_idempotency_key(
    channel: &str,
    run_id: TurnRunId,
) -> Result<IdempotencyKey, ProductWorkflowError> {
    // Deterministic per (channel, run) so a repeated redeem replays the same
    // resume idempotently instead of double-queuing the run.
    IdempotencyKey::new(format!("channel-connection-resume:{channel}:{run_id}")).map_err(|reason| {
        ProductWorkflowError::TurnResumeRejected {
            reason: format!("invalid channel-connection resume idempotency key: {reason}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_host_api::{AgentId, ProjectId, ThreadId};
    use ironclaw_turns::{
        AllowAllTurnAdmissionPolicy, DefaultTurnCoordinator, InMemoryRunProfileResolver,
        InMemoryTurnStateStore, ResumeTurnResponse, TurnError, TurnRunWake, TurnRunWakeNotifier,
        TurnRunWakeNotifyError,
    };

    use super::*;

    struct RecordingReadModel {
        runs: Vec<ChannelPairingBlockedRun>,
        seen: Mutex<Vec<(ChannelConnectionResumeScope, String)>>,
    }

    impl RecordingReadModel {
        fn new(runs: Vec<ChannelPairingBlockedRun>) -> Self {
            Self {
                runs,
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ChannelConnectionResumeReadModel for RecordingReadModel {
        async fn channel_pairing_blocked_runs(
            &self,
            scope: &ChannelConnectionResumeScope,
            channel: &str,
        ) -> Result<Vec<ChannelPairingBlockedRun>, ProductWorkflowError> {
            self.seen
                .lock()
                .expect("seen lock")
                .push((scope.clone(), channel.to_string()));
            Ok(self.runs.clone())
        }
    }

    struct SilentWakeNotifier;

    impl TurnRunWakeNotifier for SilentWakeNotifier {
        fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
            Ok(())
        }
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant:resume").expect("tenant")
    }

    fn scope(user: &str, thread: &str) -> TurnScope {
        TurnScope::new_with_owner(
            tenant(),
            Some(AgentId::new("agent:resume").expect("agent")),
            Some(ProjectId::new("project:resume").expect("project")),
            ThreadId::new(thread).expect("thread"),
            Some(UserId::new(user).expect("user")),
        )
    }

    async fn seed_blocked_auth_run(
        store: &Arc<InMemoryTurnStateStore>,
        run_scope: &TurnScope,
        actor: &TurnActor,
        gate_ref: &GateRef,
    ) -> ChannelPairingBlockedRun {
        use ironclaw_turns::runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort};
        use ironclaw_turns::{
            AcceptedMessageRef, BlockedReason, LoopCheckpointStateRef, RunProfileRequest,
            SubmitTurnRequest, SubmitTurnResponse, TurnCheckpointId, TurnLeaseToken, TurnRunnerId,
            TurnStateStore,
        };

        let admission = AllowAllTurnAdmissionPolicy;
        let profiles = InMemoryRunProfileResolver::default();
        let source_binding_ref = SourceBindingRef::new("source:resume").expect("source");
        let reply_target_binding_ref = ReplyTargetBindingRef::new("reply:resume").expect("reply");
        let submit = store
            .submit_turn(
                SubmitTurnRequest {
                    scope: run_scope.clone(),
                    actor: actor.clone(),
                    accepted_message_ref: AcceptedMessageRef::new("message:resume")
                        .expect("message"),
                    source_binding_ref: source_binding_ref.clone(),
                    reply_target_binding_ref: reply_target_binding_ref.clone(),
                    requested_run_profile: Some(
                        RunProfileRequest::new("default").expect("profile"),
                    ),
                    idempotency_key: IdempotencyKey::new(format!(
                        "submit:{}",
                        run_scope.thread_id.as_str()
                    ))
                    .expect("submit key"),
                    received_at: chrono::Utc::now(),
                    requested_run_id: None,
                    parent_run_id: None,
                    subagent_depth: 0,
                    spawn_tree_root_run_id: None,
                    product_context: None,
                },
                &admission,
                &profiles,
            )
            .await
            .expect("submit");
        let SubmitTurnResponse::Accepted { run_id, .. } = submit;
        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        store
            .claim_next_run(ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: Some(run_scope.clone()),
            })
            .await
            .expect("claim")
            .expect("queued run");
        store
            .block_run(BlockRunRequest {
                run_id,
                runner_id,
                lease_token,
                checkpoint_id: TurnCheckpointId::new(),
                state_ref: LoopCheckpointStateRef::new("checkpoint:resume").expect("checkpoint"),
                reason: BlockedReason::Auth {
                    gate_ref: gate_ref.clone(),
                    credential_requirements: Vec::new(),
                },
            })
            .await
            .expect("block");
        ChannelPairingBlockedRun {
            scope: run_scope.clone(),
            actor: actor.clone(),
            run_id,
            gate_ref: gate_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
        }
    }

    #[tokio::test]
    async fn resume_flips_each_supplied_run_and_reports_run_ids() {
        let store = Arc::new(InMemoryTurnStateStore::default());
        let coordinator: Arc<dyn TurnCoordinator> = Arc::new(
            DefaultTurnCoordinator::new(Arc::clone(&store))
                .with_wake_notifier(Arc::new(SilentWakeNotifier)),
        );
        let actor = TurnActor::new(UserId::new("user:alice").expect("user"));
        let gate = GateRef::new("gate:resume-alice").expect("gate");
        let run =
            seed_blocked_auth_run(&store, &scope("user:alice", "thread:a"), &actor, &gate).await;
        let run_id = run.run_id;

        let read_model = Arc::new(RecordingReadModel::new(vec![run]));
        let service = DefaultChannelConnectionResumeService::new(
            read_model.clone(),
            Arc::clone(&coordinator),
        );

        let response = service
            .resume_channel_connection(ResumeChannelConnectionRequest {
                scope: ChannelConnectionResumeScope {
                    tenant_id: tenant(),
                    user_id: UserId::new("user:alice").expect("user"),
                },
                // Alias normalizes to the read-model channel filter input.
                channel: "  SLACK  ".to_string(),
            })
            .await
            .expect("resume");

        assert_eq!(response.resumed_runs, vec![run_id]);
        // The read model was queried with the normalized channel.
        let seen = read_model.seen.lock().expect("seen");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].1, "slack");
    }

    #[tokio::test]
    async fn resume_with_no_parked_runs_is_a_success() {
        let store = Arc::new(InMemoryTurnStateStore::default());
        let coordinator: Arc<dyn TurnCoordinator> = Arc::new(
            DefaultTurnCoordinator::new(store).with_wake_notifier(Arc::new(SilentWakeNotifier)),
        );
        let read_model = Arc::new(RecordingReadModel::new(Vec::new()));
        let service = DefaultChannelConnectionResumeService::new(read_model, coordinator);

        let response = service
            .resume_channel_connection(ResumeChannelConnectionRequest {
                scope: ChannelConnectionResumeScope {
                    tenant_id: tenant(),
                    user_id: UserId::new("user:alice").expect("user"),
                },
                channel: "slack".to_string(),
            })
            .await
            .expect("resume");

        assert!(response.resumed_runs.is_empty());
    }

    #[tokio::test]
    async fn resume_rejects_a_read_model_run_that_escapes_caller_scope() {
        let store = Arc::new(InMemoryTurnStateStore::default());
        let coordinator: Arc<dyn TurnCoordinator> = Arc::new(
            DefaultTurnCoordinator::new(Arc::clone(&store))
                .with_wake_notifier(Arc::new(SilentWakeNotifier)),
        );
        // A misbehaving read model hands back another user's run.
        let actor = TurnActor::new(UserId::new("user:bob").expect("user"));
        let gate = GateRef::new("gate:resume-bob").expect("gate");
        let foreign =
            seed_blocked_auth_run(&store, &scope("user:bob", "thread:b"), &actor, &gate).await;
        let read_model = Arc::new(RecordingReadModel::new(vec![foreign]));
        let service = DefaultChannelConnectionResumeService::new(read_model, coordinator);

        let error = service
            .resume_channel_connection(ResumeChannelConnectionRequest {
                scope: ChannelConnectionResumeScope {
                    tenant_id: tenant(),
                    user_id: UserId::new("user:alice").expect("user"),
                },
                channel: "slack".to_string(),
            })
            .await
            .expect_err("cross-scope run must be rejected");

        assert!(matches!(
            error,
            ProductWorkflowError::AuthInteractionRejected {
                kind: AuthInteractionRejectionKind::CrossScopeDenied
            }
        ));
    }

    // Compile-time guard that resume errors carry the underlying turn error
    // rather than dropping it (error-handling.md: fail loud).
    #[allow(dead_code)]
    fn resume_error_carries_cause(error: TurnError) -> ProductWorkflowError {
        ProductWorkflowError::TurnResumeDenied { error }
    }

    #[allow(dead_code)]
    fn resume_response_type_guard(response: ResumeTurnResponse) -> TurnRunId {
        response.run_id
    }
}
