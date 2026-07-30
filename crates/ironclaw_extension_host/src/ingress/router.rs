//! Transport-neutral generic ingress router.
//!
//! See the module docs in [`super`] for the pinned per-request order. The
//! router owns semantics and security; the extension contributes exactly one
//! pure call (`ChannelAdapter::inbound`). Ordinary messages reach the durable
//! admission commit before 2xx. Provider-batch fragments reach host-private
//! durable staging before 2xx, then one leased background worker admits the
//! merged message after the provider-selected quiet window (checklist ING-8).

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_extensions::ResolvedExtensionManifest;
use ironclaw_host_api::{
    ChannelIngressDescriptor, ChannelIngressMethod, RestrictedEgress, SecretHandle,
};
use ironclaw_product::{
    ChannelAdapter, ChannelError, InboundBatchFragment, InboundOutcome, NormalizedInboundMessage,
    VerifiedInbound,
};
use sha2::{Digest, Sha256};

use crate::active::ActiveExtension;
use crate::deployment_channels::{DeploymentChannelBinding, DeploymentChannelRegistry};
use crate::egress::{ChannelEgressTransport, DeclaredChannelEgress, PolicyEnforcedChannelEgress};
use crate::inbound_batches::{
    ClaimedInboundBatch, InboundBatchKey, InboundBatchSchedule, InboundBatchStageOutcome,
    InboundBatchStageRequest, InboundBatchStore,
};
use crate::lifecycle::SnapshotWatch;

use super::verifier::{IngressHeaders, VerificationCandidate, verify_recipe};

/// The canonical mounted path for one extension channel's ingress route.
pub fn canonical_ingress_path(extension_id: &str, route_suffix: &str) -> String {
    format!("/webhooks/extensions/{extension_id}/{route_suffix}")
}

/// A failed ingress port call. Ports fail closed: the router maps this to a
/// retryable 503 (never a 2xx).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ingress port unavailable: {reason}")]
pub struct IngressPortError {
    pub reason: String,
}

/// Resolves the verification-secret candidates for one extension route.
/// Implemented by composition over the host secret/config stores — secrets
/// stay host-side; the router hands them only to the constant-time verifier.
#[async_trait]
pub trait IngressSecretsPort: Send + Sync {
    /// Candidate installations (id + secret bytes) for this route. `handle`
    /// is `None` for `kind = "none"` recipes, where only the installation
    /// identity is needed and returned secrets must be empty.
    async fn verification_candidates(
        &self,
        extension_id: &str,
        installation_id: &str,
        handle: Option<&SecretHandle>,
    ) -> Result<Vec<VerificationCandidate>, IngressPortError>;
}

/// Resolves manifest-declared non-secret configuration for the installation
/// selected by ingress verification. Implementations must not return secret
/// values or handles.
#[async_trait]
pub trait IngressConfigurationPort: Send + Sync {
    async fn non_secret_config(
        &self,
        extension_id: &str,
        installation_id: &str,
    ) -> Result<Vec<(String, String)>, IngressPortError>;
}

/// One verified, normalized inbound message ready for durable admission.
pub struct InboundAdmission {
    pub extension_id: String,
    pub installation_id: String,
    pub message: NormalizedInboundMessage,
    /// The exact adapter that parsed this request. Host-only and transient:
    /// admission keeps this `Arc` across replay/policy so an activation swap
    /// cannot fetch attachment bytes through a different generation.
    pub channel_adapter: Arc<dyn ChannelAdapter>,
    /// Manifest-restricted egress built from the same resolved ingress
    /// binding as `channel_adapter`. `None` means the host has no channel
    /// transport; attachment-bearing admission then fails closed.
    pub channel_egress: Option<Arc<dyn RestrictedEgress>>,
}

/// The durable admission outcome. Both variants mean the event is durably
/// accounted for — the router may 2xx.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundAdmissionAck {
    Accepted,
    /// The `(installation, event_id)` dedupe key was already settled.
    Duplicate,
}

/// A failed admission. `retryable` selects 503 (vendor should redeliver)
/// versus 400 (permanent rejection).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("inbound admission failed: {reason}")]
pub struct InboundSinkError {
    pub retryable: bool,
    pub reason: String,
}

