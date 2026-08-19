//! The nine `Reborn*` wire DTOs that could not follow the family into
//! [`ironclaw_product_contracts::product_wire`] with the WS5 port inversion,
//! plus the `ironclaw_turns` conversions that had to become free functions.
//!
//! `ironclaw_product_contracts` may name `ironclaw_host_api` and
//! `ironclaw_extension_contracts` and nothing else internal (PROPOSAL §6.1.3,
//! pinned by `reborn_dependency_boundaries.rs`). Each type below has a field
//! whose type comes from somewhere else:
//!
//! | Type | Blocking field type |
//! |---|---|
//! | `RebornCreateThreadResponse`, `RebornListThreadsResponse`, `RebornTimelineResponse` | `ironclaw_threads::{SessionThreadRecord, ThreadMessageRecord, SummaryArtifact}` |
//! | `RebornAuthAccount`, `RebornVendorAuthAccounts`, `RebornExtensionInfo`, `RebornExtensionListResponse` | `ironclaw_auth::{AuthAccountState, AuthAccountLastError}` |
//! | `RebornGetRunStateResponse` | `ironclaw_common::llm_costs::RunCost` + `ironclaw_loop_contracts::LoopModelUsage` |
//! | `RebornExecuteProductCommandResponse` | `crate::commands::CommandResultView` (the command grammar §6.9.1 keeps here) |
//!
//! The three `From<ironclaw_turns::…>` conversions are free functions rather
//! than trait impls: their target type now lives in the contracts crate and
//! their source in the kernel, so an impl here would be an orphan.

