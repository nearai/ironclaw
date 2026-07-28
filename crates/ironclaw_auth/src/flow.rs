use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ExtensionId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};

use crate::{
    AuthErrorCode, AuthProductError, AuthorizationCodeHash, CredentialAccountId,
    CredentialAccountLabel, LifecyclePackageRef, OpaqueStateHash, ProductActionRef, Timestamp,
    TurnRunRef,
    credential::{CredentialAccountProjection, CredentialAccountStatus, CredentialOwnership},
    ids::{AuthFlowId, AuthGateRef, AuthInteractionId, AuthProviderId, OAuthAuthorizationUrl},
    scope::AuthProductScope,
};

/// Auth flow kind. Identity login is represented for future shared substrate
/// support, but credential-account semantics apply only to integration flows in
/// this first slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthFlowKind {
    IntegrationCredential,
    IdentityLogin,
}

/// Durable auth-flow lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthFlowStatus {
    Pending,
    AwaitingUser,
    CallbackReceived,
    /// Reserved for production stores that split durable claim, provider
    /// exchange, and account mutation across asynchronous workers.
    Completing,
    Completed,
    Failed,
    Expired,
    Canceled,
}

/// Stable recoverable auth challenge rendered by product adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthChallenge {
    OAuthUrl {
        authorization_url: OAuthAuthorizationUrl,
        expires_at: Timestamp,
    },
    ManualTokenRequired {
        interaction_id: AuthInteractionId,
        provider: AuthProviderId,
        label: CredentialAccountLabel,
        expires_at: Timestamp,
    },
    AccountSelectionRequired {
        provider: AuthProviderId,
        accounts: Vec<CredentialAccountProjection>,
    },
    SetupRequired {
        provider: AuthProviderId,
        message: String,
    },
    ReauthorizeRequired {
        account_id: CredentialAccountId,
        provider: AuthProviderId,
    },
}

/// Typed continuation emitted after auth completion. It intentionally stores
/// references only, never raw prompt/message content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthContinuationRef {
    SetupOnly,
    LifecycleActivation {
        package_ref: LifecyclePackageRef,
    },
    TurnGateResume {
        turn_run_ref: TurnRunRef,
        gate_ref: AuthGateRef,
    },
    ProductActionResume {
        action_ref: ProductActionRef,
    },
}

/// Emitted by fake and future production services after an auth flow completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContinuationEvent {
    pub flow_id: AuthFlowId,
    pub scope: AuthProductScope,
    pub continuation: AuthContinuationRef,
    /// Provider of the completed flow, so dispatchers can fan the completion
    /// out to other runs blocked on the same provider's credentials without
    /// re-reading the flow record.
    pub provider: AuthProviderId,
    pub credential_account_id: Option<CredentialAccountId>,
    pub emitted_at: Timestamp,
}

/// Pre-authorized credential update target captured before OAuth completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAccountUpdateBinding {
    pub account_id: CredentialAccountId,
    pub ownership: CredentialOwnership,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_extension: Option<ExtensionId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub granted_extensions: Vec<ExtensionId>,
}

impl CredentialAccountUpdateBinding {
    pub fn from_projection(account: &crate::CredentialAccountProjection) -> Self {
        Self {
            account_id: account.id,
            ownership: account.ownership,
            owner_extension: account.owner_extension.clone(),
            granted_extensions: account.granted_extensions.clone(),
        }
    }
}

/// Durable scoped auth flow record. OAuth state/verifier/code values are
/// represented by hashes only; raw callback material must stay in one-shot
/// provider-client inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFlowRecord {
    pub id: AuthFlowId,
    pub scope: AuthProductScope,
    pub kind: AuthFlowKind,
    pub status: AuthFlowStatus,
    pub provider: AuthProviderId,
    pub challenge: Option<AuthChallenge>,
    pub continuation: AuthContinuationRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_account_id: Option<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_binding: Option<CredentialAccountUpdateBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opaque_state_hash: Option<OpaqueStateHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkce_verifier_hash: Option<crate::PkceVerifierHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_code_hash: Option<AuthorizationCodeHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AuthErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_emitted_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub expires_at: Timestamp,
}

