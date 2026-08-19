use async_trait::async_trait;
use ironclaw_extension_contracts::device_link::{
    DeviceLinkMode, DeviceLinkStep, DeviceLinkStepKind,
};
use ironclaw_host_api::ids::{AgentId, ExtensionId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};

use crate::ProviderScope;
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
    /// The flow is waiting on the *vendor*, not the user: a device link that
    /// has displayed its payload and is polling for acceptance sits here.
    ///
    /// Non-terminal, and deliberately distinct from `AwaitingUser` — a card
    /// renders a countdown and a poller instead of an input, and the
    /// [`crate::AuthAccountState`] projection has an explicit arm for it
    /// (a fallthrough would have said `Disconnected` mid-link).
    AwaitingVendor,
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
    /// One frame of a multi-step device link.
    ///
    /// The step itself is the extension's
    /// ([`ironclaw_extension_contracts::device_link::DeviceLinkStep`]); the
    /// `revision` and the two clocks around it are this crate's. A consumer
    /// that advances the flow echoes `revision`, so a stale card cannot
    /// overwrite newer state — see [`AuthFlowManager::advance_flow_step`].
    ///
    /// `mode` is retained because a step re-mint after the *step* clock lapses
    /// has to restart the path the user originally chose, and the card is not
    /// the authority on that.
    DeviceLinkStep {
        extension_id: ExtensionId,
        /// The recipe's own name for the account being linked, resolved once
        /// when the flow starts. Held on the record because a card rendered
        /// from a durable flow must not have to re-resolve a manifest that may
        /// no longer be installed.
        display_name: String,
        /// The recipe's own names for the two declared paths, resolved with
        /// `display_name` when the flow starts and held for the same reason:
        /// a card rendered from a durable flow must not re-resolve a manifest,
        /// and the card is not the authority on which paths exist.
        ///
        /// `alternate_mode_label` absent means the extension declares no
        /// alternate path at all — the recipe's own documented contract — so a
        /// card offers no switch. Additive with `serde(default)`: a flow
        /// persisted before these existed rehydrates with both absent, which
        /// reads as "one path, host-labeled".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_mode_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alternate_mode_label: Option<String>,
        mode: DeviceLinkMode,
        step: DeviceLinkStep,
        revision: u64,
        expires_at: Timestamp,
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

/// Durable state of a multi-step (device-link) flow's *current* step.
///
/// Persisted, so it survives the process that minted it: after a restart the
/// owner of a non-terminal link can still be told a link is outstanding
/// (PROPOSAL §4.3 — an in-memory-only marker can only warn everybody).
///
/// Two clocks live here and they are not the same clock. `step_expires_at` is
/// the **step** clock: when it lapses the step is re-minted and the flow
/// carries on. The flow's own `expires_at` (on [`AuthFlowRecord`]) is the
/// **flow** clock: when that lapses the flow terminalizes, and no re-mint may
/// push it past its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFlowStepState {
    /// How many steps this flow has produced, counting from 1. Monotonic, and
    /// distinct from `revision`: a re-mint of the same logical step advances
    /// both, but only `index` says "this is a different frame".
    pub index: u32,
    /// The shape of the current frame, so a projection can render it without
    /// destructuring the challenge.
    pub kind: DeviceLinkStepKind,
    /// The compare-and-swap token. A writer presents the revision it read; the
    /// store applies the write only if it still matches.
    pub revision: u64,
    /// When the current step stops being valid and must be re-minted.
    pub step_expires_at: Timestamp,
    /// When a poller last asked the vendor. `None` until the first poll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_polled_at: Option<Timestamp>,
    /// Polls served for this flow, across steps. Bounds an unattended card.
    pub poll_attempts: u32,
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
    /// The installed extension whose manifest authorized this OAuth recipe.
    /// Legacy and built-in flows have no requester and resolve only through
    /// the static/bundled path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_extension: Option<ExtensionId>,
    /// The scopes this flow asked the vendor for. Held server-side rather than
    /// round-tripped through the opaque `state` value: a shared-vendor ceiling
    /// is large enough that echoing it in the authorize URL pushed that URL
    /// past its 2048-byte limit. Empty on records written before this field
    /// existed, and on flows that never requested explicit scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_scopes: Vec<ProviderScope>,
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
    /// Current step of a multi-step flow. `None` for every single-shot method
    /// (`oauth2_code`, `api_key`, manual token) and for every record written
    /// before the device-link method existed — hence `#[serde(default)]`; this
    /// is a persisted record and pre-existing rows must still rehydrate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_state: Option<AuthFlowStepState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_emitted_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub expires_at: Timestamp,
}