/// The durable dedupe + admission commit seam (one transaction keyed
/// `(installation, event_id)`), implemented by composition over the existing
/// product workflow (idempotency ledger → identity/conversation binding →
/// turn submission).
#[async_trait]
pub trait InboundSink: Send + Sync {
    async fn admit(
        &self,
        admission: InboundAdmission,
    ) -> Result<InboundAdmissionAck, InboundSinkError>;
}

/// Host-side storage key for an inbound message's opaque `reply_context`:
/// the conversation source binding it will be handed back for at delivery
/// time (checklist ING-11).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplyContextKey {
    pub extension_id: String,
    pub installation_id: String,
    /// The conversation fingerprint
    /// ([`ironclaw_product::ExternalConversationRef::conversation_fingerprint`]).
    pub conversation: String,
}

/// Host-side `reply_context` storage. Stored before admission commits;
/// the delivery coordinator (P5) reads it back for source-route replies.
#[async_trait]
pub trait ReplyContextStore: Send + Sync {
    async fn put(&self, key: ReplyContextKey, context: Vec<u8>) -> Result<(), IngressPortError>;
    async fn get(&self, key: &ReplyContextKey) -> Result<Option<Vec<u8>>, IngressPortError>;
}

/// Per-installation token-bucket rate limit (defaults match the previous
/// host-served channel ingress: 120 requests / 60 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngressRateLimitConfig {
    pub max_requests: u32,
    pub window: Duration,
}

impl Default for IngressRateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 120,
            window: Duration::from_secs(60),
        }
    }
}

/// Router-wide configuration.
#[derive(Debug, Clone, Copy)]
pub struct IngressRouterConfig {
    pub rate_limit: IngressRateLimitConfig,
    /// Bounded budget for verification + adapter + admission per request.
    pub request_deadline: Duration,
}

impl Default for IngressRouterConfig {
    fn default() -> Self {
        Self {
            rate_limit: IngressRateLimitConfig::default(),
            request_deadline: Duration::from_secs(20),
        }
    }
}

/// One inbound HTTP request, transport-neutral. Composition extracts the two
/// path segments from the mounted route pattern.
pub struct IngressRequest {
    /// HTTP method token (e.g. `POST`), matched case-insensitively.
    pub method: String,
    pub extension_id: String,
    pub route_suffix: String,
    /// Raw header entries in wire order (duplicates preserved — duplicate
    /// verification headers must be observable to fail closed).
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
}

/// The router's response, mapped 1:1 onto the HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl IngressResponse {
    fn error(status: u16, category: &str) -> Self {
        Self {
            status,
            content_type: Some("application/json".to_string()),
            body: format!("{{\"error\":\"{category}\"}}").into_bytes(),
        }
    }

    fn ok() -> Self {
        Self {
            status: 200,
            content_type: Some("text/plain".to_string()),
            body: b"ok".to_vec(),
        }
    }
}

/// Injected router dependencies (composition supplies concrete ports).
#[derive(Clone)]
pub struct ExtensionIngressRouterDeps {
    pub secrets: Arc<dyn IngressSecretsPort>,
    pub configuration: Arc<dyn IngressConfigurationPort>,
    pub sink: Arc<dyn InboundSink>,
    pub reply_context: Arc<dyn ReplyContextStore>,
    /// Host-private durable staging for provider-level batches whose
    /// fragments arrive as separate webhook requests.
    pub inbound_batches: Arc<dyn InboundBatchStore>,
    /// Host transport used only to construct per-request manifest-restricted
    /// egress from the already pinned ingress binding.
    // arch-exempt: optional_arc, minimal/test host-runtime graphs intentionally
    // omit HTTP egress; absence preserves the explicit fail-closed 503 path for
    // attachment-bearing messages, plan #4539
    pub channel_egress_transport: Option<Arc<dyn ChannelEgressTransport>>,
}

/// The generic ingress router. One instance serves every active extension's
/// channel ingress; resolution is per request through the snapshot watch.
pub struct ExtensionIngressRouter {
    watch: SnapshotWatch,
    deployment_channels: Arc<DeploymentChannelRegistry>,
    deps: ExtensionIngressRouterDeps,
    config: IngressRouterConfig,
    rate: RateLimiter,
    batch_processor: InboundBatchProcessor,
}

