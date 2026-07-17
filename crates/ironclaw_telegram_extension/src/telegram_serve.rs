//! Telegram Bot API updates route composition for the Reborn ProductAdapter
//! path.
//!
//! Mirrors `slack_serve` minus the URL-verification handshake (Telegram has
//! no challenge protocol): an axum route fragment plus a manifest-projected
//! ingress descriptor. This module does not bind listeners; the host decides
//! whether to mount the fragment. Verified updates flow through the
//! pairing-aware [`crate::telegram_dispatch::TelegramInboundPreRouter`],
//! which wraps a [`NativeProductAdapterRunner`] for paired-sender traffic.

use std::future::Future;
use std::num::{NonZeroU32, NonZeroUsize};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use ironclaw_host_api::ingress::IngressRouteDescriptor;
use ironclaw_host_api::{NetworkMethod, TenantId};
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

use crate::telegram_actor_identity::TELEGRAM_V2_ADAPTER_ID;
use crate::telegram_dispatch::TelegramInboundPreRouter;
use crate::telegram_pairing::TelegramPairingService;
use crate::telegram_setup::{
    TELEGRAM_UPDATES_ROUTE_PATH, TelegramInstallationSetup, TelegramSetupError,
    TelegramSetupService,
};
use ironclaw_channel_host::identity::RebornUserIdentityLookup;

/// `/webhooks/extensions/telegram/updates` — aliases the setup-pipeline
/// constant so the path `setWebhook` registers and the path this module
/// mounts cannot drift; the descriptor test pins both to the manifest.
/// Production routes come from the manifest projection, so the alias is
/// exercised by parity tests only.
#[cfg_attr(not(test), allow(dead_code))]
pub const TELEGRAM_UPDATES_PATH: &str = TELEGRAM_UPDATES_ROUTE_PATH;
const TELEGRAM_UPDATES_ROUTE_ID: &str = "telegram.updates";

/// The header Telegram sends the `setWebhook` shared secret in
/// (`secret_token`); verified per request before anything else runs.
pub const TELEGRAM_SECRET_TOKEN_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";

const TELEGRAM_BOT_TOKEN_EGRESS_HANDLE: &str = "telegram_bot_token";

/// Mirror of the Slack host-beta runner bounds: the timeout only covers the
/// fast intake half (auth/parse/stamp/submit), never the delivery wait.
const TELEGRAM_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(2);
const TELEGRAM_MAX_IN_FLIGHT_WEBHOOKS: NonZeroUsize = NonZeroUsize::new(64).unwrap(); // safety: 64 is a non-zero literal.

const TELEGRAM_INSTALLATION_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(120).unwrap(); // safety: 120 requests is a non-zero literal.
const TELEGRAM_INSTALLATION_RATE_WINDOW: Duration = Duration::from_secs(60);

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

    // Route internals read the fields directly; the accessors stay for the
    // #6116 fold's shared resolved-installation shape (mirrors Slack's).
    #[allow(dead_code)]
    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    #[allow(dead_code)]
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

/// Resolution outcome for a verified inbound update. Telegram has no
/// URL-verification handshake, so — unlike `ResolvedSlackIngress` — there is
/// exactly one shape: an authenticated event for the resolved installation.
#[derive(Debug, Clone)]
pub struct ResolvedTelegramIngress {
    installation: ResolvedTelegramInstallation,
}

impl ResolvedTelegramIngress {
    pub fn new(installation: ResolvedTelegramInstallation) -> Self {
        Self { installation }
    }

    pub fn installation(&self) -> &ResolvedTelegramInstallation {
        &self.installation
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

pub trait TelegramInstallationResolver: Send + Sync {
    fn resolve_ingress<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<
        Box<dyn Future<Output = Result<ResolvedTelegramIngress, TelegramIngressError>> + Send + 'a>,
    >;

    fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
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

        let built = self.build_installation(setup).await?;
        let mut lifecycle = self.lifecycle.lock().await;
        if let Some(current) = &lifecycle.current
            && current.revision == revision
        {
            return Ok(current.clone());
        }
        if let Some(previous) = lifecycle.current.replace(built.clone()) {
            lifecycle.retire(previous.dispatcher);
        }
        Ok(built)
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
        // `webhook_secret()` re-reads the current setup record; a save racing
        // this build can pair revision N with the N+1 secret for the losing
        // request. Failure mode is a 401 Telegram retries, and the cache
        // re-keys on the next update, so the window self-heals.
        let webhook_secret = self
            .setup_service
            .webhook_secret()
            .await
            .map_err(map_setup_error_to_ingress_unavailable(
                "resolve Telegram webhook secret",
            ))?
            .ok_or(TelegramIngressError::InstallationNotFound)?;
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

impl TelegramInstallationResolver for DynamicTelegramInstallationResolver {
    fn resolve_ingress<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<
        Box<dyn Future<Output = Result<ResolvedTelegramIngress, TelegramIngressError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let live = self.live_installation().await?;
            let evidence = live.dispatcher.verify_webhook_auth(headers, body)?;
            Ok(ResolvedTelegramIngress::new(
                ResolvedTelegramInstallation::new(
                    live.tenant_id,
                    live.adapter_installation_id,
                    evidence,
                    live.dispatcher,
                    live.workflow_observer,
                ),
            ))
        })
    }

    fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.drain_live_installations().await })
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

#[derive(Clone)]
pub struct TelegramIngressService {
    resolver: Arc<dyn TelegramInstallationResolver>,
    installation_rate_limiter: ironclaw_channel_host::host_ingress::InstallationRateLimiter,
}

impl TelegramIngressService {
    pub fn new(resolver: Arc<dyn TelegramInstallationResolver>) -> Self {
        Self::with_rate_limit_config(
            resolver,
            ironclaw_channel_host::host_ingress::InstallationRateLimitConfig::new(
                TELEGRAM_INSTALLATION_MAX_REQUESTS,
                TELEGRAM_INSTALLATION_RATE_WINDOW,
            ),
        )
    }

    pub fn with_rate_limit_config(
        resolver: Arc<dyn TelegramInstallationResolver>,
        rate_limit: ironclaw_channel_host::host_ingress::InstallationRateLimitConfig,
    ) -> Self {
        Self {
            resolver,
            installation_rate_limiter:
                ironclaw_channel_host::host_ingress::InstallationRateLimiter::new(rate_limit),
        }
    }

    async fn handle_updates(&self, headers: HeaderMap, body: Bytes) -> Response {
        let ingress = match self.resolver.resolve_ingress(&headers, body.as_ref()).await {
            Ok(ingress) => ingress,
            Err(error) => return ingress_error_response(error),
        };
        if let Err(exceeded) = self.installation_rate_limiter.check(
            &ingress.installation().tenant_id,
            &ingress.installation().adapter_installation_id,
        ) {
            return ingress_error_response(TelegramIngressError::InstallationRateLimited {
                tenant_id: exceeded.tenant_id,
                adapter_installation_id: exceeded.adapter_installation_id,
            });
        }
        let installation = ingress.installation();
        match installation
            .dispatcher()
            .process_verified_update(
                body.as_ref(),
                installation.evidence(),
                // Revision-scoped: the resolver rebuilds the observer together
                // with the workflow on every setup change, so no static
                // route-state observer exists to fall back to.
                installation.workflow_observer(),
            )
            .await
        {
            Ok(_) => (StatusCode::OK, "ok").into_response(),
            Err(error) => runner_error_response(error),
        }
    }

    pub async fn drain_installations(&self) {
        self.resolver.drain_installations().await;
    }
}

impl std::fmt::Debug for TelegramIngressService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramIngressService")
            .field("resolver", &"Arc<dyn TelegramInstallationResolver>")
            .field("installation_rate_limiter", &self.installation_rate_limiter)
            .finish()
    }
}

/// Route state for the updates webhook. Unlike Slack's `SlackEventsRouteState`
/// there is no static route-level workflow observer: the Telegram observer is
/// rebuilt per setup revision and travels on the resolved installation.
#[derive(Clone)]
pub struct TelegramUpdatesRouteState {
    ingress: TelegramIngressService,
}

impl TelegramUpdatesRouteState {
    pub fn new(ingress: TelegramIngressService) -> Self {
        Self { ingress }
    }

    pub fn from_resolver(resolver: Arc<dyn TelegramInstallationResolver>) -> Self {
        Self::new(TelegramIngressService::new(resolver))
    }

    pub async fn drain_immediate_ack_tasks(&self) {
        self.ingress.drain_installations().await;
    }
}

impl std::fmt::Debug for TelegramUpdatesRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramUpdatesRouteState")
            .field("ingress", &self.ingress)
            .finish()
    }
}

