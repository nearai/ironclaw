//! Composition of the generic channel ingress router (extension-runtime P4).
//!
//! Assembly only: this module constructs the [`ExtensionIngressRouter`] over
//! the generic host's snapshot watch, provides the per-extension
//! registration surface concrete channel graphs plug into (secrets + inbound
//! sink), the generic inbound sink over the ProductSurface channel admission
//! (idempotency ledger → identity/conversation binding → turn submission),
//! and — behind the serve feature — the one `PublicRouteMount` that serves
//! `/webhooks/extensions/{extension_id}/{route_suffix}` for every active
//! extension.
//!
//! Route resolution happens per request through the snapshot watch, so
//! activations/removals take effect without any HTTP-server rebuild.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_extension_host::ingress::{
    ExtensionIngressRouter, InboundAdmission, InboundAdmissionAck, InboundSink, InboundSinkError,
    IngressPortError, IngressSecretsPort, VerificationCandidate,
};
use ironclaw_host_api::{SecretHandle, TenantId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ChannelInboundClassification, ExternalConversationRef, ExternalEventId,
    NormalizedInboundMessage, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductSourceChannel, ProtocolAuthEvidence,
};
use ironclaw_product_workflow::{
    ChannelInboundSurfaceOutcome, ChannelInboundSurfaceRejectedAdmission,
    ChannelInboundSurfaceRequest, ProductOperationRequest, ProductSurface,
    WebUiAuthenticatedCaller,
};
use tokio::task::JoinSet;

use crate::extension_host::channel_pairing::ChannelPairingConsumeOutcome;

/// Fixed host route paths inside the extension ingress namespace
/// (`/webhooks/extensions/…`). An extension whose canonical route collides
/// with one of these fails activation (`SnapshotConflict::ReservedRoute`).
///
/// Empty today: no fixed host route lives under the extension namespace, and
/// legacy fixed webhook paths (e.g. the one-release channel aliases outside
/// the namespace) cannot collide with a canonical extension path by
/// construction. Any future fixed mount under `/webhooks/extensions/` MUST
/// be added here in the same change that mounts it.
pub(crate) fn reserved_fixed_ingress_routes() -> BTreeSet<String> {
    BTreeSet::new()
}

// ── Per-extension registration ──────────────────────────────────────────────

/// Post-admission follow-up for one extension's inbound messages (e.g. a
/// delivery observer that pushes the run's final reply back to the vendor).
/// Runs outside the webhook response path; must not assume the vendor can
/// retry.
#[async_trait]
pub trait PostAdmissionObserver: Send + Sync {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck);

    async fn observe_error(
        &self,
        _envelope: ProductInboundEnvelope,
        _error: ironclaw_product_adapters::ProductAdapterError,
    ) {
    }
}

/// Optional protocol-specific reclassification of a normalized message into
/// a richer channel payload classification (today: gate-resolution replies like
/// `approve` / `deny gate:<ref>` / `auth deny <ref>`). Transitional debt:
/// deleted when gate replies become a host-generic channel concern.
pub type InboundPayloadClassifier =
    dyn Fn(&NormalizedInboundMessage) -> Option<ChannelInboundClassification> + Send + Sync;

/// How the sink mints the trusted auth claim for admitted messages —
/// mirrors the ingress verification recipe the router executed.
#[derive(Debug, Clone)]
pub enum VerifiedEvidenceMint {
    RequestSignature {
        signature_header: String,
        timestamp_header: Option<String>,
    },
    SharedSecretHeader {
        header: String,
    },
}

impl VerifiedEvidenceMint {
    fn mint(&self, subject: &str) -> ProtocolAuthEvidence {
        match self {
            Self::RequestSignature {
                signature_header,
                timestamp_header,
            } => ironclaw_product_adapters::auth::mark_request_signature_verified(
                signature_header.clone(),
                timestamp_header.clone(),
                subject,
            ),
            Self::SharedSecretHeader { header } => {
                ironclaw_product_adapters::auth::mark_shared_secret_header_verified(
                    header.clone(),
                    subject,
                )
            }
        }
    }
}

/// One extension's inbound wiring: verification secrets + the durable
/// admission sink (+ optional drain hook for post-admission tasks).
pub struct ChannelIngressRegistration {
    pub secrets: Arc<dyn IngressSecretsPort>,
    pub sink: Arc<dyn InboundSink>,
    /// Awaited on graceful shutdown after ingress stops accepting requests.
    pub drain: Option<Arc<dyn ChannelIngressDrain>>,
}

/// Async drain hook for registrations that schedule post-admission work.
#[async_trait]
pub trait ChannelIngressDrain: Send + Sync {
    async fn drain(&self);
}