impl ExtensionIngressRouter {
    pub fn new(
        watch: SnapshotWatch,
        deps: ExtensionIngressRouterDeps,
        config: IngressRouterConfig,
    ) -> Self {
        let deployment_channels = Arc::new(DeploymentChannelRegistry::default());
        let batch_processor = InboundBatchProcessor {
            watch: watch.clone(),
            deployment_channels: Arc::clone(&deployment_channels),
            deps: deps.clone(),
            recovery_started: Arc::new(AtomicBool::new(false)),
            admission_deadline: config.request_deadline,
        };
        Self {
            watch,
            deployment_channels,
            deps,
            rate: RateLimiter::new(config.rate_limit),
            config,
            batch_processor,
        }
    }

    /// Resolve manifest-declared deployment ingress independently of the
    /// user-installation active snapshot.
    pub fn with_deployment_channels(
        mut self,
        deployment_channels: Arc<DeploymentChannelRegistry>,
    ) -> Self {
        self.deployment_channels = Arc::clone(&deployment_channels);
        self.batch_processor.deployment_channels = deployment_channels;
        self
    }

    /// Start the durable recovery sweep after the final deployment bindings
    /// have been attached. The sweep immediately reclaims due work and then
    /// periodically retries unavailable bindings or expired leases.
    pub fn start_pending_batch_recovery(&self) {
        self.batch_processor.start_recovery_sweep();
    }

    /// Handle one request following the pinned order. Never panics; never
    /// returns 2xx before the outcome's durable boundary (ordinary admission
    /// or provider-batch fragment staging).
    pub async fn handle(&self, request: IngressRequest) -> IngressResponse {
        // 1. Match deployment ingress first. User activation remains a
        // compatibility source for extensions not linked into the deployment
        // registry; it is not required for operator-configured channels.
        let binding = self
            .deployment_channels
            .resolve_channel_ingress(&request.extension_id, &request.route_suffix)
            .map(ResolvedIngressBinding::Deployment)
            .or_else(|| {
                self.watch
                    .current()
                    .resolve_channel_ingress(&request.extension_id, &request.route_suffix)
                    .map(ResolvedIngressBinding::Active)
            });
        let Some(binding) = binding else {
            return IngressResponse::error(404, "unknown_route");
        };
        let Some(ingress) = binding
            .resolved()
            .channel
            .as_ref()
            .and_then(|channel| channel.ingress.as_ref())
        else {
            return IngressResponse::error(404, "unknown_route");
        };

        // 2. Method / body-limit / rate-limit — before any verification or
        //    adapter work.
        if !method_allowed(&request.method, ingress) {
            return IngressResponse::error(405, "method_not_allowed");
        }
        if request.body.len() as u64 > ingress.body_limit_bytes {
            return IngressResponse::error(413, "payload_too_large");
        }
        if !self.rate.try_admit(&request.extension_id) {
            return IngressResponse::error(429, "capacity");
        }

        // 3. Deadline around verification + adapter + durable admission.
        let deadline = self.config.request_deadline;
        match tokio::time::timeout(
            deadline,
            self.verify_and_dispatch(&request, &binding, ingress),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => {
                tracing::debug!(
                    extension_id = %request.extension_id,
                    "extension ingress request exceeded its bounded deadline"
                );
                IngressResponse::error(503, "temporarily_unavailable")
            }
        }
    }

