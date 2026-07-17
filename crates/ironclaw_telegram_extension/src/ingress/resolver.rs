//! Setup-revision-aware Telegram installation resolution.
//!
//! Verified updates flow through the
//! pairing-aware [`crate::ingress::dispatch::TelegramInboundPreRouter`],
//! which wraps a [`NativeProductAdapterRunner`] for paired-sender traffic.

use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderMap;
use ironclaw_host_api::TenantId;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, EgressCredentialHandle, ProductAdapter,
    ProductAdapterId, ProductWorkflow, ProtocolAuthEvidence,
};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig,
};
use ironclaw_wasm_product_adapters::{
    ImmediateAckWorkflowObserver, NativeProductAdapterRunner, NativeProductAdapterRunnerConfig,
    RunnerError, SharedSecretHeaderAuth, WebhookAuth, WebhookProcessOutcome,
};
use secrecy::ExposeSecret;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::ingress::dispatch::TelegramInboundPreRouter;
use crate::pairing::TelegramPairingService;
use crate::setup::{TelegramInstallationSetup, TelegramSetupError, TelegramSetupService};
use crate::telegram_actor_identity::TELEGRAM_V2_ADAPTER_ID;
use ironclaw_channel_host::identity::RebornUserIdentityLookup;

/// The header Telegram sends the `setWebhook` shared secret in
/// (`secret_token`); verified per request before anything else runs.
pub const TELEGRAM_SECRET_TOKEN_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";

const TELEGRAM_BOT_TOKEN_EGRESS_HANDLE: &str = "telegram_bot_token";

/// Mirror of the Slack host-beta runner bounds: the timeout only covers the
/// fast intake half (auth/parse/stamp/submit), never the delivery wait.
const TELEGRAM_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(2);
const TELEGRAM_MAX_IN_FLIGHT_WEBHOOKS: NonZeroUsize = match NonZeroUsize::new(64) {
    Some(value) => value,
    None => NonZeroUsize::MIN,
};