/// One registry slot: the registration plus whether the generic channel
/// host assembly manages its lifetime (snapshot-driven register/replace/
/// unregister). Lane-owned registrations (`managed: false`) are never
/// touched by the assembly's reconcile passes.
struct RegisteredChannel {
    entry: Arc<ChannelIngressRegistration>,
    managed: bool,
}

/// Outcome of a managed (assembly-driven) registration attempt.
pub(crate) enum ManagedRegistrationOutcome {
    /// The managed entry is now registered; `replaced` carries a previously
    /// managed entry whose post-admission work still needs draining.
    Registered {
        replaced: Option<Arc<ChannelIngressRegistration>>,
    },
    /// A lane-owned (unmanaged) registration already serves this extension;
    /// the managed entry was not installed.
    SkippedUnmanaged,
}

/// The per-extension registration table behind the generic router's ports.
/// Registrations are data: concrete channel graphs (and the integration
/// harness) register their extension id; the router itself stays generic.
#[derive(Default)]
pub struct ExtensionIngressRegistry {
    registrations: RwLock<HashMap<String, RegisteredChannel>>,
}

impl ExtensionIngressRegistry {
    /// Register (or replace) one extension's inbound wiring. Lane-owned:
    /// the generic assembly's reconcile passes never replace or remove it.
    pub fn register(&self, extension_id: impl Into<String>, entry: ChannelIngressRegistration) {
        let mut registrations = match self.registrations.write() {
            Ok(registrations) => registrations,
            Err(poisoned) => poisoned.into_inner(),
        };
        registrations.insert(
            extension_id.into(),
            RegisteredChannel {
                entry: Arc::new(entry),
                managed: false,
            },
        );
    }

    /// Register an assembly-managed entry. Installs only when the slot is
    /// empty or currently holds another managed entry — a lane-owned
    /// registration always wins (check-and-insert under one write lock, so
    /// a concurrent lane registration cannot be clobbered).
    pub(crate) fn register_managed(
        &self,
        extension_id: impl Into<String>,
        entry: ChannelIngressRegistration,
    ) -> ManagedRegistrationOutcome {
        let mut registrations = match self.registrations.write() {
            Ok(registrations) => registrations,
            Err(poisoned) => poisoned.into_inner(),
        };
        let slot = extension_id.into();
        match registrations.get(&slot) {
            Some(existing) if !existing.managed => ManagedRegistrationOutcome::SkippedUnmanaged,
            existing => {
                let replaced = existing.map(|existing| Arc::clone(&existing.entry));
                registrations.insert(
                    slot,
                    RegisteredChannel {
                        entry: Arc::new(entry),
                        managed: true,
                    },
                );
                ManagedRegistrationOutcome::Registered { replaced }
            }
        }
    }

    /// Remove an assembly-managed entry (no-op for lane-owned entries).
    /// Returns the removed registration so the caller can drain it.
    pub(crate) fn unregister_managed(
        &self,
        extension_id: &str,
    ) -> Option<Arc<ChannelIngressRegistration>> {
        let mut registrations = match self.registrations.write() {
            Ok(registrations) => registrations,
            Err(poisoned) => poisoned.into_inner(),
        };
        match registrations.get(extension_id) {
            Some(existing) if existing.managed => registrations
                .remove(extension_id)
                .map(|removed| removed.entry),
            _ => None,
        }
    }

    /// Whether any inbound wiring (lane-owned or managed) is registered for
    /// this extension.
    pub fn is_registered(&self, extension_id: &str) -> bool {
        let registrations = match self.registrations.read() {
            Ok(registrations) => registrations,
            Err(poisoned) => poisoned.into_inner(),
        };
        registrations.contains_key(extension_id)
    }

    fn registration(&self, extension_id: &str) -> Option<Arc<ChannelIngressRegistration>> {
        let registrations = match self.registrations.read() {
            Ok(registrations) => registrations,
            Err(poisoned) => poisoned.into_inner(),
        };
        registrations
            .get(extension_id)
            .map(|registered| Arc::clone(&registered.entry))
    }

    /// Drain every registration's post-admission work (graceful shutdown).
    pub async fn drain(&self) {
        let drains: Vec<Arc<dyn ChannelIngressDrain>> = {
            let registrations = match self.registrations.read() {
                Ok(registrations) => registrations,
                Err(poisoned) => poisoned.into_inner(),
            };
            registrations
                .values()
                .filter_map(|registered| registered.entry.drain.clone())
                .collect()
        };
        for drain in drains {
            drain.drain().await;
        }
    }
}