impl AuthFlowRecord {
    /// The revision a step writer must present. A flow that has not produced a
    /// step yet is at revision `0`, so the first advance is expressible without
    /// a separate "create the step state" call.
    pub fn step_revision(&self) -> u64 {
        self.step_state.map_or(0, |state| state.revision)
    }

    /// The current device-link frame, when this flow is carrying one.
    pub fn device_link_step(&self) -> Option<&DeviceLinkStep> {
        match self.challenge.as_ref() {
            Some(AuthChallenge::DeviceLinkStep { step, .. }) => Some(step),
            _ => None,
        }
    }
}

/// One step transition, presented for compare-and-swap.
///
/// **The CAS is what makes a duplicated poll safe.** Two cards (or a card and
/// a retry) can observe the same revision and both call the vendor; the store
/// applies exactly one. The loser is told so — see
/// [`AuthFlowStepAdvance::applied`] — and must not re-invoke the adapter,
/// because a vendor transition that already ran is not idempotent
/// (PROPOSAL §4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthFlowStepAdvanceInput {
    pub flow_id: AuthFlowId,
    /// The revision the caller read before it invoked the vendor.
    pub expected_revision: u64,
    /// The frame to persist, already validated by its owner.
    pub challenge: AuthChallenge,
    /// The lifecycle status the new frame implies.
    pub status: AuthFlowStatus,
    /// Shape of the new frame.
    pub step_kind: DeviceLinkStepKind,
    /// The **step** clock for the new frame.
    pub step_expires_at: Timestamp,
    /// A new **flow** clock, when the transition extends it. Implementations
    /// must never move a flow's deadline *earlier* through this field, and the
    /// caller is responsible for the cap.
    pub flow_expires_at: Option<Timestamp>,
    /// Set when this transition was produced by a poll, so the store can
    /// account the attempt in one write instead of two.
    pub polled_at: Option<Timestamp>,
    /// Terminal error code, when `status` is terminal.
    pub error: Option<AuthErrorCode>,
    /// The credential account the completed link resolved to. Auth reports
    /// `Completed` only once this is present: custody is durable before the
    /// account exists, and the account exists before completion is reported.
    pub credential_account_id: Option<CredentialAccountId>,
}