use chrono::{DateTime, Utc};
use ironclaw_auth::{AuthAccountLastError, AuthAccountState};
use ironclaw_common::llm_costs::RunCost;
use ironclaw_extension_contracts::state::LifecyclePublicState;
use ironclaw_host_api::turn::{
    AcceptedMessageRef, EventCursor, SanitizedFailure, TurnCheckpointId, TurnGateRef, TurnRunId,
    TurnStatus,
};
use ironclaw_loop_contracts::LoopModelUsage;
use ironclaw_product_contracts::package_lifecycle::{LifecycleInstallScope, LifecyclePackageRef};
use ironclaw_product_contracts::product_wire::{
    RebornCancelRunResponse, RebornCommandRejection, RebornExtensionOnboardingPayload,
    RebornExtensionSurface, RebornResumeGateResponse, RebornRetryRunResponse,
};
use ironclaw_threads::{SessionThreadRecord, SummaryArtifact, ThreadMessageRecord};
use ironclaw_turns::{CancelRunResponse, ResumeTurnResponse, RetryTurnResponse, TurnRunState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCreateThreadResponse {
    pub thread: SessionThreadRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTimelineResponse {
    pub thread: SessionThreadRecord,
    pub messages: Vec<ThreadMessageRecord>,
    pub summary_artifacts: Vec<SummaryArtifact>,
    /// Opaque cursor to pass back as `cursor` on the follow-up request
    /// to load the older page. `None` means the caller has reached the
    /// start of the thread and there is nothing more to load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Project a kernel cancel-run result onto its product wire shape.
pub fn reborn_cancel_run_response(value: CancelRunResponse) -> RebornCancelRunResponse {
    RebornCancelRunResponse {
        run_id: value.run_id,
        status: value.status,
        event_cursor: value.event_cursor,
        already_terminal: value.already_terminal,
    }
}

/// Project a kernel resume-turn result onto its product wire shape.
pub fn reborn_resume_gate_response(value: ResumeTurnResponse) -> RebornResumeGateResponse {
    RebornResumeGateResponse {
        run_id: value.run_id,
        status: value.status,
        event_cursor: value.event_cursor,
    }
}

/// Project a kernel retry-turn result onto its product wire shape.
pub fn reborn_retry_run_response(value: RetryTurnResponse) -> RebornRetryRunResponse {
    RebornRetryRunResponse {
        run_id: value.run_id,
        status: value.status,
        event_cursor: value.event_cursor,
    }
}

/// Stable run-state projection returned to WebUI route handlers.
///
/// Deliberately omits M3-internal fields carried on [`TurnRunState`]:
/// `scope`, `source_binding_ref`, `reply_target_binding_ref`, and
/// `resolved_model_route`. Route handlers and downstream M5 consumers must
/// build their views from this surface only. Per-run token `usage` and USD
/// `cost` are surfaced (mirroring the OpenAI-compatible API); the resolved
/// model route stays internal — only its model id feeds cost pricing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGetRunStateResponse {
    pub turn_id: String,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub resolved_run_profile_id: String,
    pub resolved_run_profile_version: u64,
    pub received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<TurnCheckpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<TurnGateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SanitizedFailure>,
    /// Cumulative token usage for the run, once the model has reported it.
    /// `None` until the first model response lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LoopModelUsage>,
    /// USD cost priced from `usage` for the run's resolved model. `None` when
    /// usage is absent or no concrete model was resolved (a default-model run
    /// reports usage without cost until the active model is surfaced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<RunCost>,
}

impl RebornGetRunStateResponse {
    /// Build the WebUI run-state projection from canonical run state, pricing
    /// USD `cost` from the run's `model_usage`.
    ///
    /// The model that feeds the cost table is chosen in this order:
    /// 1. the run's `resolved_model_route` model id, when the caller explicitly
    ///    selected/routed a model (only its id crosses; the route stays
    ///    internal);
    /// 2. otherwise `active_model_id` — the runtime's live default model, which
    ///    for a default (unrouted) run is the model that actually ran.
    ///
    /// `cost` stays `None` only when no usage was reported or neither source
    /// yields a concrete model id (the `"default"` alias and empty strings are
    /// treated as non-concrete so a run is never mispriced against a sentinel).
    pub fn from_run_state(value: TurnRunState, active_model_id: Option<&str>) -> Self {
        let priced_model = value
            .resolved_model_route
            .as_ref()
            .map(|route| route.model_id())
            .or(active_model_id)
            .map(str::trim)
            .filter(|model| !model.is_empty() && !model.eq_ignore_ascii_case("default"));
        let cost = match (value.model_usage, priced_model) {
            (Some(usage), Some(model_id)) => Some(RunCost::from_usage(
                model_id,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens,
            )),
            _ => None,
        };
        Self {
            turn_id: value.turn_id.to_string(),
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
            accepted_message_ref: value.accepted_message_ref,
            resolved_run_profile_id: value.resolved_run_profile_id.as_str().to_string(),
            resolved_run_profile_version: value.resolved_run_profile_version.as_u64(),
            received_at: value.received_at,
            checkpoint_id: value.checkpoint_id,
            gate_ref: value.gate_ref,
            // Public WebUI shape: strip the model-visible `detail` so free-form
            // backend cause text never reaches the browser. `category` (the
            // user-facing signal) is retained. See
            // `SanitizedFailure::public_projection`.
            failure: value
                .failure
                .as_ref()
                .map(SanitizedFailure::public_projection),
            usage: value.model_usage,
            cost,
        }
    }
}

impl From<TurnRunState> for RebornGetRunStateResponse {
    /// Convenience conversion with no default-model fallback: a default-model
    /// run (no `resolved_model_route`) reports usage without cost. Callers that
    /// can supply the live active model — the service's `get_run_state` — use
    /// [`RebornGetRunStateResponse::from_run_state`] so those runs are priced.
    fn from(value: TurnRunState) -> Self {
        Self::from_run_state(value, None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListThreadsResponse {
    pub threads: Vec<SessionThreadRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionListResponse {
    pub extensions: Vec<RebornExtensionInfo>,
}

/// A vendor's connected accounts, as modeled on the extensions wire
/// (overview §6.4, `adr/0001-multiple-accounts-per-vendor.md`). One account per
/// vendor per user today; the list shape is frozen so the accepted
/// multi-account follow-up extends behavior without a wire break. The connect
/// card reads `accounts[0]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornVendorAuthAccounts {
    /// The credential-authority vendor id (the provider namespace a recipe
    /// authenticates against).
    pub vendor: String,
    pub accounts: Vec<RebornAuthAccount>,
}

/// One connected account for a vendor. `state` is the shared §6.3 auth-account
/// state machine, exposed exactly — no vendor- or extension-specific state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAuthAccount {
    pub account_id: String,
    /// User-facing label (defaulted from the recipe's identity claim — email,
    /// workspace name — falling back to the extension display name until
    /// identity extraction lands).
    pub label: String,
    pub state: AuthAccountState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<AuthAccountLastError>,
    /// Recipe-ceiling scopes the granted account does not hold (#7660). A
    /// non-empty list on a `connected` account means the vendor recipe
    /// widened after this grant: the card offers a re-consent ("update
    /// access") affordance; it is NOT a broken or unfinished connection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_recipe_scopes: Vec<String>,
    /// Exactly one account per (user, vendor) is the default whenever any
    /// account exists. Always `true` today (list length ≤ 1).
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionInfo {
    pub package_ref: LifecyclePackageRef,
    pub display_name: String,
    /// Runtime implementation name (`wasm` / `mcp` / `first_party` / ...).
    /// Implementation detail — product taxonomy lives in `surfaces`.
    pub runtime: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// The **only** caller-visible lifecycle field (§6.1): exactly
    /// `setup_needed` or `active`; absence from this response means
    /// uninstalled.
    ///
    /// This is the caller-scoped verdict, not the host checkpoint — it folds
    /// the installation phase together with *this* caller's readiness
    /// (required credentials, personal auth, channel pairing/connection) and
    /// any activation failure. An extension whose host record is `Active` is
    /// still `setup_needed` for a caller who has not connected it.
    ///
    /// Internal setup/publication failures stay attached to `setup_needed`
    /// through `activation_error`; they never become a third state. Do not
    /// re-add parallel `needs_setup` / `active` / `authenticated` /
    /// `onboarding_state` booleans — the browser reads this field alone
    /// (`extension-actions.ts::extensionLifecycleState`), and a second source
    /// of truth is what regressed the unpaired-channel card in #6616.
    pub installation_state: LifecyclePublicState,
    /// Redacted internal setup/publication failure; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<RebornExtensionOnboardingPayload>,
    /// Per-vendor connected accounts (§6.4, ADR 0001). Length ≤ 1 today; the
    /// list shape is frozen for the multi-account follow-up. The connect card
    /// reads `accounts[0]`; affordances derive from this state + the
    /// installation state + config completeness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_accounts: Vec<RebornVendorAuthAccounts>,
    /// Declared product surfaces (tool / auth / channel-with-direction).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<RebornExtensionSurface>,
    /// Whether this install is tenant-shared or private to the caller
    /// (#5459 P1); `None` on pre-#5459 payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_scope: Option<LifecycleInstallScope>,
}

/// Generic client-side effect produced by a product command. Keeping this
/// independent of command names lets future commands navigate without the
/// WebUI learning command-specific behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RebornProductCommandEffect {
    OpenThread { thread_id: String },
}

impl RebornProductCommandEffect {
    pub fn open_thread_id(&self) -> Option<&str> {
        match self {
            Self::OpenThread { thread_id } => Some(thread_id.as_str()),
        }
    }
}

/// Response for `product.commands.execute`. Exactly one of `result` /
/// `rejection` is present; `command` names the resolved command token when
/// one could be extracted from `text` (empty when parsing failed before a
/// command name was ever identified).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExecuteProductCommandResponse {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<crate::commands::CommandResultView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<RebornCommandRejection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<RebornProductCommandEffect>,
}