#[async_trait]
impl IngressSecretsPort for ExtensionIngressRegistry {
    async fn verification_candidates(
        &self,
        extension_id: &str,
        installation_id: &str,
        handle: Option<&SecretHandle>,
    ) -> Result<Vec<VerificationCandidate>, IngressPortError> {
        let Some(entry) = self.registration(extension_id) else {
            // Active route without inbound wiring: fail closed (503), never
            // an unauthenticated 401 that would make the vendor drop events.
            return Err(IngressPortError {
                reason: format!("extension `{extension_id}` has no ingress registration"),
            });
        };
        entry
            .secrets
            .verification_candidates(extension_id, installation_id, handle)
            .await
    }
}

#[async_trait]
impl InboundSink for ExtensionIngressRegistry {
    async fn admit(
        &self,
        admission: InboundAdmission,
    ) -> Result<InboundAdmissionAck, InboundSinkError> {
        let Some(entry) = self.registration(&admission.extension_id) else {
            return Err(InboundSinkError {
                retryable: true,
                reason: format!(
                    "extension `{}` has no ingress registration",
                    admission.extension_id
                ),
            });
        };
        entry.sink.admit(admission).await
    }
}

// ── The generic inbound sink over ProductSurface admission ──────────────────

/// Pre-admission pairing interception for `WebGeneratedCode` channels: a
/// direct message from an actor with no identity binding is offered to the
/// pairing seam BEFORE ProductSurface admission, so a pairing code is consumed
/// instead of becoming (or failing as) a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChannelPairingInterception {
    NotHandled,
    Consumed(ChannelPairingConsumeOutcome),
    Failed,
}

#[async_trait]
pub(crate) trait ChannelPairingInterceptor: Send + Sync {
    async fn intercept(
        &self,
        installation_id: &AdapterInstallationId,
        message: &NormalizedInboundMessage,
    ) -> ChannelPairingInterception;
}

/// Configuration for [`GenericChannelInboundSink`].
pub struct ChannelInboundSinkConfig {
    /// The adapter identity stamped on inbound envelopes.
    pub adapter_id: ProductAdapterId,
    /// Auth-claim shape matching the executed verification recipe.
    pub evidence: VerifiedEvidenceMint,
    /// Optional protocol-specific payload reclassification (gate replies).
    pub classifier: Option<Arc<InboundPayloadClassifier>>,
    /// The ProductSurface channel admission door: durable idempotency ledger →
    /// identity/conversation binding → turn submission.
    pub surface: Arc<dyn ProductSurface>,
    /// Optional post-admission follow-up (e.g. final-reply delivery).
    pub observer: Option<Arc<dyn PostAdmissionObserver>>,
}

#[derive(Clone)]
pub(super) enum ChannelPairingOutcomeObserver {
    RunDelivery(Arc<crate::extension_host::channel_host::RunDeliveryPostAdmissionObserver>),
    #[cfg(test)]
    Recording(Arc<std::sync::Mutex<Vec<ChannelPairingConsumeOutcome>>>),
}

impl ChannelPairingOutcomeObserver {
    async fn observe(
        &self,
        conversation: ExternalConversationRef,
        event_id: ExternalEventId,
        outcome: ChannelPairingConsumeOutcome,
    ) {
        match self {
            Self::RunDelivery(observer) => {
                observer
                    .observe_pairing_outcome(conversation, event_id, outcome)
                    .await;
            }
            #[cfg(test)]
            Self::Recording(outcomes) => match outcomes.lock() {
                Ok(mut outcomes) => outcomes.push(outcome),
                Err(poisoned) => poisoned.into_inner().push(outcome),
            },
        }
    }
}

/// The generic [`InboundSink`]: builds the trusted inbound envelope from a
/// normalized message and submits it synchronously through ProductSurface —
/// the durable dedupe + admission commit the router requires
/// before acking 2xx. Post-admission observers run on tracked background
/// tasks drained at shutdown.
pub struct GenericChannelInboundSink {
    config: ChannelInboundSinkConfig,
    pairing: Option<Arc<dyn ChannelPairingInterceptor>>,
    pairing_outcome_observer: Option<ChannelPairingOutcomeObserver>,
    observer_tasks: tokio::sync::Mutex<JoinSet<()>>,
}

impl GenericChannelInboundSink {
    pub fn new(config: ChannelInboundSinkConfig) -> Self {
        Self {
            config,
            pairing: None,
            pairing_outcome_observer: None,
            observer_tasks: tokio::sync::Mutex::new(JoinSet::new()),
        }
    }

    pub(super) fn with_pairing(
        mut self,
        pairing: Arc<dyn ChannelPairingInterceptor>,
        observer: Option<ChannelPairingOutcomeObserver>,
    ) -> Self {
        self.pairing = Some(pairing);
        self.pairing_outcome_observer = observer;
        self
    }

