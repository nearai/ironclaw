use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowRecord, AuthFlowState,
    AuthProductScope, CredentialAccountId, CredentialAccountStatus, Timestamp,
};
use ironclaw_product_adapters::ProductWorkflowRejectionKind;
use ironclaw_turns::{CancelRunResponse, GateRef, IdempotencyKey, ResumeTurnResponse, TurnActor};
use ironclaw_turns::{TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

use super::auth_rejected;
use crate::error::ProductWorkflowError;

const FALLBACK_AUTH_SUMMARY: &str = "Authentication required";

/// Stable reject reasons for product auth interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthInteractionRejectionKind {
    MissingAuth,
    /// More than one pending auth gate exists for the same bare-reply
    /// conversation. The user must supply an explicit `auth gate:<ref>` to
    /// disambiguate.
    AmbiguousAuth,
    StaleAuth,
    CrossScopeDenied,
    InvalidGateRef,
    InvalidCredentialRef,
    InvalidCallbackRef,
    UnsupportedResult,
    FlowUnavailable,
    CorruptState,
    InvalidBindingRef,
}

impl AuthInteractionRejectionKind {
    pub fn sanitized_reason(self) -> &'static str {
        match self {
            Self::MissingAuth => "auth interaction was not found",
            Self::AmbiguousAuth => {
                "multiple auth interactions are pending — use auth gate:<ref> to specify which one"
            }
            Self::StaleAuth => "auth interaction is stale",
            Self::CrossScopeDenied => "auth interaction is not visible to this caller",
            Self::InvalidGateRef => "auth gate reference is invalid",
            Self::InvalidCredentialRef => "credential reference is invalid",
            Self::InvalidCallbackRef => "callback reference is invalid",
            Self::UnsupportedResult => "auth interaction result is not supported",
            Self::FlowUnavailable => "auth interaction service is unavailable",
            Self::CorruptState => "auth interaction state is corrupt",
            Self::InvalidBindingRef => "auth resume binding is invalid",
        }
    }

    pub fn workflow_rejection_kind(self) -> ProductWorkflowRejectionKind {
        match self {
            Self::MissingAuth => ProductWorkflowRejectionKind::ScopeNotFound,
            Self::AmbiguousAuth => ProductWorkflowRejectionKind::Ambiguous,
            Self::StaleAuth => ProductWorkflowRejectionKind::Conflict,
            Self::CrossScopeDenied => ProductWorkflowRejectionKind::Unauthorized,
            Self::InvalidGateRef
            | Self::InvalidCredentialRef
            | Self::InvalidCallbackRef
            | Self::UnsupportedResult
            | Self::InvalidBindingRef => ProductWorkflowRejectionKind::InvalidRequest,
            Self::FlowUnavailable => ProductWorkflowRejectionKind::Unavailable,
            Self::CorruptState => ProductWorkflowRejectionKind::Unavailable,
        }
    }

    pub fn status_code(self) -> u16 {
        match self {
            Self::MissingAuth => 404,
            Self::CrossScopeDenied => 403,
            Self::AmbiguousAuth | Self::StaleAuth => 409,
            Self::FlowUnavailable => 503,
            Self::CorruptState => 500,
            Self::InvalidGateRef
            | Self::InvalidCredentialRef
            | Self::InvalidCallbackRef
            | Self::UnsupportedResult
            | Self::InvalidBindingRef => 400,
        }
    }

    pub fn retryable(self) -> bool {
        matches!(self, Self::FlowUnavailable)
    }
}

/// Caller-visible scope for auth interactions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthInteractionScope {
    pub tenant_id: ironclaw_host_api::TenantId,
    pub user_id: ironclaw_host_api::UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<ironclaw_host_api::AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ironclaw_host_api::ProjectId>,
    pub thread_id: ironclaw_host_api::ThreadId,
}