/// Stable owner fields used by read models that project auth flows.
///
/// Invocation id, surface, session, and mission are intentionally excluded:
/// they describe how setup happened, not who owns the blocked auth interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthFlowOwnerScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub thread_id: ThreadId,
}

impl AuthFlowOwnerScope {
    pub fn matches(&self, flow: &AuthFlowRecord) -> bool {
        let resource = &flow.scope.resource;
        resource.tenant_id == self.tenant_id
            && resource.user_id == self.user_id
            && resource.agent_id == self.agent_id
            && resource.project_id == self.project_id
            && resource.mission_id.is_none()
            && resource.thread_id.as_ref() == Some(&self.thread_id)
    }
}

/// Query for one auth flow that backs a blocked turn gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnGateAuthFlowQuery {
    pub owner: AuthFlowOwnerScope,
    pub turn_run_ref: TurnRunRef,
    pub gate_ref: AuthGateRef,
    pub include_terminal: bool,
}

/// Input used to create an auth flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAuthFlow {
    pub id: Option<AuthFlowId>,
    pub scope: AuthProductScope,
    pub kind: AuthFlowKind,
    pub provider: AuthProviderId,
    pub challenge: AuthChallenge,
    pub continuation: AuthContinuationRef,
    pub update_binding: Option<CredentialAccountUpdateBinding>,
    pub opaque_state_hash: Option<OpaqueStateHash>,
    pub pkce_verifier_hash: Option<crate::PkceVerifierHash>,
    pub expires_at: Timestamp,
}

/// Provider callback result after route parsing and provider exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCallbackOutcome {
    Authorized {
        exchange: Box<crate::OAuthProviderExchange>,
    },
    Denied,
}

/// Typed OAuth callback completion input. It carries only state/code hashes and
/// provider-exchange output. Raw code/verifier material belongs in
/// [`crate::OAuthProviderCallbackRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackInput {
    pub flow_id: AuthFlowId,
    pub opaque_state_hash: OpaqueStateHash,
    pub outcome: ProviderCallbackOutcome,
}

/// Terminal failure input for an already-claimed OAuth callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackFailureInput {
    pub flow_id: AuthFlowId,
    pub opaque_state_hash: OpaqueStateHash,
    pub error: AuthErrorCode,
}

/// User-selected configured credential that completes an account-selection
/// auth flow without exposing credential internals to product surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialSelectionInput {
    pub flow_id: AuthFlowId,
    pub credential_account_id: CredentialAccountId,
}

/// User-submitted manual token that completed a manual-token auth flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualTokenCompletionInput {
    pub interaction_id: AuthInteractionId,
    pub credential_account_id: CredentialAccountId,
}

/// Pre-egress claim for an authorized OAuth callback. This validates and marks
/// the scoped flow before one-shot provider exchange can consume a raw code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackClaimRequest {
    pub flow_id: AuthFlowId,
    pub opaque_state_hash: OpaqueStateHash,
    pub provider: AuthProviderId,
    pub pkce_verifier_hash: crate::PkceVerifierHash,
}