    fn permanent(reason: impl std::fmt::Display) -> InboundSinkError {
        InboundSinkError {
            retryable: false,
            reason: reason.to_string(),
        }
    }

    fn channel_caller() -> Result<WebUiAuthenticatedCaller, InboundSinkError> {
        Ok(WebUiAuthenticatedCaller::new(
            TenantId::new("channel-ingress").map_err(Self::permanent)?,
            UserId::new("channel-ingress").map_err(Self::permanent)?,
            None,
            None,
        ))
    }

    async fn spawn_observer<F>(&self, run: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut tasks = self.observer_tasks.lock().await;
        // Reap finished tasks so the set stays bounded.
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result {
                tracing::debug!(
                    error = %error,
                    "post-admission observer task finished with join error"
                );
            }
        }
        tasks.spawn(run);
    }
}

#[async_trait]
impl ChannelIngressDrain for GenericChannelInboundSink {
    async fn drain(&self) {
        let mut tasks = self.observer_tasks.lock().await;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                tracing::debug!(
                    error = %error,
                    "post-admission observer task finished with join error"
                );
            }
        }
    }
}

#[async_trait]
impl InboundSink for GenericChannelInboundSink {
    async fn admit(
        &self,
        admission: InboundAdmission,
    ) -> Result<InboundAdmissionAck, InboundSinkError> {
        let InboundAdmission {
            extension_id: _,
            installation_id,
            message,
        } = admission;
        let installation = AdapterInstallationId::new(&installation_id).map_err(Self::permanent)?;
        // Pairing pre-admission gate: a serviced pairing interaction is
        // durably reflected in the pairing/identity stores, not the turn
        // ledger — the vendor still gets its 2xx.
        if let Some(pairing) = &self.pairing {
            // Boxed: the consume path (CAS claim → identity bind → completion
            // fan-out) is a deep async subtree nested inside the admission
            // future; boxing keeps instrumented builds off the stack limit.
            match Box::pin(pairing.intercept(&installation, &message)).await {
                ChannelPairingInterception::NotHandled => {}
                ChannelPairingInterception::Consumed(outcome) => {
                    if let Some(observer) = self.pairing_outcome_observer.clone() {
                        let conversation = message.conversation.clone();
                        let event_id = message.event_id.clone();
                        self.spawn_observer(async move {
                            observer.observe(conversation, event_id, outcome).await;
                        })
                        .await;
                    }
                    return Ok(InboundAdmissionAck::Accepted);
                }
                ChannelPairingInterception::Failed => {
                    return Ok(InboundAdmissionAck::Accepted);
                }
            }
        }
        let evidence = self.config.evidence.mint(&installation_id);
        // Durable dedupe + admission commit (idempotency ledger keyed by
        // installation + external event fingerprint) plus identity/
        // conversation binding and turn submission — synchronous, so the
        // router's 2xx is ack-after-commit.
        // Boxed: ProductSurface admission (ledger → identity/actor resolution →
        // conversation binding → turn submission) is the deepest subtree in
        // this future; boxing keeps instrumented builds off the stack limit.
        let request = ChannelInboundSurfaceRequest {
            adapter_id: self.config.adapter_id.clone(),
            source_channel: ProductSourceChannel::new(self.config.adapter_id.as_str())
                .map_err(Self::permanent)?,
            installation_id: installation,
            evidence,
            received_at: Utc::now(),
            classification: self
                .config
                .classifier
                .as_ref()
                .and_then(|classify| classify(&message)),
            message,
        };
        let response = Box::pin(self.config.surface.execute_command(
            Self::channel_caller()?,
            ProductOperationRequest::channel_inbound(request),
        ))
        .await
        .map_err(Self::permanent)?
        .into_channel_inbound()
        .map_err(Self::permanent)?;
        match response {
            ChannelInboundSurfaceOutcome::Admitted(admission) => {
                let admission = *admission;
                let envelope = admission.envelope;
                let ack = admission.ack;
                let duplicate = matches!(ack, ProductInboundAck::Duplicate { .. });
                let durable = ack.is_durable_outcome();
                if let Some(observer) = self.config.observer.clone() {
                    self.spawn_observer(async move {
                        observer.observe_ack(envelope, ack).await;
                    })
                    .await;
                }
                if duplicate {
                    Ok(InboundAdmissionAck::Duplicate)
                } else if durable {
                    Ok(InboundAdmissionAck::Accepted)
                } else {
                    Err(InboundSinkError {
                        retryable: true,
                        reason: "ProductSurface returned a non-durable rejection".to_string(),
                    })
                }
            }
            ChannelInboundSurfaceOutcome::Invalid(error) => Err(Self::permanent(error)),
            ChannelInboundSurfaceOutcome::Rejected(rejection) => {
                let ChannelInboundSurfaceRejectedAdmission { envelope, error } = *rejection;
                let retryable = error.is_retryable();
                if let Some(observer) = self.config.observer.clone() {
                    self.spawn_observer(async move {
                        observer.observe_error(envelope, error).await;
                    })
                    .await;
                } else if !retryable {
                    tracing::debug!(
                        "inbound admission settled terminally with no post-admission observer"
                    );
                }
                if retryable {
                    Err(InboundSinkError {
                        retryable: true,
                        reason: "ProductSurface admission failed retryably".to_string(),
                    })
                } else {
                    // A non-retryable ProductSurface error is settled in the durable
                    // idempotency ledger (a vendor redelivery replays as
                    // Duplicate) — the event is durably accounted for, so the
                    // vendor gets its 2xx; user-visible feedback flows through
                    // the post-admission observer.
                    Ok(InboundAdmissionAck::Accepted)
                }
            }
        }
    }
}