impl AuthInteractionScope {
    pub fn from_turn(scope: &TurnScope, actor: &TurnActor) -> Self {
        let user_id = scope
            .explicit_owner_user_id()
            .cloned()
            .unwrap_or_else(|| actor.user_id.clone());
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id,
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
        }
    }

    fn from_auth_scope(scope: &AuthProductScope) -> Result<Self, ProductWorkflowError> {
        let Some(thread_id) = scope.resource.thread_id.clone() else {
            return Err(auth_rejected(
                AuthInteractionRejectionKind::CrossScopeDenied,
            ));
        };
        Ok(Self {
            tenant_id: scope.resource.tenant_id.clone(),
            user_id: scope.resource.user_id.clone(),
            agent_id: scope.resource.agent_id.clone(),
            project_id: scope.resource.project_id.clone(),
            thread_id,
        })
    }
}

/// Redacted challenge shape safe for product/UI display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthInteractionChallengeView {
    OAuthRedirectRequired {
        expires_at: Timestamp,
    },
    ManualTokenRequired {
        interaction_id: ironclaw_auth::AuthInteractionId,
        provider: ironclaw_auth::AuthProviderId,
        expires_at: Timestamp,
    },
    AccountSelectionRequired {
        provider: ironclaw_auth::AuthProviderId,
        accounts: Vec<AuthCredentialAccountChoiceView>,
    },
    SetupRequired {
        provider: ironclaw_auth::AuthProviderId,
    },
    ReauthorizeRequired {
        account_id: ironclaw_auth::CredentialAccountId,
        provider: ironclaw_auth::AuthProviderId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthCredentialAccountChoiceView {
    pub credential_ref: String,
    pub status: CredentialAccountStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthInteractionStatus {
    Open,
    Processing,
}

impl AuthInteractionStatus {
    fn from_flow_state(state: AuthFlowState) -> Option<Self> {
        match state {
            AuthFlowState::Open => Some(Self::Open),
            AuthFlowState::Processing => Some(Self::Processing),
            AuthFlowState::Resolved(_) => None,
        }
    }
}

impl AuthInteractionChallengeView {
    fn from_challenge(challenge: &AuthChallenge) -> Self {
        match challenge {
            AuthChallenge::OAuthUrl { expires_at, .. } => Self::OAuthRedirectRequired {
                expires_at: *expires_at,
            },
            AuthChallenge::ManualTokenRequired {
                interaction_id,
                provider,
                expires_at,
                ..
            } => Self::ManualTokenRequired {
                interaction_id: *interaction_id,
                provider: provider.clone(),
                expires_at: *expires_at,
            },
            AuthChallenge::AccountSelectionRequired { provider, accounts } => {
                Self::AccountSelectionRequired {
                    provider: provider.clone(),
                    accounts: accounts
                        .iter()
                        .map(|account| AuthCredentialAccountChoiceView {
                            credential_ref: account.id.to_string(),
                            status: account.status,
                        })
                        .collect(),
                }
            }
            AuthChallenge::SetupRequired { provider, .. } => Self::SetupRequired {
                provider: provider.clone(),
            },
            AuthChallenge::ReauthorizeRequired {
                account_id,
                provider,
            } => Self::ReauthorizeRequired {
                account_id: *account_id,
                provider: provider.clone(),
            },
        }
    }
}

/// Product/UI-safe pending auth DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAuthInteractionView {
    pub scope: AuthInteractionScope,
    pub run_id: TurnRunId,
    pub auth_request_ref: GateRef,
    pub flow_id: AuthFlowId,
    pub status: AuthInteractionStatus,
    pub provider: ironclaw_auth::AuthProviderId,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<AuthInteractionChallengeView>,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthGateRecord {
    scope: AuthInteractionScope,
    run_id: TurnRunId,
    gate_ref: GateRef,
    flow: AuthFlowRecord,
}

impl AuthGateRecord {
    pub fn new(
        run_id: TurnRunId,
        gate_ref: GateRef,
        flow: AuthFlowRecord,
    ) -> Result<Self, ProductWorkflowError> {
        let scope = AuthInteractionScope::from_auth_scope(&flow.scope)?;
        let AuthContinuationRef::TurnGateResume {
            turn_run_ref,
            gate_ref: continuation_gate_ref,
        } = &flow.continuation
        else {
            return Err(auth_rejected(
                AuthInteractionRejectionKind::UnsupportedResult,
            ));
        };
        if turn_run_ref.as_str() != run_id.to_string() {
            return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
        }
        let expected_gate = GateRef::new(continuation_gate_ref.as_str().to_string())
            .map_err(|_| auth_rejected(AuthInteractionRejectionKind::InvalidGateRef))?;
        if gate_ref != expected_gate {
            return Err(auth_rejected(AuthInteractionRejectionKind::InvalidGateRef));
        }
        Ok(Self {
            scope,
            run_id,
            gate_ref,
            flow,
        })
    }

    pub fn scope(&self) -> &AuthInteractionScope {
        &self.scope
    }

    pub fn run_id(&self) -> TurnRunId {
        self.run_id
    }

    pub fn gate_ref(&self) -> &GateRef {
        &self.gate_ref
    }

    pub fn flow(&self) -> &AuthFlowRecord {
        &self.flow
    }

    pub fn state(&self) -> AuthFlowState {
        self.flow.state
    }

    pub(super) fn to_view(&self, now: Timestamp) -> Option<PendingAuthInteractionView> {
        // Honor `expires_at` in the projection (RFC 6819 §5.1.5.3). A
        // non-terminal flow whose TTL has passed is not a live interaction, even
        // when a background sweep has not yet transitioned it to `Expired`.
        // Without this, an abandoned "Connect" flow keeps reading as
        // authenticating past its deadline. The durable write paths expire flows
        // lazily; read projections must guard the clock themselves. The
        // boundary is inclusive (`>=`) to match `project_flow_status`'s
        // `expires_at <= now`, so both projections flip at the same instant.
        if now >= self.flow.expires_at {
            return None;
        }
        let status = AuthInteractionStatus::from_flow_state(self.flow.state)?;
        Some(PendingAuthInteractionView {
            scope: self.scope.clone(),
            run_id: self.run_id,
            auth_request_ref: self.gate_ref.clone(),
            flow_id: self.flow.id,
            status,
            provider: self.flow.provider.clone(),
            summary: display_safe_auth_summary(),
            challenge: self
                .flow
                .challenge
                .as_ref()
                .map(AuthInteractionChallengeView::from_challenge),
            expires_at: self.flow.expires_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPendingAuthInteractionsRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPendingAuthInteractionsResponse {
    pub auth_interactions: Vec<PendingAuthInteractionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum AuthInteractionDecision {
    CredentialProvided { credential_ref: CredentialAccountId },
    CallbackCompleted { callback_ref: AuthFlowId },
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveAuthInteractionRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id_hint: Option<TurnRunId>,
    pub gate_ref: GateRef,
    pub decision: AuthInteractionDecision,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveAuthInteractionResponse {
    Resumed(ResumeTurnResponse),
    Canceled(CancelRunResponse),
}

pub(super) fn is_pending_auth_state(state: AuthFlowState) -> bool {
    AuthInteractionStatus::from_flow_state(state).is_some()
}

fn display_safe_auth_summary() -> String {
    FALLBACK_AUTH_SUMMARY.to_string()
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};

    use super::*;

    #[test]
    fn auth_interaction_scope_from_turn_uses_scope_owner_over_actor() {
        let actor = TurnActor::new(UserId::new("user:actor").unwrap());
        let owner_user_id = UserId::new("user:subject").unwrap();
        let thread_id = ThreadId::new("thread:shared").unwrap();
        let scope = TurnScope::new_with_owner(
            TenantId::new("tenant:shared").unwrap(),
            Some(AgentId::new("agent:shared").unwrap()),
            Some(ProjectId::new("project:shared").unwrap()),
            thread_id.clone(),
            Some(owner_user_id.clone()),
        );

        let interaction_scope = AuthInteractionScope::from_turn(&scope, &actor);

        assert_eq!(interaction_scope.user_id, owner_user_id);
        assert_eq!(interaction_scope.thread_id, thread_id);
        assert_eq!(
            interaction_scope.agent_id.as_ref().map(AgentId::as_str),
            Some("agent:shared")
        );
        assert_eq!(
            interaction_scope.project_id.as_ref().map(ProjectId::as_str),
            Some("project:shared")
        );
    }
}
