//! Transport-neutral generic ingress router.
//!
//! See the module docs in [`super`] for the pinned per-request order. The
//! router owns semantics and security; the extension contributes exactly one
//! pure call (`ChannelAdapter::inbound`). Everything durable (dedupe +
//! admission commit) happens through the injected [`InboundSink`] **before**
//! any 2xx leaves this router (checklist ING-8).

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ironclaw_host_api::{ChannelIngressDescriptor, ChannelIngressMethod, SecretHandle};
use ironclaw_product_adapters::{InboundOutcome, NormalizedInboundMessage, VerifiedInbound};

use crate::active::ActiveExtension;
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

/// One verified, normalized inbound message ready for durable admission.
pub struct InboundAdmission {
    pub extension_id: String,
    pub installation_id: String,
    pub message: NormalizedInboundMessage,
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
    /// ([`ironclaw_product_adapters::ExternalConversationRef::conversation_fingerprint`]).
    pub conversation: String,
}

/// Host-side `reply_context` storage. Stored before admission commits;
/// the delivery coordinator (P5) reads it back for source-route replies.
#[async_trait]
pub trait ReplyContextStore: Send + Sync {
    async fn put(&self, key: ReplyContextKey, context: Vec<u8>) -> Result<(), IngressPortError>;
    async fn get(&self, key: &ReplyContextKey) -> Result<Option<Vec<u8>>, IngressPortError>;
}

/// Bounded in-memory reply-context store (latest context per conversation;
/// FIFO eviction beyond the cap).
pub struct InMemoryReplyContextStore {
    entries: RwLock<Vec<(ReplyContextKey, Vec<u8>)>>,
    cap: usize,
}

impl Default for InMemoryReplyContextStore {
    fn default() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            cap: 1024,
        }
    }
}

#[async_trait]
impl ReplyContextStore for InMemoryReplyContextStore {
    async fn put(&self, key: ReplyContextKey, context: Vec<u8>) -> Result<(), IngressPortError> {
        let mut entries = match self.entries.write() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.retain(|(existing, _)| existing != &key);
        entries.push((key, context));
        if entries.len() > self.cap {
            let excess = entries.len() - self.cap;
            entries.drain(0..excess);
        }
        Ok(())
    }

    async fn get(&self, key: &ReplyContextKey) -> Result<Option<Vec<u8>>, IngressPortError> {
        let entries = match self.entries.read() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        Ok(entries
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, context)| context.clone()))
    }
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
pub struct ExtensionIngressRouterDeps {
    pub secrets: Arc<dyn IngressSecretsPort>,
    pub sink: Arc<dyn InboundSink>,
    pub reply_context: Arc<dyn ReplyContextStore>,
}

/// The generic ingress router. One instance serves every active extension's
/// channel ingress; resolution is per request through the snapshot watch.
pub struct ExtensionIngressRouter {
    watch: SnapshotWatch,
    deps: ExtensionIngressRouterDeps,
    config: IngressRouterConfig,
    rate: RateLimiter,
}

impl ExtensionIngressRouter {
    pub fn new(
        watch: SnapshotWatch,
        deps: ExtensionIngressRouterDeps,
        config: IngressRouterConfig,
    ) -> Self {
        Self {
            watch,
            deps,
            rate: RateLimiter::new(config.rate_limit),
            config,
        }
    }

    /// Handle one request following the pinned order. Never panics; never
    /// returns 2xx before the durable admission commit.
    pub async fn handle(&self, request: IngressRequest) -> IngressResponse {
        // 1. Match against the ACTIVE snapshot (no HTTP rebuild on swap).
        let snapshot = self.watch.current();
        let Some(active) =
            snapshot.resolve_channel_ingress(&request.extension_id, &request.route_suffix)
        else {
            return IngressResponse::error(404, "unknown_route");
        };
        let Some(ingress) = active
            .resolved
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
            self.verify_and_dispatch(&request, &active, ingress),
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
        active: &Arc<ActiveExtension>,
        ingress: &ChannelIngressDescriptor,
    ) -> IngressResponse {
        // 4. Verification recipe execution — host-side, before the adapter.
        let candidates = match self
            .deps
            .secrets
            .verification_candidates(
                &active.extension_id,
                &active.installation_id,
                ingress.verification.secret_handle(),
            )
            .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::debug!(
                    extension_id = %active.extension_id,
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
                    extension_id = %active.extension_id,
                    failure = %failure,
                    "extension ingress verification rejected"
                );
                return IngressResponse::error(401, "authentication");
            }
        };
        drop(candidates); // secrets leave scope before any adapter work

        // 5. adapter.inbound — pure, panic-isolated; verification headers are
        //    consumed by the host and never forwarded.
        let Some(channel) = active.channel.clone() else {
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
                extension_id: &active.extension_id,
                installation_id: &verified.installation_id,
                body: &request.body,
                headers: &forwarded_headers,
            };
            match catch_unwind(AssertUnwindSafe(|| channel.inbound(inbound))) {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => {
                    tracing::debug!(
                        extension_id = %active.extension_id,
                        error = %error,
                        "channel adapter rejected verified inbound request"
                    );
                    return IngressResponse::error(400, "malformed_payload");
                }
                Err(_) => {
                    tracing::warn!(
                        extension_id = %active.extension_id,
                        "channel adapter panicked on verified inbound request"
                    );
                    return IngressResponse::error(503, "temporarily_unavailable");
                }
            }
        };

        // 6. Outcome handling. 2xx only after durable commits.
        match outcome {
            InboundOutcome::Ignore => IngressResponse::ok(),
            InboundOutcome::Respond(response) => {
                if response.validate().is_err() || !(200..=299).contains(&response.status) {
                    tracing::warn!(
                        extension_id = %active.extension_id,
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
                self.admit_messages(active, &verified.installation_id, messages)
                    .await
            }
        }
    }

    async fn admit_messages(
        &self,
        active: &Arc<ActiveExtension>,
        installation_id: &str,
        messages: Vec<NormalizedInboundMessage>,
    ) -> IngressResponse {
        if messages.is_empty() {
            return IngressResponse::ok();
        }
        for message in messages {
            if let Err(error) = message.validate() {
                tracing::debug!(
                    extension_id = %active.extension_id,
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
                    extension_id: active.extension_id.clone(),
                    installation_id: installation_id.to_string(),
                    conversation: message.conversation.conversation_fingerprint(),
                };
                if let Err(error) = self.deps.reply_context.put(key, context.clone()).await {
                    tracing::debug!(
                        extension_id = %active.extension_id,
                        error = %error,
                        "reply context store unavailable"
                    );
                    return IngressResponse::error(503, "temporarily_unavailable");
                }
            }
            // Durable dedupe + admission commit — before any 2xx.
            match self
                .deps
                .sink
                .admit(InboundAdmission {
                    extension_id: active.extension_id.clone(),
                    installation_id: installation_id.to_string(),
                    message,
                })
                .await
            {
                Ok(InboundAdmissionAck::Accepted) | Ok(InboundAdmissionAck::Duplicate) => {}
                Err(error) if error.retryable => {
                    tracing::debug!(
                        extension_id = %active.extension_id,
                        error = %error,
                        "inbound admission failed retryably"
                    );
                    return IngressResponse::error(503, "temporarily_unavailable");
                }
                Err(error) => {
                    tracing::debug!(
                        extension_id = %active.extension_id,
                        error = %error,
                        "inbound admission rejected permanently"
                    );
                    return IngressResponse::error(400, "rejected");
                }
            }
        }
        IngressResponse::ok()
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