/// Build the raw updates-route fragment: the axum router (state applied) plus
/// the manifest-projected route descriptors. Composition wraps the pair into
/// its public-route mount shape and installs
/// [`TelegramUpdatesRouteState::drain_immediate_ack_tasks`] as the drain —
/// this crate cannot name composition's mount types without a cycle.
pub fn telegram_updates_route_parts(
    state: TelegramUpdatesRouteState,
) -> (Router, Vec<IngressRouteDescriptor>) {
    let descriptor = TELEGRAM_INGRESS_DESCRIPTORS.updates.clone();
    (
        Router::new()
            .route(
                descriptor.route_pattern().as_str(),
                post(telegram_updates_handler),
            )
            .with_state(state),
        vec![descriptor],
    )
}

// Manifest-projection parity seam, pinned by the descriptor test.
#[cfg_attr(not(test), allow(dead_code))]
pub fn telegram_updates_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![TELEGRAM_INGRESS_DESCRIPTORS.updates.clone()]
}

/// The Telegram host-ingress route descriptor, projected from the bundled
/// Telegram channel manifest in a single parse on first use (the manifest is
/// a compile-time constant, so the projection is deterministic and cached for
/// the process lifetime).
///
/// The route's path/method/policy are declared as data in
/// `assets/telegram/manifest.toml` (`[[product_adapter.inbound.host_ingress]]`)
/// and validated by `ironclaw_host_api` plus
/// `ironclaw_product_adapter_registry`; only the declarative descriptor lives
/// in the manifest — the axum handler and the shared-secret verifier stay in
/// this module, and the mount function builds its route from the descriptor
/// so what axum mounts cannot drift from what the manifest declares. Panics
/// if the bundled manifest does not declare the route or declares it with a
/// non-POST method: `TELEGRAM_MANIFEST` is a compile-time constant, so either
/// is a build-time invariant violation, surfaced at startup.
static TELEGRAM_INGRESS_DESCRIPTORS: LazyLock<TelegramIngressDescriptors> = LazyLock::new(|| {
    let descriptors = ironclaw_channel_host::host_ingress::bundled_host_ingress_descriptors(
        crate::telegram_manifest::telegram_manifest_toml(),
    )
    .unwrap_or_else(|error| {
        panic!("bundled Telegram manifest must project host-ingress routes: {error}")
    });
    TelegramIngressDescriptors {
        updates: bundled_telegram_post_descriptor(&descriptors, TELEGRAM_UPDATES_ROUTE_ID),
    }
});

struct TelegramIngressDescriptors {
    updates: IngressRouteDescriptor,
}

fn bundled_telegram_post_descriptor(
    descriptors: &[IngressRouteDescriptor],
    route_id: &str,
) -> IngressRouteDescriptor {
    let descriptor =
        ironclaw_channel_host::host_ingress::descriptor_for_route(descriptors, route_id)
            .unwrap_or_else(|error| {
                panic!(
                    "bundled Telegram manifest must declare host-ingress route {route_id}: {error}"
                )
            });
    // The mount function wires its handler with `post(...)`; fail closed at
    // projection time if the manifest ever declares another method.
    if descriptor.method() != NetworkMethod::Post {
        panic!(
            "bundled Telegram manifest declares host-ingress route {route_id} with method {}, \
             but the serve layer mounts POST handlers",
            descriptor.method()
        );
    }
    descriptor
}

async fn telegram_updates_handler(
    State(state): State<TelegramUpdatesRouteState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.ingress.handle_updates(headers, body).await
}

fn ingress_error_response(error: TelegramIngressError) -> Response {
    match error {
        TelegramIngressError::Runner(error) => runner_error_response(error),
        TelegramIngressError::InstallationNotFound => {
            tracing::debug!(
                target = "ironclaw::reborn::telegram_updates",
                reason = "not_found",
                "Telegram updates installation resolution failed"
            );
            ironclaw_channel_host::host_ingress::webhook_error_response(
                StatusCode::UNAUTHORIZED,
                ironclaw_channel_host::host_ingress::WebhookErrorCategory::Authentication,
            )
        }
        TelegramIngressError::TemporarilyUnavailable => {
            tracing::debug!(
                target = "ironclaw::reborn::telegram_updates",
                reason = "unavailable",
                "Telegram updates installation resolution temporarily unavailable"
            );
            ironclaw_channel_host::host_ingress::webhook_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                ironclaw_channel_host::host_ingress::WebhookErrorCategory::TemporarilyUnavailable,
            )
        }
        TelegramIngressError::InstallationRateLimited {
            tenant_id,
            adapter_installation_id,
        } => {
            tracing::debug!(
                target = "ironclaw::reborn::telegram_updates",
                tenant_id = %tenant_id,
                adapter_installation_id = %adapter_installation_id,
                "Telegram updates installation rate limit exceeded"
            );
            ironclaw_channel_host::host_ingress::webhook_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                ironclaw_channel_host::host_ingress::WebhookErrorCategory::Capacity,
            )
        }
    }
}