/// The verified-update dispatch seam between the route handler and the
/// pairing-aware pre-router (which itself wraps the native runner for
/// paired-sender traffic). Mirrors `SlackEventsWebhookDispatcher`.
pub trait TelegramUpdatesWebhookDispatcher: Send + Sync {
    fn verify_webhook_auth(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError>;

    fn process_verified_update<'a>(
        &'a self,
        body: &'a [u8],
        evidence: &'a ProtocolAuthEvidence,
        observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>;

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

impl TelegramUpdatesWebhookDispatcher for NativeProductAdapterRunner {
    fn verify_webhook_auth(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError> {
        NativeProductAdapterRunner::verify_webhook_auth(self, headers, body)
    }

    fn process_verified_update<'a>(
        &'a self,
        body: &'a [u8],
        evidence: &'a ProtocolAuthEvidence,
        observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>> {
        Box::pin(
            NativeProductAdapterRunner::process_verified_webhook_immediate_ack_with_observer(
                self, body, evidence, observer,
            ),
        )
    }

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(NativeProductAdapterRunner::drain_immediate_ack_tasks(self))
    }
}

/// A resolved-and-verified Telegram installation: the deployment bot the
/// request authenticated against, the dispatcher that handles it, plus the
/// setup-revision-scoped delivery observer (mirrors Slack's
/// `ResolvedSlackInstallation`, so a bot swap re-keys the observer's adapter
/// together with the verifier and workflow).
#[derive(Clone)]
pub struct ResolvedTelegramInstallation {
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
    evidence: ProtocolAuthEvidence,
    dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher>,
    workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
}

impl ResolvedTelegramInstallation {
    pub fn new(
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
        evidence: ProtocolAuthEvidence,
        dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher>,
        workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Self {
        Self {
            tenant_id,
            adapter_installation_id,
            evidence,
            dispatcher,
            workflow_observer,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn adapter_installation_id(&self) -> &AdapterInstallationId {
        &self.adapter_installation_id
    }

    pub fn evidence(&self) -> &ProtocolAuthEvidence {
        &self.evidence
    }

    pub fn dispatcher(&self) -> Arc<dyn TelegramUpdatesWebhookDispatcher> {
        Arc::clone(&self.dispatcher)
    }

    pub fn workflow_observer(&self) -> Option<Arc<dyn ImmediateAckWorkflowObserver>> {
        self.workflow_observer.clone()
    }
}

impl std::fmt::Debug for ResolvedTelegramInstallation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedTelegramInstallation")
            .field("tenant_id", &self.tenant_id)
            .field("adapter_installation_id", &self.adapter_installation_id)
            .field("dispatcher", &"Arc<dyn TelegramUpdatesWebhookDispatcher>")
            .field("workflow_observer", &self.workflow_observer.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramIngressError {
    #[error(transparent)]
    Runner(#[from] RunnerError),
    #[error("no configured Telegram installation matched the request")]
    InstallationNotFound,
    /// Host-side availability fault (setup/secret store outage, workflow
    /// build failure) — retryable, so Telegram redelivers once the host
    /// recovers, unlike the authentication-shaped `InstallationNotFound`.
    #[error("Telegram installation resolution temporarily unavailable")]
    TemporarilyUnavailable,
    #[error(
        "Telegram installation rate limit exceeded for tenant {tenant_id} installation {adapter_installation_id}"
    )]
    InstallationRateLimited {
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
    },
}

/// Per-setup-revision workflow assembly: the product workflow the runner
/// submits verified updates into, plus the final-reply delivery observer
/// scoped to the same installation identity.
pub struct TelegramRevisionWorkflow {
    pub workflow: Arc<dyn ProductWorkflow>,
    pub workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("telegram revision workflow build failed: {reason}")]
pub struct TelegramRevisionWorkflowBuildError {
    pub reason: String,
}

impl TelegramRevisionWorkflowBuildError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Builds the workflow + delivery observer for one setup revision. Production
/// (`telegram_host_beta`) assembles the real `DefaultProductWorkflow` /
/// delivery observer from revision-independent runtime parts; serve-tier
/// tests inject counting fakes so per-revision routing stays assertable.
pub trait TelegramRevisionWorkflowBuilder: Send + Sync {
    fn build_revision_workflow(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<TelegramRevisionWorkflow, TelegramRevisionWorkflowBuildError>;
}

/// Setup-store-backed resolver for the single operator-managed deployment
/// bot. Reads [`TelegramSetupService::current_setup`] on every inbound update
/// (so WebUI setup changes take effect on the next webhook) and caches the
/// built verifier/adapter/runner/workflow/observer chain per setup revision.
/// Mirrors `DynamicSlackInstallationResolver`.
#[derive(Clone)]
pub struct DynamicTelegramInstallationResolver {
    setup_service: Arc<TelegramSetupService>,
    pairing: Arc<TelegramPairingService>,
    identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    revision_workflows: Arc<dyn TelegramRevisionWorkflowBuilder>,
    lifecycle: Arc<Mutex<DynamicTelegramDispatcherLifecycle>>,
}

impl DynamicTelegramInstallationResolver {
    /// `revision_workflows` builds the T7-owned `ProductWorkflow` (inbound
    /// turn service + idempotency ledger + conversation binding) plus the
    /// final-reply delivery observer PER SETUP REVISION, so a first configure
    /// or bot swap after boot re-keys the installation scope and observer
    /// adapter without a process restart; this resolver stays self-contained
    /// by taking the builder as a constructor param instead of assembling
    /// workflows from runtime parts here.
    pub fn new(
        setup_service: Arc<TelegramSetupService>,
        pairing: Arc<TelegramPairingService>,
        identity_lookup: Arc<dyn RebornUserIdentityLookup>,
        revision_workflows: Arc<dyn TelegramRevisionWorkflowBuilder>,
    ) -> Self {
        Self {
            setup_service,
            pairing,
            identity_lookup,
            revision_workflows,
            lifecycle: Arc::new(Mutex::new(DynamicTelegramDispatcherLifecycle::default())),
        }
    }

    async fn live_installation(&self) -> Result<LiveTelegramInstallation, TelegramIngressError> {
        // Read setup before consulting the live holder so WebUI changes take
        // effect on the next webhook; the holder exists for dispatcher
        // lifecycle/drain ownership, not to hide setup-store I/O.
        // Only Ok(None) — genuinely unconfigured — is the 401 shape; a store
        // outage stays retryable (503) so Telegram redelivers on recovery.
        for _attempt in 0..4 {
            let setup = self
                .setup_service
                .current_setup()
                .await
                .map_err(map_setup_error_to_ingress_unavailable(
                    "read Telegram setup",
                ))?
                .ok_or(TelegramIngressError::InstallationNotFound)?;
            let revision = setup.revision;
            if let Some(live) = self.live_for_revision(revision).await {
                return Ok(live);
            }

            let built = self.build_installation(setup.clone()).await?;
            // A slow revision-N build must never publish after revision N+1
            // became authoritative. Both the verifier secret and workflow were
            // built from `setup`; confirm that exact snapshot still wins.
            if self
                .setup_service
                .current_setup()
                .await
                .map_err(map_setup_error_to_ingress_unavailable(
                    "confirm Telegram setup revision",
                ))?
                .as_ref()
                != Some(&setup)
            {
                continue;
            }

            let mut lifecycle = self.lifecycle.lock().await;
            if let Some(current) = &lifecycle.current
                && current.revision == revision
            {
                return Ok(current.clone());
            }
            if let Some(previous) = lifecycle.current.replace(built.clone()) {
                lifecycle.retire(previous.dispatcher);
            }
            return Ok(built);
        }
        Err(TelegramIngressError::TemporarilyUnavailable)
    }

    async fn live_for_revision(&self, revision: u64) -> Option<LiveTelegramInstallation> {
        let lifecycle = self.lifecycle.lock().await;
        lifecycle
            .current
            .as_ref()
            .filter(|current| current.revision == revision)
            .cloned()
    }

    async fn build_installation(
        &self,
        setup: TelegramInstallationSetup,
    ) -> Result<LiveTelegramInstallation, TelegramIngressError> {
        let installation_id =
            setup
                .installation_id()
                .map_err(map_setup_error_to_ingress_unavailable(
                    "derive Telegram installation id",
                ))?;
        let webhook_secret = self
            .setup_service
            .webhook_secret_for_setup(&setup)
            .await
            .map_err(map_setup_error_to_ingress_unavailable(
                "resolve Telegram webhook secret",
            ))?;
        let verifier = SharedSecretHeaderAuth {
            header_name: TELEGRAM_SECRET_TOKEN_HEADER.to_string(),
            expected_secret: webhook_secret.expose_secret().to_string(),
            subject: installation_id.as_str().to_string(),
        };
        let adapter_id = ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID).map_err(
            map_build_reason_to_ingress_unavailable("build Telegram adapter id"),
        )?;
        let egress_credential_handle =
            EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_EGRESS_HANDLE).map_err(
                map_build_reason_to_ingress_unavailable("build Telegram bot token handle"),
            )?;
        let adapter: Arc<dyn ProductAdapter> =
            Arc::new(TelegramV2Adapter::new(TelegramV2AdapterConfig {
                adapter_id,
                installation_id: installation_id.clone(),
                group_trigger_policy: GroupTriggerPolicy {
                    bot_username: setup.bot_username.clone(),
                    bot_user_id: setup.bot_id,
                    recognized_commands: vec![],
                },
                egress_credential_handle,
                auth_requirement: AuthRequirement::SharedSecretHeader {
                    header_name: TELEGRAM_SECRET_TOKEN_HEADER.into(),
                },
                progress_push_enabled: false,
            }));
        let revision_workflow = self
            .revision_workflows
            .build_revision_workflow(&setup)
            .map_err(map_build_reason_to_ingress_unavailable(
                "build Telegram revision workflow",
            ))?;
        let runner = Arc::new(NativeProductAdapterRunner::with_config(
            adapter,
            Arc::clone(&revision_workflow.workflow),
            WebhookAuth::SharedSecretHeader(verifier),
            NativeProductAdapterRunnerConfig::new(
                TELEGRAM_WEBHOOK_WORKFLOW_TIMEOUT,
                TELEGRAM_MAX_IN_FLIGHT_WEBHOOKS,
            ),
        ));
        let dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher> =
            Arc::new(TelegramInboundPreRouter::new(
                Arc::clone(&self.pairing),
                Arc::clone(&self.identity_lookup),
                self.setup_service.bot_api(),
                Arc::clone(&self.setup_service),
                installation_id.clone(),
                runner,
            ));
        Ok(LiveTelegramInstallation {
            revision: setup.revision,
            tenant_id: self.setup_service.tenant_id().clone(),
            adapter_installation_id: installation_id,
            dispatcher,
            workflow_observer: revision_workflow.workflow_observer,
        })
    }

    async fn drain_live_installations(&self) {
        let (watermark, dispatchers) = {
            let lifecycle = self.lifecycle.lock().await;
            (lifecycle.retire_seq, lifecycle.dispatchers())
        };
        for dispatcher in &dispatchers {
            dispatcher.drain_immediate_ack_tasks().await;
        }
        let mut lifecycle = self.lifecycle.lock().await;
        lifecycle.forget_retired_before(watermark);
    }
}

impl DynamicTelegramInstallationResolver {
    pub async fn resolve_ingress(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ResolvedTelegramInstallation, TelegramIngressError> {
        let live = self.live_installation().await?;
        let evidence = live.dispatcher.verify_webhook_auth(headers, body)?;
        Ok(ResolvedTelegramInstallation::new(
            live.tenant_id,
            live.adapter_installation_id,
            evidence,
            live.dispatcher,
            live.workflow_observer,
        ))
    }

    pub async fn drain_installations(&self) {
        self.drain_live_installations().await;
    }
}

#[derive(Clone)]
struct LiveTelegramInstallation {
    revision: u64,
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
    dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher>,
    workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
}

/// Dispatcher lifecycle holder: the current revision-keyed dispatcher plus
/// retired ones that may still own in-flight immediate-ack tasks. Retirement
/// entries carry a monotonic sequence so a drain can forget exactly the
/// dispatchers it drained without pointer comparisons on `dyn` handles.
#[derive(Default)]
struct DynamicTelegramDispatcherLifecycle {
    current: Option<LiveTelegramInstallation>,
    retired: Vec<RetiredTelegramDispatcher>,
    retire_seq: u64,
}

struct RetiredTelegramDispatcher {
    seq: u64,
    dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher>,
}

impl DynamicTelegramDispatcherLifecycle {
    fn retire(&mut self, dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher>) {
        let seq = self.retire_seq;
        self.retire_seq = self.retire_seq.saturating_add(1);
        self.retired
            .push(RetiredTelegramDispatcher { seq, dispatcher });
    }

    fn dispatchers(&self) -> Vec<Arc<dyn TelegramUpdatesWebhookDispatcher>> {
        self.current
            .iter()
            .map(|current| Arc::clone(&current.dispatcher))
            .chain(
                self.retired
                    .iter()
                    .map(|retired| Arc::clone(&retired.dispatcher)),
            )
            .collect()
    }

    fn forget_retired_before(&mut self, watermark: u64) {
        self.retired.retain(|retired| retired.seq >= watermark);
    }
}

fn map_setup_error_to_ingress_unavailable(
    context: &'static str,
) -> impl FnOnce(TelegramSetupError) -> TelegramIngressError {
    move |error| {
        tracing::debug!(
            target = "ironclaw::reborn::telegram_updates",
            %error,
            context,
            "Telegram setup unavailable for dynamic ingress"
        );
        TelegramIngressError::TemporarilyUnavailable
    }
}

fn map_build_reason_to_ingress_unavailable<E: std::fmt::Display>(
    context: &'static str,
) -> impl FnOnce(E) -> TelegramIngressError {
    move |error| {
        tracing::debug!(
            target = "ironclaw::reborn::telegram_updates",
            %error,
            context,
            "Telegram installation build failed for dynamic ingress"
        );
        TelegramIngressError::TemporarilyUnavailable
    }
}