    async fn verify_and_dispatch(
        &self,
        request: &IngressRequest,
        binding: &ResolvedIngressBinding,
        ingress: &ChannelIngressDescriptor,
    ) -> IngressResponse {
        // 4. Verification recipe execution — host-side, before the adapter.
        let candidates = match self
            .deps
            .secrets
            .verification_candidates(
                binding.extension_id(),
                binding.installation_hint(),
                ingress.verification.secret_handle(),
            )
            .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::debug!(
                    extension_id = %binding.extension_id(),
                    error = %error,
                    "extension ingress verification secrets unavailable"
                );
                return IngressResponse::error(503, "temporarily_unavailable");
            }
        };
        let now_unix_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        let verified = match verify_recipe(
            &ingress.verification,
            &IngressHeaders::new(&request.headers),
            &request.body,
            now_unix_seconds,
            &candidates,
        ) {
            Ok(verified) => verified,
            Err(failure) => {
                tracing::debug!(
                    extension_id = %binding.extension_id(),
                    failure = %failure,
                    "extension ingress verification rejected"
                );
                return IngressResponse::error(401, "authentication");
            }
        };
        drop(candidates); // secrets leave scope before any adapter work

        // 5. Resolve only manifest-declared non-secret configuration for the
        //    installation selected by successful verification.
        let non_secret_config = match self
            .deps
            .configuration
            .non_secret_config(binding.extension_id(), &verified.installation_id)
            .await
        {
            Ok(config) => config,
            Err(error) => {
                tracing::debug!(
                    extension_id = %binding.extension_id(),
                    error = %error,
                    "extension ingress non-secret configuration unavailable"
                );
                return IngressResponse::error(503, "temporarily_unavailable");
            }
        };

        // 6. adapter.inbound — pure, panic-isolated; verification headers are
        //    consumed by the host and never forwarded.
        let Some(channel) = binding.adapter() else {
            return IngressResponse::error(404, "unknown_route");
        };
        let forwarded_headers: Vec<(String, String)> = request
            .headers
            .iter()
            .filter(|(name, _)| {
                !verified
                    .consumed_headers
                    .iter()
                    .any(|consumed| consumed.eq_ignore_ascii_case(name))
            })
            .map(|(name, value)| (name.clone(), String::from_utf8_lossy(value).into_owned()))
            .collect();
        let outcome = {
            let inbound = VerifiedInbound {
                extension_id: binding.extension_id(),
                installation_id: &verified.installation_id,
                config: &non_secret_config,
                body: &request.body,
                headers: &forwarded_headers,
            };
            match catch_unwind(AssertUnwindSafe(|| channel.inbound(inbound))) {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(ChannelError::Configuration { .. })) => {
                    tracing::debug!(
                        extension_id = %binding.extension_id(),
                        "channel adapter host configuration is unavailable"
                    );
                    return IngressResponse::error(503, "temporarily_unavailable");
                }
                Ok(Err(error)) => {
                    tracing::debug!(
                        extension_id = %binding.extension_id(),
                        error = %error,
                        "channel adapter rejected verified inbound request"
                    );
                    return IngressResponse::error(400, "malformed_payload");
                }
                Err(_) => {
                    tracing::warn!(
                        extension_id = %binding.extension_id(),
                        "channel adapter panicked on verified inbound request"
                    );
                    return IngressResponse::error(503, "temporarily_unavailable");
                }
            }
        };

        // 7. Outcome handling. 2xx only after the outcome's durable boundary:
        // admission for ordinary messages, staging for provider batches.
        match outcome {
            InboundOutcome::Ignore => IngressResponse::ok(),
            InboundOutcome::Respond(response) => {
                if response.validate().is_err() || !(200..=299).contains(&response.status) {
                    tracing::warn!(
                        extension_id = %binding.extension_id(),
                        "channel adapter immediate response violated host bounds"
                    );
                    return IngressResponse::error(500, "adapter");
                }
                IngressResponse {
                    status: response.status,
                    content_type: response.content_type,
                    body: response.body,
                }
            }
            InboundOutcome::Messages(messages) => {
                let channel_egress = self.channel_egress(binding, &verified.installation_id);
                admit_messages(
                    &self.deps,
                    binding.extension_id(),
                    &verified.installation_id,
                    channel,
                    channel_egress,
                    messages,
                )
                .await
            }
            InboundOutcome::BatchFragment(fragment) => {
                let binding_fingerprint = match binding.fingerprint() {
                    Ok(fingerprint) => fingerprint,
                    Err(error) => {
                        tracing::debug!(
                            extension_id = %binding.extension_id(),
                            error = %error,
                            "channel adapter binding fingerprint is unavailable"
                        );
                        return IngressResponse::error(503, "temporarily_unavailable");
                    }
                };
                let channel_egress = self.channel_egress(binding, &verified.installation_id);
                self.batch_processor
                    .stage_and_schedule(
                        binding.extension_id(),
                        &verified.installation_id,
                        binding_fingerprint,
                        channel,
                        channel_egress,
                        *fragment,
                    )
                    .await
            }
        }
    }

    fn channel_egress(
        &self,
        binding: &ResolvedIngressBinding,
        installation_id: &str,
    ) -> Option<Arc<dyn RestrictedEgress>> {
        let transport = self.deps.channel_egress_transport.as_ref()?;
        let declared = binding
            .resolved()
            .channel
            .as_ref()
            .map(|channel| {
                channel
                    .egress
                    .iter()
                    .map(DeclaredChannelEgress::from_descriptor)
                    .collect()
            })
            .unwrap_or_default();
        Some(Arc::new(PolicyEnforcedChannelEgress::new(
            binding.extension_id(),
            installation_id,
            declared,
            Arc::clone(transport),
        )))
    }
}