fn runner_error_response(error: RunnerError) -> Response {
    let (status, category) = ironclaw_channel_host::host_ingress::runner_error_status(&error);
    tracing::debug!(
        target = "ironclaw::reborn::telegram_updates",
        status = status.as_u16(),
        error = %error,
        "Telegram updates webhook rejected"
    );
    ironclaw_channel_host::host_ingress::webhook_error_response(status, category)
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use ironclaw_host_api::NetworkMethod;
    use ironclaw_host_api::ingress::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
        IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressScopeSource, ListenerClass,
        RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
    };
    use ironclaw_product_adapters::ProtocolAuthFailure;
    use ironclaw_product_adapters::auth::mark_shared_secret_header_verified;
    use secrecy::ExposeSecret;
    use std::num::{NonZeroU32, NonZeroU64};
    use tower::ServiceExt;

    use super::*;
    use crate::telegram_actor_identity::{
        TELEGRAM_IDENTITY_PROVIDER, telegram_user_identity_provider_user_id,
    };
    use crate::telegram_dispatch::test_fixtures::{
        CountingWorkflow, FakeIdentityLookup, RecordingBotApi, configured_setup_service,
        fixture_installation_id, pairing_service_with, private_text_update_body,
        unconfigured_setup_service,
    };
    use crate::telegram_setup::TelegramInstallationSetupUpdate;
    use secrecy::SecretString;

    /// Rebuild the Telegram ingress descriptor as a Rust literal so the
    /// manifest-projected descriptor can be asserted equal to it (the
    /// manifest-driven ingress contract stays real and load-bearing).
    fn expected_telegram_descriptor() -> IngressRouteDescriptor {
        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::PublicWebhook,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::WebhookSignature],
            },
            scope_source: IngressScopeSource::HostResolved,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(1024 * 1024).expect("nonzero"),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::Global,
                max_requests: NonZeroU32::new(12_000).expect("nonzero"),
                window_seconds: NonZeroU32::new(60).expect("nonzero"),
            },
            cors: CorsPolicy::NotApplicable,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::ProductWorkflow,
        })
        .expect("policy validates");
        IngressRouteDescriptor::new(
            TELEGRAM_UPDATES_ROUTE_ID,
            NetworkMethod::Post,
            TELEGRAM_UPDATES_PATH,
            policy,
        )
        .expect("descriptor validates")
    }

    #[derive(Clone)]
    struct FakeTelegramDispatcher {
        verify_result: Result<ProtocolAuthEvidence, RunnerError>,
        dispatch_result: Result<WebhookProcessOutcome, RunnerError>,
        dispatch_calls: Arc<AtomicUsize>,
    }

    impl FakeTelegramDispatcher {
        fn verified() -> Self {
            Self {
                verify_result: Ok(mark_shared_secret_header_verified(
                    TELEGRAM_SECRET_TOKEN_HEADER,
                    "tg-bot-4242",
                )),
                dispatch_result: Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch),
                dispatch_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn auth_failure() -> Self {
            Self {
                verify_result: Err(RunnerError::AuthenticationFailed {
                    failure: ProtocolAuthFailure::Missing,
                }),
                ..Self::verified()
            }
        }

        fn at_capacity() -> Self {
            Self {
                dispatch_result: Err(RunnerError::TooManyInFlight { max_in_flight: 1 }),
                ..Self::verified()
            }
        }

        fn workflow_timeout() -> Self {
            Self {
                dispatch_result: Err(RunnerError::WorkflowTimeout {
                    timeout: Duration::from_secs(1),
                }),
                ..Self::verified()
            }
        }
    }

    impl TelegramUpdatesWebhookDispatcher for FakeTelegramDispatcher {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            self.verify_result.clone()
        }

        fn process_verified_update<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
            _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
            let result = self.dispatch_result.clone();
            Box::pin(async move { result })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    struct FakeTelegramResolver {
        dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher>,
    }

    impl FakeTelegramResolver {
        fn new(dispatcher: Arc<dyn TelegramUpdatesWebhookDispatcher>) -> Self {
            Self { dispatcher }
        }
    }

    impl TelegramInstallationResolver for FakeTelegramResolver {
        fn resolve_ingress<'a>(
            &'a self,
            headers: &'a HeaderMap,
            body: &'a [u8],
        ) -> Pin<
            Box<
                dyn Future<Output = Result<ResolvedTelegramIngress, TelegramIngressError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let evidence = self.dispatcher.verify_webhook_auth(headers, body)?;
                Ok(ResolvedTelegramIngress::new(
                    ResolvedTelegramInstallation::new(
                        TenantId::new("tenant-alpha").expect("valid tenant"),
                        AdapterInstallationId::new("tg-bot-4242").expect("valid installation"),
                        evidence,
                        Arc::clone(&self.dispatcher),
                        None,
                    ),
                ))
            })
        }

        fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.dispatcher.drain_immediate_ack_tasks().await;
            })
        }
    }

    async fn post_to_state(
        state: &TelegramUpdatesRouteState,
        body: String,
        headers: Vec<(&'static str, String)>,
    ) -> Response {
        let (router, _descriptors) = telegram_updates_route_parts(state.clone());
        let mut builder = Request::builder().method("POST").uri(TELEGRAM_UPDATES_PATH);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        router
            .oneshot(
                builder
                    .body(Body::from(body))
                    .expect("request should build"),
            )
            .await
            .expect("router should respond")
    }

    async fn post_with_fake_dispatcher(
        dispatcher: FakeTelegramDispatcher,
        body: &'static str,
    ) -> Response {
        let resolver = Arc::new(FakeTelegramResolver::new(Arc::new(dispatcher)));
        let state = TelegramUpdatesRouteState::from_resolver(resolver);
        post_to_state(&state, body.to_string(), Vec::new()).await
    }

    async fn assert_error_body(response: Response, expected: &str) {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json error body");
        assert_eq!(body["error"], expected);
    }

    /// Observer stub the fake revision builder attaches so the dispatch path
    /// exercises the per-revision `Some(observer)` shape end to end (which
    /// revision handled an update is asserted via the per-installation
    /// counting workflows).
    struct NoopObserver;

    #[async_trait::async_trait]
    impl ImmediateAckWorkflowObserver for NoopObserver {
        async fn observe_workflow_ack(
            &self,
            _envelope: ironclaw_product_adapters::ProductInboundEnvelope,
            _ack: ironclaw_product_adapters::ProductInboundAck,
        ) {
        }
    }

    /// Revision-builder fake: hands out one `CountingWorkflow` PER
    /// INSTALLATION ID so tests can assert which setup revision's workflow
    /// (and observer) handled a given update after a bot swap.
    #[derive(Default)]
    struct FakeRevisionWorkflowBuilder {
        counters: StdMutex<HashMap<String, Arc<AtomicUsize>>>,
        builds: AtomicUsize,
    }

    impl FakeRevisionWorkflowBuilder {
        fn counter_for_installation(&self, installation_id: &str) -> Arc<AtomicUsize> {
            Arc::clone(
                self.counters
                    .lock()
                    .expect("lock")
                    .entry(installation_id.to_string())
                    .or_default(),
            )
        }

        fn builds(&self) -> usize {
            self.builds.load(Ordering::SeqCst)
        }
    }

    impl TelegramRevisionWorkflowBuilder for FakeRevisionWorkflowBuilder {
        fn build_revision_workflow(
            &self,
            setup: &TelegramInstallationSetup,
        ) -> Result<TelegramRevisionWorkflow, TelegramRevisionWorkflowBuildError> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            let installation_id = setup
                .installation_id()
                .map_err(|error| TelegramRevisionWorkflowBuildError::new(error.to_string()))?;
            let counter = self.counter_for_installation(installation_id.as_str());
            Ok(TelegramRevisionWorkflow {
                workflow: Arc::new(CountingWorkflow::new(counter)),
                workflow_observer: Some(Arc::new(NoopObserver)),
            })
        }
    }

    struct DynamicFixture {
        state: TelegramUpdatesRouteState,
        webhook_secret: Option<String>,
        submitted: Arc<AtomicUsize>,
        bot_api: Arc<RecordingBotApi>,
        lookup: Arc<FakeIdentityLookup>,
        setup: Arc<TelegramSetupService>,
        revision_workflows: Arc<FakeRevisionWorkflowBuilder>,
    }

    async fn dynamic_fixture(configured: bool) -> DynamicFixture {
        let bot_api = Arc::new(RecordingBotApi::default());
        let setup = if configured {
            configured_setup_service(bot_api.clone()).await
        } else {
            unconfigured_setup_service(bot_api.clone())
        };
        let webhook_secret = if configured {
            Some(
                setup
                    .webhook_secret()
                    .await
                    .expect("secret read")
                    .expect("secret present")
                    .expose_secret()
                    .to_string(),
            )
        } else {
            None
        };
        let pairing = pairing_service_with(Arc::clone(&setup));
        let lookup = Arc::new(FakeIdentityLookup::default());
        let revision_workflows = Arc::new(FakeRevisionWorkflowBuilder::default());
        // The pre-swap deployment bot's workflow counter (existing tests
        // assert against the fixture bot `tg-bot-4242`).
        let submitted =
            revision_workflows.counter_for_installation(fixture_installation_id().as_str());
        let resolver = Arc::new(DynamicTelegramInstallationResolver::new(
            Arc::clone(&setup),
            pairing,
            lookup.clone() as Arc<dyn RebornUserIdentityLookup>,
            Arc::clone(&revision_workflows) as Arc<dyn TelegramRevisionWorkflowBuilder>,
        ));
        DynamicFixture {
            state: TelegramUpdatesRouteState::from_resolver(resolver),
            webhook_secret,
            submitted,
            bot_api,
            lookup,
            setup,
            revision_workflows,
        }
    }

    async fn current_webhook_secret(setup: &TelegramSetupService) -> String {
        setup
            .webhook_secret()
            .await
            .expect("secret read")
            .expect("secret present")
            .expose_secret()
            .to_string()
    }

    fn bind_paired_sender(
        lookup: &FakeIdentityLookup,
        installation_id: &AdapterInstallationId,
        telegram_user_id: &str,
        user: &str,
    ) {
        lookup.bind(
            TELEGRAM_IDENTITY_PROVIDER,
            &telegram_user_identity_provider_user_id(installation_id, telegram_user_id),
            user,
        );
    }

    #[tokio::test]
    async fn telegram_updates_handler_returns_401_when_unconfigured() {
        let fixture = dynamic_fixture(false).await;
        let response =
            post_to_state(&fixture.state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_error_body(response, "authentication").await;
    }

    /// A setup-store outage is an availability fault, not an authentication
    /// decision: the webhook answers a retryable 503 (Telegram redelivers
    /// once the store recovers). Only `Ok(None)` — genuinely unconfigured —
    /// is the 401 shape.
    #[tokio::test]
    async fn telegram_updates_handler_returns_503_when_setup_store_is_down() {
        #[derive(Debug)]
        struct FailingSetupStore;

        #[async_trait::async_trait]
        impl crate::telegram_setup::TelegramInstallationSetupStore for FailingSetupStore {
            async fn get_telegram_installation_setup(
                &self,
            ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
                Err(TelegramSetupError::StoreUnavailable)
            }

            async fn put_telegram_installation_setup(
                &self,
                _setup: &TelegramInstallationSetup,
            ) -> Result<(), TelegramSetupError> {
                Err(TelegramSetupError::StoreUnavailable)
            }

            async fn delete_telegram_installation_setup(&self) -> Result<(), TelegramSetupError> {
                Err(TelegramSetupError::StoreUnavailable)
            }
        }

        let setup = Arc::new(TelegramSetupService::new(
            TenantId::new("tenant-a").expect("tenant"),
            ironclaw_host_api::AgentId::new("agent-a").expect("agent"),
            None,
            ironclaw_host_api::UserId::new("operator").expect("user"),
            Arc::new(FailingSetupStore),
            Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            Arc::new(RecordingBotApi::default()),
            Some("https://ironclaw.example".to_string()),
        ));
        let pairing = pairing_service_with(Arc::clone(&setup));
        let resolver = Arc::new(DynamicTelegramInstallationResolver::new(
            setup,
            pairing,
            Arc::new(FakeIdentityLookup::default()) as Arc<dyn RebornUserIdentityLookup>,
            Arc::new(FakeRevisionWorkflowBuilder::default())
                as Arc<dyn TelegramRevisionWorkflowBuilder>,
        ));
        let state = TelegramUpdatesRouteState::from_resolver(resolver);

        let response = post_to_state(&state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_error_body(response, "temporarily_unavailable").await;
    }

    #[tokio::test]
    async fn telegram_updates_handler_returns_401_on_missing_secret_header() {
        let fixture = dynamic_fixture(true).await;
        let response =
            post_to_state(&fixture.state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_error_body(response, "authentication").await;
        assert_eq!(fixture.submitted.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn telegram_updates_handler_returns_401_on_wrong_secret_header() {
        let fixture = dynamic_fixture(true).await;
        let response = post_to_state(
            &fixture.state,
            r#"{"update_id":1}"#.to_string(),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, "wrong-secret".to_string())],
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_error_body(response, "authentication").await;
        assert_eq!(fixture.submitted.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn telegram_updates_handler_acks_non_actionable_update_with_valid_secret() {
        let fixture = dynamic_fixture(true).await;
        let secret = fixture.webhook_secret.clone().expect("configured secret");
        let response = post_to_state(
            &fixture.state,
            r#"{"update_id":9}"#.to_string(),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        fixture.state.drain_immediate_ack_tasks().await;
        assert_eq!(fixture.submitted.load(Ordering::SeqCst), 0);
        assert!(
            fixture.bot_api.sends().is_empty(),
            "silently-handled updates must not trigger replies"
        );
    }

    #[tokio::test]
    async fn telegram_updates_handler_forwards_paired_sender_through_native_runner() {
        let fixture = dynamic_fixture(true).await;
        let secret = fixture.webhook_secret.clone().expect("configured secret");
        let installation_id =
            AdapterInstallationId::new("tg-bot-4242").expect("valid installation");
        fixture.lookup.bind(
            TELEGRAM_IDENTITY_PROVIDER,
            &telegram_user_identity_provider_user_id(&installation_id, "42"),
            "ben",
        );

        let body = private_text_update_body(42, 555, Some("hello ironclaw"));
        let response = post_to_state(
            &fixture.state,
            String::from_utf8(body).expect("utf8 body"),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        assert_eq!(&bytes[..], b"ok");
        fixture.state.drain_immediate_ack_tasks().await;
        assert_eq!(
            fixture.submitted.load(Ordering::SeqCst),
            1,
            "paired ordinary text must reach the workflow through the native runner"
        );
        assert!(
            fixture.bot_api.sends().is_empty(),
            "paired traffic must not trigger static replies"
        );
    }

    /// FIX-A regression, first-configure half: the workflow/observer used to
    /// be assembled once at mount-build time from the boot-time setup, so
    /// configuring the bot after boot required a process restart. The
    /// resolver now builds workflow + observer per setup revision: boot
    /// unconfigured (401), save a setup through the setup service, and —
    /// WITHOUT rebuilding the route state — a verified webhook from a paired
    /// sender dispatches into that revision's workflow.
    #[tokio::test]
    async fn telegram_updates_dispatch_after_first_configure_without_rebuild() {
        let fixture = dynamic_fixture(false).await;

        let unconfigured =
            post_to_state(&fixture.state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;
        assert_eq!(unconfigured.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            fixture.revision_workflows.builds(),
            0,
            "no setup record must mean no workflow assembly"
        );

        fixture
            .setup
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("123:abc".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("first configure saves");
        let secret = current_webhook_secret(&fixture.setup).await;
        bind_paired_sender(&fixture.lookup, &fixture_installation_id(), "42", "ben");

        let body = private_text_update_body(42, 555, Some("hello after configure"));
        let response = post_to_state(
            &fixture.state,
            String::from_utf8(body).expect("utf8 body"),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        fixture.state.drain_immediate_ack_tasks().await;
        assert_eq!(
            fixture.submitted.load(Ordering::SeqCst),
            1,
            "first-configure-after-boot must dispatch to the workflow without a rebuild"
        );
        assert_eq!(fixture.revision_workflows.builds(), 1);
    }

    /// FIX-A regression, bot-swap half: rotating the deployment to a
    /// DIFFERENT bot re-keys the webhook verifier AND the workflow/observer
    /// pair. The old installation's secret is rejected, the new secret
    /// parses, and the update dispatches to the NEW revision's workflow
    /// (asserted via per-installation counting workflows).
    #[tokio::test]
    async fn telegram_updates_bot_swap_rekeys_workflow_and_rejects_old_secret() {
        let fixture = dynamic_fixture(true).await;
        let old_secret = fixture.webhook_secret.clone().expect("configured secret");
        bind_paired_sender(&fixture.lookup, &fixture_installation_id(), "42", "ben");

        let before = post_to_state(
            &fixture.state,
            String::from_utf8(private_text_update_body(42, 555, Some("before swap")))
                .expect("utf8 body"),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, old_secret.clone())],
        )
        .await;
        assert_eq!(before.status(), StatusCode::OK);
        fixture.state.drain_immediate_ack_tasks().await;
        assert_eq!(fixture.submitted.load(Ordering::SeqCst), 1);

        // Swap the deployment to a different bot: new installation id
        // `tg-bot-7777`, fresh webhook secret, revision 2.
        fixture.bot_api.set_bot_identity(7777, "other_qa_bot");
        fixture
            .setup
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("777:xyz".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("bot swap saves");
        let new_secret = current_webhook_secret(&fixture.setup).await;
        assert_ne!(
            old_secret, new_secret,
            "rotation must mint a fresh webhook secret"
        );

        let stale = post_to_state(
            &fixture.state,
            String::from_utf8(private_text_update_body(42, 555, Some("stale secret")))
                .expect("utf8 body"),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, old_secret)],
        )
        .await;
        assert_eq!(
            stale.status(),
            StatusCode::UNAUTHORIZED,
            "the old installation's webhook secret must be rejected after the swap"
        );

        let new_installation = AdapterInstallationId::new("tg-bot-7777").expect("valid id");
        bind_paired_sender(&fixture.lookup, &new_installation, "42", "ben");
        let swapped = post_to_state(
            &fixture.state,
            String::from_utf8(private_text_update_body(42, 555, Some("after swap")))
                .expect("utf8 body"),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, new_secret)],
        )
        .await;
        assert_eq!(swapped.status(), StatusCode::OK);
        fixture.state.drain_immediate_ack_tasks().await;

        let new_counter = fixture
            .revision_workflows
            .counter_for_installation(new_installation.as_str());
        assert_eq!(
            new_counter.load(Ordering::SeqCst),
            1,
            "post-swap update must dispatch to the NEW revision's workflow"
        );
        assert_eq!(
            fixture.submitted.load(Ordering::SeqCst),
            1,
            "the old revision's workflow must not receive post-swap updates"
        );
        assert_eq!(
            fixture.revision_workflows.builds(),
            2,
            "one workflow assembly per setup revision"
        );
    }

    #[tokio::test]
    async fn telegram_updates_handler_returns_401_on_auth_failure() {
        let dispatcher = FakeTelegramDispatcher::auth_failure();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_error_body(response, "authentication").await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn telegram_updates_handler_returns_ok_on_successful_dispatch() {
        let dispatcher = FakeTelegramDispatcher::verified();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        assert_eq!(&bytes[..], b"ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn telegram_updates_handler_returns_429_when_at_capacity() {
        let dispatcher = FakeTelegramDispatcher::at_capacity();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_error_body(response, "capacity").await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn telegram_updates_handler_returns_503_on_workflow_timeout() {
        let dispatcher = FakeTelegramDispatcher::workflow_timeout();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_error_body(response, "temporarily_unavailable").await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn telegram_updates_handler_rate_limits_per_installation() {
        let dispatcher = FakeTelegramDispatcher::verified();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let resolver = Arc::new(FakeTelegramResolver::new(Arc::new(dispatcher)));
        let state = TelegramUpdatesRouteState::new(TelegramIngressService::with_rate_limit_config(
            resolver,
            ironclaw_channel_host::host_ingress::InstallationRateLimitConfig::new(
                NonZeroU32::new(1).expect("nonzero"),
                Duration::from_secs(60),
            ),
        ));

        let first = post_to_state(&state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;
        assert_eq!(first.status(), StatusCode::OK);

        let second = post_to_state(&state, r#"{"update_id":2}"#.to_string(), Vec::new()).await;
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_error_body(second, "capacity").await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn telegram_updates_route_descriptor_matches_manifest_projection() {
        assert_eq!(
            telegram_updates_route_descriptors(),
            vec![expected_telegram_descriptor()]
        );
        // The serve-layer path (aliasing the setup pipeline's `setWebhook`
        // path) and the manifest-projected route pattern must be one value.
        assert_eq!(
            TELEGRAM_UPDATES_PATH,
            "/webhooks/extensions/telegram/updates"
        );
        assert_eq!(
            TELEGRAM_INGRESS_DESCRIPTORS
                .updates
                .route_pattern()
                .as_str(),
            TELEGRAM_UPDATES_PATH
        );
    }
}