/// A static secrets port: fixed candidates for one extension (operator
/// config resolved at registration time). Dynamic setups implement
/// [`IngressSecretsPort`] directly and re-read their stores per request.
pub struct StaticIngressSecrets {
    candidates: Vec<VerificationCandidate>,
}

impl StaticIngressSecrets {
    pub fn new(candidates: Vec<VerificationCandidate>) -> Self {
        Self { candidates }
    }
}

#[async_trait]
impl IngressSecretsPort for StaticIngressSecrets {
    async fn verification_candidates(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        _handle: Option<&SecretHandle>,
    ) -> Result<Vec<VerificationCandidate>, IngressPortError> {
        Ok(self.candidates.clone())
    }
}

// ── The composed router parts + serve mount ─────────────────────────────────

/// The composed generic ingress: the deployment-first router (with an active
/// snapshot compatibility fallback) plus the registration surface. Built once
/// by composition; the serve layer mounts [`extension_ingress_route_mount`]
/// over it.
#[derive(Clone)]
pub struct ExtensionIngressParts {
    pub router: Arc<ExtensionIngressRouter>,
    pub registry: Arc<ExtensionIngressRegistry>,
    /// The router's `reply_context` storage — shared with the delivery
    /// coordinator's read half (ING-11).
    pub reply_context: Arc<dyn ironclaw_extension_host::ingress::ReplyContextStore>,
}

/// Build the generic ingress router over deployment bindings and the generic
/// host's compatibility snapshot watch.
/// `reply_context` is the durable ING-11 store (production: the
/// filesystem-backed [`crate::extension_host::reply_contexts::FilesystemReplyContextStore`],
/// so contexts stored before admission survive a restart to delivery time).
pub(crate) fn build_extension_ingress(
    watch: ironclaw_extension_host::SnapshotWatch,
    deployment_channels: Arc<ironclaw_extension_host::DeploymentChannelRegistry>,
    reply_context: Arc<dyn ironclaw_extension_host::ingress::ReplyContextStore>,
) -> ExtensionIngressParts {
    let registry = Arc::new(ExtensionIngressRegistry::default());
    let router = Arc::new(
        ExtensionIngressRouter::new(
            watch,
            ironclaw_extension_host::ingress::ExtensionIngressRouterDeps {
                secrets: Arc::clone(&registry) as Arc<dyn IngressSecretsPort>,
                sink: Arc::clone(&registry) as Arc<dyn InboundSink>,
                reply_context: Arc::clone(&reply_context),
            },
            ironclaw_extension_host::ingress::IngressRouterConfig::default(),
        )
        .with_deployment_channels(deployment_channels),
    );
    ExtensionIngressParts {
        router,
        registry,
        reply_context,
    }
}

pub use serve_mount::{EXTENSION_INGRESS_ROUTE_PATTERN, extension_ingress_route_mount};

mod serve_mount {
    use std::num::{NonZeroU32, NonZeroU64};
    use std::pin::Pin;