async fn admit_messages(
    deps: &ExtensionIngressRouterDeps,
    extension_id: &str,
    installation_id: &str,
    channel_adapter: Arc<dyn ChannelAdapter>,
    channel_egress: Option<Arc<dyn RestrictedEgress>>,
    messages: Vec<NormalizedInboundMessage>,
) -> IngressResponse {
    if messages.is_empty() {
        return IngressResponse::ok();
    }
    for message in messages {
        if let Err(error) = message.validate() {
            tracing::debug!(
                extension_id,
                error = %error,
                "channel adapter emitted an out-of-bounds normalized message"
            );
            return IngressResponse::error(400, "malformed_payload");
        }
        // reply_context is stored host-side, keyed to the conversation
        // source binding, before the admission commit — the delivery
        // coordinator reads it back for source-route replies.
        if let Some(context) = &message.reply_context {
            let key = ReplyContextKey {
                extension_id: extension_id.to_string(),
                installation_id: installation_id.to_string(),
                conversation: message.conversation.conversation_fingerprint(),
            };
            if let Err(error) = deps.reply_context.put(key, context.clone()).await {
                tracing::debug!(
                    extension_id,
                    error = %error,
                    "reply context store unavailable"
                );
                return IngressResponse::error(503, "temporarily_unavailable");
            }
        }
        // Durable dedupe + admission commit — before any ordinary-message
        // webhook 2xx and before a staged batch is marked complete.
        match deps
            .sink
            .admit(InboundAdmission {
                extension_id: extension_id.to_string(),
                installation_id: installation_id.to_string(),
                message,
                channel_adapter: Arc::clone(&channel_adapter),
                channel_egress: channel_egress.clone(),
            })
            .await
        {
            Ok(InboundAdmissionAck::Accepted) | Ok(InboundAdmissionAck::Duplicate) => {}
            Err(error) if error.retryable => {
                tracing::debug!(
                    extension_id,
                    error = %error,
                    "inbound admission failed retryably"
                );
                return IngressResponse::error(503, "temporarily_unavailable");
            }
            Err(error) => {
                tracing::debug!(
                    extension_id,
                    error = %error,
                    "inbound admission rejected permanently"
                );
                return IngressResponse::error(400, "rejected");
            }
        }
    }
    IngressResponse::ok()
}

#[derive(Clone)]
struct InboundBatchProcessor {
    watch: SnapshotWatch,
    deployment_channels: Arc<DeploymentChannelRegistry>,
    deps: ExtensionIngressRouterDeps,
    recovery_started: Arc<AtomicBool>,
    admission_deadline: Duration,
}

type ResolvedBatchBinding = (Arc<dyn ChannelAdapter>, Option<Arc<dyn RestrictedEgress>>);