/// Result of [`AuthFlowManager::advance_flow_step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthFlowStepAdvance {
    /// The record as it now stands — the caller's write when `applied`, the
    /// winner's when not.
    pub record: AuthFlowRecord,
    /// `false` when the compare-and-swap lost. The call still succeeds: a
    /// duplicated poll is idempotent by contract, and the caller renders the
    /// already-advanced record instead of retrying the vendor.
    pub applied: bool,
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
    pub requester_extension: Option<ExtensionId>,
    pub requested_scopes: Vec<ProviderScope>,
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

    /// Advance a multi-step flow's durable step state under compare-and-swap.
    ///
    /// Contract:
    /// - The write applies iff the stored revision equals
    ///   `input.expected_revision`. On success the revision increments and
    ///   `index` advances.
    /// - **A lost CAS is `Ok`, not an error** — with `applied = false` and the
    ///   winner's record. The loser has already made its vendor call; telling
    ///   it to retry would run a non-idempotent transition twice.
    /// - An already-terminal flow rejects with
    ///   [`AuthProductError::FlowAlreadyTerminal`], so a late step cannot
    ///   resurrect a canceled or completed link.
    /// - `flow_expires_at` may only move the flow deadline later.
    ///
    /// Defaulted so that an [`AuthFlowManager`] with no multi-step method
    /// keeps compiling; the default fails closed. Every implementation that
    /// serves a device-link flow **must** override it — the two production
    /// implementations (the filesystem store and the in-memory fake) do.
    async fn advance_flow_step(
        &self,
        _scope: &AuthProductScope,
        _input: AuthFlowStepAdvanceInput,
    ) -> Result<AuthFlowStepAdvance, AuthProductError> {
        Err(AuthProductError::UnsupportedOperation {
            operation: "advance_flow_step",
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::device_link::{DeviceLinkDisplayKind, DeviceLinkPayload};
    use ironclaw_host_api::{ids::InvocationId, resource::ResourceScope};
    use std::time::Duration as StdDuration;

    fn record() -> AuthFlowRecord {
        let scope = AuthProductScope::new(
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap(),
            crate::AuthSurface::Web,
        );
        AuthFlowRecord {
            id: AuthFlowId::new(),
            scope,
            kind: AuthFlowKind::IntegrationCredential,
            status: AuthFlowStatus::AwaitingVendor,
            provider: AuthProviderId::new("acmevendor").unwrap(),
            requester_extension: Some(ExtensionId::new("acme").unwrap()),
            requested_scopes: Vec::new(),
            challenge: None,
            continuation: AuthContinuationRef::SetupOnly,
            credential_account_id: None,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            authorization_code_hash: None,
            error: None,
            step_state: None,
            continuation_emitted_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now(),
        }
    }

    /// `step_state` is additive on a **persisted** record: a row written before
    /// the device-link method existed carries no such key, and must still
    /// rehydrate rather than failing the whole store open.
    #[test]
    fn a_flow_row_written_without_step_state_still_rehydrates() {
        let mut wire = serde_json::to_value(record()).expect("serialize");
        let object = wire.as_object_mut().expect("record is a JSON object");
        assert!(
            object.remove("step_state").is_none(),
            "a None step_state must not be written at all (skip_serializing_if)"
        );

        let decoded: AuthFlowRecord = serde_json::from_value(wire).expect("legacy row rehydrates");
        assert_eq!(decoded.step_state, None);
        assert_eq!(
            decoded.step_revision(),
            0,
            "a flow with no step state is at revision 0, so the first advance is expressible"
        );
    }

    /// The populated shape round-trips, and the device-link challenge rides
    /// with it — the frame is durable state, not a per-request projection.
    #[test]
    fn a_device_link_step_round_trips_through_the_persisted_record() {
        let mut original = record();
        original.step_state = Some(AuthFlowStepState {
            index: 2,
            kind: DeviceLinkStepKind::Display,
            revision: 7,
            step_expires_at: chrono::Utc::now(),
            last_polled_at: Some(chrono::Utc::now()),
            poll_attempts: 4,
        });
        original.challenge = Some(AuthChallenge::DeviceLinkStep {
            extension_id: ExtensionId::new("acme").unwrap(),
            display_name: "Acme personal account".to_string(),
            default_mode_label: None,
            alternate_mode_label: None,
            mode: DeviceLinkMode::Default,
            step: DeviceLinkStep::Display {
                kind: DeviceLinkDisplayKind::QrCode,
                payload: DeviceLinkPayload::new("tg://login?token=abc").unwrap(),
                expires_in: StdDuration::from_secs(30),
            },
            revision: 7,
            expires_at: chrono::Utc::now(),
        });

        let wire = serde_json::to_string(&original).expect("serialize");
        let decoded: AuthFlowRecord = serde_json::from_str(&wire).expect("deserialize");
        assert_eq!(decoded.step_revision(), 7);
        assert_eq!(decoded.step_state.map(|state| state.poll_attempts), Some(4));
        assert!(matches!(
            decoded.device_link_step(),
            Some(DeviceLinkStep::Display { .. })
        ));
    }
}