    use axum::{
        Router,
        body::Bytes,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
    };
    use ironclaw_extension_host::ingress::{IngressRequest, IngressResponse};
    use ironclaw_host_api::NetworkMethod;
    use ironclaw_host_api::ingress::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
        IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
        IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
        WebSocketOriginPolicy,
    };

    use super::*;
    use crate::webui::route_mounts::{PublicRouteDrain, PublicRouteMount};

    /// The canonical generic ingress route pattern (axum path params).
    pub const EXTENSION_INGRESS_ROUTE_PATTERN: &str =
        "/webhooks/extensions/{extension_id}/{route_suffix}";

    const EXTENSION_INGRESS_ROUTE_ID: &str = "extensions.channel_ingress";

    /// Host ceiling for any extension channel body (per-extension limits from
    /// the channel descriptor are enforced inside the router, and are
    /// expected to be at or below this).
    const EXTENSION_INGRESS_BODY_CEILING_BYTES: u64 = 8 * 1024 * 1024;

    /// Host policy floor for public webhook ingress (mirrors the previous
    /// per-channel mounts). Compile-time non-zero.
    const PUBLIC_WEBHOOK_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(12_000) {
        Some(value) => value,
        None => unreachable!(),
    };
    const PUBLIC_WEBHOOK_WINDOW_SECONDS: NonZeroU32 = match NonZeroU32::new(60) {
        Some(value) => value,
        None => unreachable!(),
    };

    /// Build the single `PublicRouteMount` serving every extension channel's
    /// ingress. Mounted once; route resolution follows deployment bindings
    /// first and active snapshot bindings second.
    pub fn extension_ingress_route_mount(
        parts: &ExtensionIngressParts,
    ) -> Result<PublicRouteMount, crate::RebornBuildError> {
        let descriptor =
            ingress_route_descriptor(EXTENSION_INGRESS_ROUTE_ID, EXTENSION_INGRESS_ROUTE_PATTERN)?;

        let router = Router::new()
            .route(EXTENSION_INGRESS_ROUTE_PATTERN, post(ingress_handler))
            .with_state(Arc::clone(&parts.router));
        Ok(
            PublicRouteMount::new(router, vec![descriptor]).with_drain(Arc::new(RegistryDrain {
                registry: Arc::clone(&parts.registry),
            })),
        )
    }

    fn ingress_route_descriptor(
        route_id: &'static str,
        path: &'static str,
    ) -> Result<IngressRouteDescriptor, crate::RebornBuildError> {
        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::PublicWebhook,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::WebhookSignature],
            },
            scope_source: IngressScopeSource::HostResolved,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(EXTENSION_INGRESS_BODY_CEILING_BYTES)
                    .unwrap_or(NonZeroU64::MIN),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::Global,
                max_requests: PUBLIC_WEBHOOK_MAX_REQUESTS,
                window_seconds: PUBLIC_WEBHOOK_WINDOW_SECONDS,
            },
            cors: CorsPolicy::NotApplicable,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::ProductWorkflow,
        })
        .map_err(|error| crate::RebornBuildError::InvalidConfig {
            reason: format!("extension ingress policy invalid: {error}"),
        })?;
        IngressRouteDescriptor::new(route_id, NetworkMethod::Post, path, policy).map_err(|error| {
            crate::RebornBuildError::InvalidConfig {
                reason: format!("extension ingress descriptor invalid: {error}"),
            }
        })
    }

    struct RegistryDrain {
        registry: Arc<ExtensionIngressRegistry>,
    }

    impl PublicRouteDrain for RegistryDrain {
        fn drain<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(self.registry.drain())
        }
    }

    async fn ingress_handler(
        State(router): State<Arc<ExtensionIngressRouter>>,
        Path((extension_id, route_suffix)): Path<(String, String)>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let response = router
            .handle(ingress_request(
                "POST",
                extension_id,
                route_suffix,
                &headers,
                body,
            ))
            .await;
        into_axum_response(response)
    }

    fn ingress_request(
        method: &str,
        extension_id: String,
        route_suffix: String,
        headers: &HeaderMap,
        body: Bytes,
    ) -> IngressRequest {
        IngressRequest {
            method: method.to_string(),
            extension_id,
            route_suffix,
            headers: headers
                .iter()
                .map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec()))
                .collect(),
            body: body.to_vec(),
        }
    }

    fn into_axum_response(response: IngressResponse) -> Response {
        let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY);
        match response.content_type {
            Some(content_type) => {
                (status, [("content-type", content_type)], response.body).into_response()
            }
            None => (status, response.body).into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_host_api::UserId;
    use ironclaw_product_adapters::{
        ExternalActorRef, ExternalConversationRef, ExternalEventId, ParsedProductInbound,
        ProductInboundPayload, ProductTriggerReason, TrustedInboundContext, UserMessagePayload,
    };
    use ironclaw_product_workflow::{
        ChannelInboundSurfaceAdmission, ChannelInboundSurfaceOutcome, ProductOperationId,
        ProductOperationResponse, ProductOperationTypedInput, ProductSurface, RebornServicesError,
        RebornServicesErrorCode, RebornServicesErrorKind,
    };
    use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

    use super::*;
    use crate::extension_host::channel_pairing::ChannelPairingConsumeOutcome;

    struct CountingSurface {
        submissions: AtomicUsize,
    }

    impl CountingSurface {
        fn new() -> Self {
            Self {
                submissions: AtomicUsize::new(0),
            }
        }

        fn submit_count(&self) -> usize {
            self.submissions.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ProductSurface for CountingSurface {
        async fn execute_command(
            &self,
            _caller: WebUiAuthenticatedCaller,
            request: ProductOperationRequest,
        ) -> Result<ProductOperationResponse, RebornServicesError> {
            let operation_id =
                ProductOperationId::parse(request.operation_id.as_str()).ok_or_else(unavailable)?;
            if operation_id != ProductOperationId::ChannelInboundAdmit {
                return Err(unavailable());
            }
            let Some(ProductOperationTypedInput::ChannelInbound(request)) = request.typed_input
            else {
                return Ok(ProductOperationResponse::channel_inbound(
                    ChannelInboundSurfaceOutcome::unavailable(),
                ));
            };
            self.submissions.fetch_add(1, Ordering::SeqCst);
            let ack = ProductInboundAck::Accepted {
                accepted_message_ref: AcceptedMessageRef::new("msg:extension-ingress-test")
                    .expect("accepted message ref"),
                submitted_run_id: TurnRunId::new(),
            };
            let envelope = ProductInboundEnvelope::from_trusted_parse(
                TrustedInboundContext::from_verified_evidence_with_source_channel(
                    request.adapter_id,
                    request.source_channel,
                    request.installation_id,
                    request.received_at,
                    &request.evidence,
                )
                .expect("verified evidence"),
                ParsedProductInbound::new(
                    request.message.event_id,
                    request.message.actor,
                    request.message.conversation,
                    ProductInboundPayload::UserMessage(
                        UserMessagePayload::new(
                            request.message.text,
                            Vec::new(),
                            request.message.trigger,
                        )
                        .expect("user message payload"),
                    ),
                )
                .expect("parsed inbound"),
            )
            .expect("trusted envelope");
            Ok(ProductOperationResponse::channel_inbound(
                ChannelInboundSurfaceOutcome::Admitted(Box::new(ChannelInboundSurfaceAdmission {
                    envelope,
                    ack,
                })),
            ))
        }
    }

    fn unavailable() -> RebornServicesError {
        RebornServicesError {
            code: RebornServicesErrorCode::Unavailable,
            kind: RebornServicesErrorKind::ServiceUnavailable,
            status_code: 503,
            retryable: false,
            field: None,
            validation_code: None,
        }
    }

    struct ScriptedPairingInterceptor {
        interception: ChannelPairingInterception,
    }

    #[async_trait]
    impl ChannelPairingInterceptor for ScriptedPairingInterceptor {
        async fn intercept(
            &self,
            _installation_id: &AdapterInstallationId,
            _message: &NormalizedInboundMessage,
        ) -> ChannelPairingInterception {
            self.interception.clone()
        }
    }

    fn admission_for(text: &str) -> InboundAdmission {
        InboundAdmission {
            extension_id: "vendorx".to_string(),
            installation_id: "install".to_string(),
            message: NormalizedInboundMessage {
                actor: ExternalActorRef::new("vendor_user", "user-1", None::<&str>).expect("actor"),
                conversation: ExternalConversationRef::new(None, "chat-1", None, None)
                    .expect("conversation"),
                event_id: ExternalEventId::new("evt-1").expect("event"),
                text: text.to_string(),
                trigger: ProductTriggerReason::DirectChat,
                attachments: Vec::new(),
                reply_context: None,
            },
        }
    }

    fn pairing_sink(
        interception: ChannelPairingInterception,
    ) -> (
        GenericChannelInboundSink,
        Arc<CountingSurface>,
        Arc<std::sync::Mutex<Vec<ChannelPairingConsumeOutcome>>>,
    ) {
        let workflow = Arc::new(CountingSurface::new());
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id: ProductAdapterId::new("vendorx").expect("adapter id"),
            evidence: VerifiedEvidenceMint::SharedSecretHeader {
                header: "X-Vendor-Secret".to_string(),
            },
            classifier: None,
            surface: Arc::clone(&workflow) as Arc<dyn ProductSurface>,
            observer: None,
        })
        .with_pairing(
            Arc::new(ScriptedPairingInterceptor { interception }),
            Some(ChannelPairingOutcomeObserver::Recording(Arc::clone(
                &outcomes,
            ))),
        );
        (sink, workflow, outcomes)
    }

    struct FailingSink;

    #[async_trait]
    impl InboundSink for FailingSink {
        async fn admit(
            &self,
            _admission: InboundAdmission,
        ) -> Result<InboundAdmissionAck, InboundSinkError> {
            Err(InboundSinkError {
                retryable: true,
                reason: "test sink".to_string(),
            })
        }
    }

    fn registration(secret: &[u8]) -> ChannelIngressRegistration {
        ChannelIngressRegistration {
            secrets: Arc::new(StaticIngressSecrets::new(vec![VerificationCandidate {
                installation_id: "install".to_string(),
                secret: secret.to_vec(),
            }])),
            sink: Arc::new(FailingSink),
            drain: None,
        }
    }

    async fn registered_secret(registry: &ExtensionIngressRegistry, extension_id: &str) -> Vec<u8> {
        registry
            .verification_candidates(extension_id, "install", None)
            .await
            .expect("registration present")
            .first()
            .expect("one candidate")
            .secret
            .clone()
    }

    #[tokio::test]
    async fn managed_registration_never_replaces_a_lane_owned_entry() {
        let registry = ExtensionIngressRegistry::default();
        registry.register("vendorx", registration(b"lane"));

        assert!(matches!(
            registry.register_managed("vendorx", registration(b"managed")),
            ManagedRegistrationOutcome::SkippedUnmanaged
        ));
        assert_eq!(registered_secret(&registry, "vendorx").await, b"lane");
        assert!(
            registry.unregister_managed("vendorx").is_none(),
            "a lane-owned entry must survive managed unregistration"
        );
        assert!(registry.is_registered("vendorx"));
    }

    #[tokio::test]
    async fn managed_registration_installs_replaces_and_unregisters_managed_entries() {
        let registry = ExtensionIngressRegistry::default();
        assert!(!registry.is_registered("vendorx"));

        let ManagedRegistrationOutcome::Registered { replaced } =
            registry.register_managed("vendorx", registration(b"one"))
        else {
            panic!("empty slot must accept a managed entry");
        };
        assert!(replaced.is_none());
        assert_eq!(registered_secret(&registry, "vendorx").await, b"one");

        let ManagedRegistrationOutcome::Registered { replaced } =
            registry.register_managed("vendorx", registration(b"two"))
        else {
            panic!("a managed entry must be replaceable by the assembly");
        };
        assert!(
            replaced.is_some(),
            "the replaced managed entry is returned for draining"
        );
        assert_eq!(registered_secret(&registry, "vendorx").await, b"two");

        assert!(registry.unregister_managed("vendorx").is_some());
        assert!(!registry.is_registered("vendorx"));
    }

    #[tokio::test]
    async fn lane_registration_reclaims_a_managed_slot() {
        let registry = ExtensionIngressRegistry::default();
        let ManagedRegistrationOutcome::Registered { .. } =
            registry.register_managed("vendorx", registration(b"managed"))
        else {
            panic!("empty slot must accept a managed entry");
        };

        registry.register("vendorx", registration(b"lane"));
        assert_eq!(registered_secret(&registry, "vendorx").await, b"lane");
        assert!(matches!(
            registry.register_managed("vendorx", registration(b"managed-again")),
            ManagedRegistrationOutcome::SkippedUnmanaged
        ));
    }

    #[tokio::test]
    async fn pairing_interception_preserves_every_typed_consume_outcome_for_the_observer() {
        let user_id = UserId::new("paired-user").expect("user id");
        for outcome in [
            ChannelPairingConsumeOutcome::Paired {
                user_id: user_id.clone(),
            },
            ChannelPairingConsumeOutcome::AlreadyPairedSameUser {
                user_id: user_id.clone(),
            },
            ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser,
            ChannelPairingConsumeOutcome::ExpiredOrUnknown,
        ] {
            let (sink, workflow, observer) =
                pairing_sink(ChannelPairingInterception::Consumed(outcome.clone()));

            let ack = sink
                .admit(admission_for("ABCDEFGH"))
                .await
                .expect("admitted");
            assert_eq!(ack, InboundAdmissionAck::Accepted);
            sink.drain().await;
            assert_eq!(workflow.submit_count(), 0);
            assert_eq!(observer.lock().expect("outcomes lock").pop(), Some(outcome));
        }
    }

    #[tokio::test]
    async fn pairing_not_handled_continues_to_the_product_workflow() {
        let (sink, workflow, observer) = pairing_sink(ChannelPairingInterception::NotHandled);

        let ack = sink.admit(admission_for("hello")).await.expect("admitted");
        assert_eq!(ack, InboundAdmissionAck::Accepted);
        sink.drain().await;
        assert_eq!(workflow.submit_count(), 1);
        assert_eq!(observer.lock().expect("outcomes lock").pop(), None);
    }
}