impl InboundBatchProcessor {
    async fn stage_and_schedule(
        &self,
        extension_id: &str,
        installation_id: &str,
        binding_fingerprint: String,
        channel_adapter: Arc<dyn ChannelAdapter>,
        channel_egress: Option<Arc<dyn RestrictedEgress>>,
        fragment: InboundBatchFragment,
    ) -> IngressResponse {
        if let Err(error) = fragment.validate() {
            tracing::debug!(
                extension_id,
                error = %error,
                "channel adapter emitted invalid inbound batch metadata"
            );
            return IngressResponse::error(400, "malformed_payload");
        }
        let request = InboundBatchStageRequest {
            key: InboundBatchKey {
                extension_id: extension_id.to_string(),
                installation_id: installation_id.to_string(),
                batch_key: fragment.batch_key.clone(),
            },
            binding_fingerprint,
            fragment,
            staged_at: Utc::now(),
        };
        match self.deps.inbound_batches.stage(request).await {
            Ok(InboundBatchStageOutcome::Pending(schedule)) => {
                self.spawn_schedule(schedule, channel_adapter, channel_egress);
                // The fragment itself is now durable. Providers that serialize
                // a logical batch may send the next webhook immediately.
                IngressResponse::ok()
            }
            Ok(InboundBatchStageOutcome::AlreadyCompleted) => IngressResponse::ok(),
            Ok(InboundBatchStageOutcome::Rejected) => {
                IngressResponse::error(400, "malformed_payload")
            }
            Err(error) if error.retryable => {
                tracing::debug!(
                    extension_id,
                    error = %error,
                    "inbound batch staging failed retryably"
                );
                IngressResponse::error(503, "temporarily_unavailable")
            }
            Err(error) => {
                tracing::debug!(
                    extension_id,
                    error = %error,
                    "inbound batch staging rejected the payload"
                );
                IngressResponse::error(400, "malformed_payload")
            }
        }
    }

    fn spawn_schedule(
        &self,
        schedule: InboundBatchSchedule,
        channel_adapter: Arc<dyn ChannelAdapter>,
        channel_egress: Option<Arc<dyn RestrictedEgress>>,
    ) {
        let processor = self.clone();
        tokio::spawn(async move {
            processor
                .process_schedule(schedule, channel_adapter, channel_egress)
                .await;
        });
    }