#[async_trait]
pub trait AuthFlowManager: Send + Sync {
    /// Mint a new durable auth flow.
    ///
    /// Contract: when the request's continuation is setup-class
    /// ([`is_setup_class_continuation`]), creation itself supersedes — it
    /// cancels every prior non-terminal setup-class flow for the same owner
    /// root + provider before the new flow becomes visible,
    /// so "≤1 live setup-class flow per owner+provider" holds structurally and
    /// no start route can forget it. `TurnGateResume`/`ProductActionResume`
    /// creations supersede nothing and are never superseded: a parked
    /// turn/action must outlive an unrelated setup start.
    async fn create_flow(&self, request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError>;

    async fn get_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError>;

    async fn claim_oauth_callback(
        &self,
        scope: &AuthProductScope,
        request: OAuthCallbackClaimRequest,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    async fn complete_oauth_callback(
        &self,
        scope: &AuthProductScope,
        input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    async fn complete_credential_selection(
        &self,
        scope: &AuthProductScope,
        input: CredentialSelectionInput,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    async fn complete_manual_token(
        &self,
        scope: &AuthProductScope,
        input: ManualTokenCompletionInput,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    async fn cancel_manual_token(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError>;

    async fn fail_oauth_callback(
        &self,
        scope: &AuthProductScope,
        input: OAuthCallbackFailureInput,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    async fn mark_continuation_dispatched(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        emitted_at: Timestamp,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    async fn cancel_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    /// Terminalize a completed OAuth flow whose typed continuation dispatch
    /// failed terminally.
    ///
    /// The honest extension state machine treats a failed lifecycle activation
    /// as terminal: the completed flow must not remain re-dispatchable, so a
    /// `Completed` flow whose continuation has not yet been acknowledged
    /// (`continuation_emitted_at` is `None`) transitions to `Failed` carrying
    /// `error`. A flow that already acknowledged its continuation, or that is
    /// already terminal in another state, returns
    /// [`AuthProductError::FlowAlreadyTerminal`] and is left untouched — the
    /// call is safe to race against a concurrent completion.
    async fn fail_completed_continuation(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        error: AuthErrorCode,
    ) -> Result<AuthFlowRecord, AuthProductError>;
}

/// Whether a continuation belongs to the setup surface — the class a new setup
/// start supersedes. `SetupOnly` is the plain web connect button;
/// `LifecycleActivation` is the extension card's connect button, which
/// `start_setup_oauth_flow` receives verbatim. Both mean "the user is
/// (re-)connecting this provider from a settings surface", so a fresh start
/// replaces them. `TurnGateResume` and `ProductActionResume` have a parked
/// turn/action waiting on them and must outlive an unrelated setup start.
pub fn is_setup_class_continuation(continuation: &AuthContinuationRef) -> bool {
    matches!(
        continuation,
        AuthContinuationRef::SetupOnly | AuthContinuationRef::LifecycleActivation { .. }
    )
}

/// Owner-root match for supersede-on-start: two auth scopes share a setup-flow
/// root iff they carry the same owner (tenant/user/agent/project), surface, and
/// session — the exact granularity of the durable flow-root path, which omits
/// the transient thread/mission/invocation axes. Full scope equality would miss
/// a prior setup flow started under a different per-request invocation.
pub fn flow_shares_setup_owner_root(
    flow_scope: &AuthProductScope,
    scope: &AuthProductScope,
) -> bool {
    let flow_resource = &flow_scope.resource;
    let resource = &scope.resource;
    flow_resource.tenant_id == resource.tenant_id
        && flow_resource.user_id == resource.user_id
        && flow_resource.agent_id == resource.agent_id
        && flow_resource.project_id == resource.project_id
        && flow_scope.surface == scope.surface
        && flow_scope.session_id == scope.session_id
}

/// Read-only auth-flow projection source for product interaction views.
///
/// This is intentionally smaller than [`AuthFlowManager`]: callers can list
/// sanitized flow records for scoped read-model composition, but cannot mutate
/// auth-flow state or bypass manager validation.
#[async_trait]
pub trait AuthFlowRecordSource: Send + Sync {
    async fn flow_for_turn_gate(
        &self,
        query: TurnGateAuthFlowQuery,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError>;

    async fn flows_for_owner(
        &self,
        owner: AuthFlowOwnerScope,
    ) -> Result<Vec<AuthFlowRecord>, AuthProductError>;
}

pub fn flow_matches_turn_gate_query(flow: &AuthFlowRecord, query: &TurnGateAuthFlowQuery) -> bool {
    if !query.include_terminal && crate::is_terminal_status(flow.status) {
        return false;
    }
    if !query.owner.matches(flow) {
        return false;
    }
    matches!(
        &flow.continuation,
        AuthContinuationRef::TurnGateResume {
            turn_run_ref,
            gate_ref,
        } if turn_run_ref == &query.turn_run_ref && gate_ref == &query.gate_ref
    )
}

pub fn credential_status_for_completed_flow() -> CredentialAccountStatus {
    CredentialAccountStatus::Configured
}