    async fn process_schedule(
        &self,
        mut schedule: InboundBatchSchedule,
        channel_adapter: Arc<dyn ChannelAdapter>,
        channel_egress: Option<Arc<dyn RestrictedEgress>>,
    ) {
        let mut retry_attempt = 0u32;
        loop {
            tokio::time::sleep(delay_until(schedule.due_at)).await;
            let claim = match self
                .deps
                .inbound_batches
                .claim_due(&schedule, Utc::now())
                .await
            {
                Ok(Some(claim)) => claim,
                Ok(None) => return,
                Err(error) => {
                    tracing::debug!(
                        extension_id = %schedule.key.extension_id,
                        error = %error,
                        "inbound batch claim is temporarily unavailable"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let response = match merge_batch_fragments(claim.fragments.clone()) {
                Ok(Some(message)) => match tokio::time::timeout(
                    self.admission_deadline,
                    admit_messages(
                        &self.deps,
                        &claim.schedule.key.extension_id,
                        &claim.schedule.key.installation_id,
                        Arc::clone(&channel_adapter),
                        channel_egress.clone(),
                        vec![message],
                    ),
                )
                .await
                {
                    Ok(response) => response,
                    Err(_) => {
                        tracing::debug!(
                            extension_id = %claim.schedule.key.extension_id,
                            "inbound batch admission exceeded its bounded deadline"
                        );
                        IngressResponse::error(503, "temporarily_unavailable")
                    }
                },
                Ok(None) => IngressResponse::ok(),
                Err(error) => {
                    tracing::debug!(
                        extension_id = %claim.schedule.key.extension_id,
                        error = %error,
                        "durably staged inbound batch fragments are inconsistent"
                    );
                    self.finish_claim(&claim, false).await;
                    return;
                }
            };
            if response.status == 200 {
                self.finish_claim(&claim, true).await;
                return;
            }
            if response.status != 503 {
                self.finish_claim(&claim, false).await;
                return;
            }

            retry_attempt = retry_attempt.saturating_add(1);
            let retry_after = Duration::from_secs(1u64 << retry_attempt.min(5));
            match self
                .deps
                .inbound_batches
                .release(&claim, Utc::now(), retry_after)
                .await
            {
                Ok(Some(next)) => schedule = next,
                Ok(None) => return,
                Err(error) => {
                    tracing::debug!(
                        extension_id = %claim.schedule.key.extension_id,
                        error = %error,
                        "inbound batch retry lease could not be released"
                    );
                    // The recovery sweep can reclaim this claim after the
                    // durable lease expires.
                    return;
                }
            }
        }
    }

    async fn finish_claim(&self, claim: &ClaimedInboundBatch, completed: bool) {
        let mut retry_attempt = 0u32;
        loop {
            let result = if completed {
                self.deps.inbound_batches.complete(claim, Utc::now()).await
            } else {
                self.deps.inbound_batches.reject(claim, Utc::now()).await
            };
            match result {
                Ok(true) | Ok(false) => return,
                Err(error) => {
                    tracing::debug!(
                        extension_id = %claim.schedule.key.extension_id,
                        error = %error,
                        "inbound batch terminal state could not be persisted"
                    );
                    retry_attempt = retry_attempt.saturating_add(1);
                    tokio::time::sleep(Duration::from_secs(1u64 << retry_attempt.min(5))).await;
                }
            }
        }
    }

    fn start_recovery_sweep(&self) {
        if self.recovery_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let processor = self.clone();
        tokio::spawn(async move {
            loop {
                processor.recover_pending().await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
    }

    async fn recover_pending(&self) {
        let schedules = match self.deps.inbound_batches.pending(Utc::now()).await {
            Ok(schedules) => schedules,
            Err(error) => {
                tracing::debug!(error = %error, "inbound batch recovery scan failed");
                return;
            }
        };
        for schedule in schedules {
            let Some((channel_adapter, channel_egress)) = self.resolve_recovery_binding(&schedule)
            else {
                tracing::debug!(
                    extension_id = %schedule.key.extension_id,
                    "inbound batch recovery binding is unavailable or changed"
                );
                continue;
            };
            self.spawn_schedule(schedule, channel_adapter, channel_egress);
        }
    }

    fn resolve_recovery_binding(
        &self,
        schedule: &InboundBatchSchedule,
    ) -> Option<ResolvedBatchBinding> {
        if let Some(binding) = self
            .deployment_channels
            .extension(&schedule.key.extension_id)
        {
            let fingerprint =
                resolved_binding_fingerprint("deployment", binding.resolved.as_ref()).ok()?;
            if fingerprint != schedule.binding_fingerprint {
                return None;
            }
            let egress = self.channel_egress_for_resolved(
                &schedule.key.extension_id,
                &schedule.key.installation_id,
                binding.resolved.as_ref(),
            );
            return Some((Arc::clone(&binding.adapter), egress));
        }
        let active = self.watch.current().extension(&schedule.key.extension_id)?;
        let fingerprint = resolved_binding_fingerprint("active", active.resolved.as_ref()).ok()?;
        if fingerprint != schedule.binding_fingerprint {
            return None;
        }
        let adapter = active.channel.clone()?;
        let egress = self.channel_egress_for_resolved(
            &schedule.key.extension_id,
            &schedule.key.installation_id,
            active.resolved.as_ref(),
        );
        Some((adapter, egress))
    }

    fn channel_egress_for_resolved(
        &self,
        extension_id: &str,
        installation_id: &str,
        resolved: &ResolvedExtensionManifest,
    ) -> Option<Arc<dyn RestrictedEgress>> {
        let transport = self.deps.channel_egress_transport.as_ref()?;
        let declared = resolved
            .channel
            .as_ref()
            .map(|channel| {
                channel
                    .egress
                    .iter()
                    .map(DeclaredChannelEgress::from_descriptor)
                    .collect()
            })
            .unwrap_or_default();
        Some(Arc::new(PolicyEnforcedChannelEgress::new(
            extension_id,
            installation_id,
            declared,
            Arc::clone(transport),
        )))
    }
}

fn delay_until(due_at: chrono::DateTime<Utc>) -> Duration {
    due_at
        .signed_duration_since(Utc::now())
        .to_std()
        .unwrap_or(Duration::ZERO)
}

fn resolved_binding_fingerprint(
    kind: &str,
    resolved: &ResolvedExtensionManifest,
) -> Result<String, serde_json::Error> {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(resolved)?);
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn merge_batch_fragments(
    mut fragments: Vec<InboundBatchFragment>,
) -> Result<Option<NormalizedInboundMessage>, ChannelError> {
    fragments.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.fragment_id.cmp(&right.fragment_id))
    });
    let Some(first) = fragments.first() else {
        return Err(ChannelError::Parse {
            reason: "inbound batch contained no fragments".to_string(),
        });
    };
    let triggered = fragments
        .iter()
        .filter(|fragment| fragment.triggered)
        .collect::<Vec<_>>();
    if triggered.is_empty() {
        return Ok(None);
    }
    for fragment in &fragments {
        if fragment.batch_key != first.batch_key
            || fragment.settle_millis != first.settle_millis
            || fragment.message.event_id != first.message.event_id
            || fragment.message.actor != first.message.actor
            || fragment.message.conversation != first.message.conversation
        {
            return Err(ChannelError::Parse {
                reason: "inbound batch fragments disagree on stable identity or timing".to_string(),
            });
        }
    }
    let canonical = triggered
        .iter()
        .rev()
        .find(|fragment| !fragment.message.text.is_empty())
        .copied()
        .unwrap_or(triggered[0]);
    if triggered
        .iter()
        .any(|fragment| fragment.message.trigger != canonical.message.trigger)
    {
        return Err(ChannelError::Parse {
            reason: "triggered inbound batch fragments disagree on trigger reason".to_string(),
        });
    }
    let mut message = canonical.message.clone();
    message.attachments = fragments
        .into_iter()
        .flat_map(|fragment| fragment.message.attachments)
        .collect();
    message.validate()?;
    Ok(Some(message))
}

enum ResolvedIngressBinding {
    Deployment(Arc<DeploymentChannelBinding>),
    Active(Arc<ActiveExtension>),
}

impl ResolvedIngressBinding {
    fn extension_id(&self) -> &str {
        match self {
            Self::Deployment(binding) => &binding.extension_id,
            Self::Active(active) => &active.extension_id,
        }
    }

    fn installation_hint(&self) -> &str {
        match self {
            // Deployment ingress has no user installation. The secrets port
            // returns the authoritative installation identity with the
            // successful verification candidate.
            Self::Deployment(binding) => &binding.extension_id,
            Self::Active(active) => &active.installation_id,
        }
    }

    fn resolved(&self) -> &ResolvedExtensionManifest {
        match self {
            Self::Deployment(binding) => binding.resolved.as_ref(),
            Self::Active(active) => active.resolved.as_ref(),
        }
    }

    fn adapter(&self) -> Option<Arc<dyn ChannelAdapter>> {
        match self {
            Self::Deployment(binding) => Some(Arc::clone(&binding.adapter)),
            Self::Active(active) => active.channel.clone(),
        }
    }

    fn fingerprint(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Deployment(binding) => {
                resolved_binding_fingerprint("deployment", binding.resolved.as_ref())
            }
            Self::Active(active) => {
                resolved_binding_fingerprint("active", active.resolved.as_ref())
            }
        }
    }
}

fn method_allowed(method: &str, ingress: &ChannelIngressDescriptor) -> bool {
    match ingress.method {
        ChannelIngressMethod::Post => method.eq_ignore_ascii_case("POST"),
    }
}

/// Token-bucket rate limiter keyed by extension id (pre-verification, the
/// installation is not yet resolved; one installation per extension today).
struct RateLimiter {
    config: IngressRateLimitConfig,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    last_refilled_at: Instant,
}

impl RateLimiter {
    fn new(config: IngressRateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn try_admit(&self, key: &str) -> bool {
        let now = Instant::now();
        let capacity = f64::from(self.config.max_requests.max(1));
        let mut buckets = match self.buckets.lock() {
            Ok(buckets) => buckets,
            Err(poisoned) => poisoned.into_inner(),
        };
        // Prune buckets idle for two windows to bound memory.
        let ttl = self.config.window.saturating_mul(2);
        buckets.retain(|_, bucket| {
            now.duration_since(bucket.last_refilled_at) < ttl || bucket.tokens < capacity
        });
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: capacity,
            last_refilled_at: now,
        });
        let elapsed = now.duration_since(bucket.last_refilled_at);
        if !elapsed.is_zero() {
            let refill_ratio = if self.config.window.is_zero() {
                1.0
            } else {
                elapsed.as_secs_f64() / self.config.window.as_secs_f64()
            };
            bucket.tokens = capacity.min(bucket.tokens + refill_ratio * capacity);
            bucket.last_refilled_at = now;
        }
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }
}
