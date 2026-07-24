//! Generic ingress router contract tests (extension-runtime P4, workstream E).
//!
//! Drives [`ExtensionIngressRouter`] over a REAL `ExtensionHost` snapshot
//! (activation publishes the route; removal unpublishes it — no router
//! rebuild) and pins the per-request order: match → method/body/rate/deadline
//! → verification → panic-isolated `inbound` → durable admission before any
//! 2xx. Checklist: ING-1/2/5/6/7/8-unit/9/11-storage; the recipe byte
//! semantics themselves are pinned by the verifier unit tests (ING-3/4).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use ironclaw_extension_host::ingress::{
    ExtensionIngressRouter, ExtensionIngressRouterDeps, InboundAdmission, InboundAdmissionAck,
    InboundSink, InboundSinkError, IngressPortError, IngressRateLimitConfig, IngressRequest,
    IngressRouterConfig, IngressSecretsPort, ReplyContextKey, ReplyContextStore,
    VerificationCandidate, canonical_ingress_path,
};
use ironclaw_extension_host::test_support::resolve_manifest_toml;
use ironclaw_extension_host::{
    ExtensionBindings, ExtensionEntrypoint, ExtensionHost, ExtensionHostDeps, ExtensionLoader,
    InstallationRecord, InstallationRecordStore, InstallationState, LifecycleError, LoadContext,
    LoadedExtension, RehydratedInstallationRecordStore, SnapshotConflict,
};
use ironclaw_host_api::SecretHandle;
use ironclaw_product::{
    ChannelAdapter, ChannelError, DeliveryReport, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ImmediateResponse, InboundOutcome, NormalizedInboundMessage, OutboundEnvelope,
    ProductTriggerReason, VerifiedInbound,
};

/// What the scripted adapter observed per call: forwarded headers, body,
/// and the resolved installation id.
type SeenInbound = (Vec<(String, String)>, Vec<u8>, String);

const EXTENSION_ID: &str = "acme-chat";
const SUFFIX: &str = "events";
const SECRET: &[u8] = b"contract-signing-secret";

/// Channel-only manifest with a small body limit and the acme-shaped
/// timestamped hmac recipe, so limit/verification ordering is observable.
fn manifest() -> ironclaw_extensions::ResolvedExtensionManifest {
    resolve_manifest_toml(
        r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme-chat"
name = "Acme Chat"
version = "0.1.0"
description = "router contract fixture"
trust = "third_party"

[admin_configuration]
group_id = "extension.acme-chat"
display_name = "Acme Chat deployment configuration"
fields = [ { handle = "acme_chat_signing_secret", label = "Signing secret", secret = true, required = false } ]

[runtime]
kind = "wasm"
module = "wasm/acme_chat.wasm"

[channel]
id = "messages"
display_name = "Acme chat"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 512

[channel.ingress.verification]
kind = "hmac_sha256"
secret_handle = "acme_chat_signing_secret"
signature_header = "X-Acme-Signature"
signature_prefix = "v0="
signature_encoding = "hex"
timestamp_header = "X-Acme-Request-Timestamp"
max_age_seconds = 300
signed_payload = [
  { literal = "v0:" },
  { header = "X-Acme-Request-Timestamp" },
  { literal = ":" },
  { body = true },
]

[[channel.egress]]
scheme = "https"
host = "api.acme.example"
methods = ["post"]
"#,
    )
}

// ── Scripted ports ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdapterMode {
    /// Parse `{"text":..., "event":..., "conversation":...}` into one message.
    Message,
    /// Message with a `reply_context` payload attached.
    MessageWithReplyContext,
    Respond,
    OversizedRespond,
    Ignore,
    Panic,
}

struct ScriptedChannelAdapter {
    mode: AdapterMode,
    inbound_calls: Arc<AtomicUsize>,
    /// Everything the adapter observed: forwarded headers and body, per call.
    seen: Arc<std::sync::Mutex<Vec<SeenInbound>>>,
}

#[async_trait]
impl ChannelAdapter for ScriptedChannelAdapter {
    fn inbound(&self, request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        self.inbound_calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().expect("seen lock").push((
            request.headers.to_vec(),
            request.body.to_vec(),
            request.installation_id.to_string(),
        ));
        match self.mode {
            AdapterMode::Panic => panic!("scripted adapter panic"),
            AdapterMode::Ignore => Ok(InboundOutcome::Ignore),
            AdapterMode::Respond => Ok(InboundOutcome::Respond(ImmediateResponse {
                status: 200,
                content_type: Some("text/plain".to_string()),
                body: b"challenge-token".to_vec(),
            })),
            AdapterMode::OversizedRespond => Ok(InboundOutcome::Respond(ImmediateResponse {
                status: 200,
                content_type: None,
                body: vec![0u8; 64 * 1024 + 1],
            })),
            AdapterMode::Message | AdapterMode::MessageWithReplyContext => {
                let value: serde_json::Value =
                    serde_json::from_slice(request.body).map_err(|error| ChannelError::Parse {
                        reason: error.to_string(),
                    })?;
                let text = value["text"].as_str().unwrap_or_default().to_string();
                let event = value["event"].as_str().unwrap_or("event-1");
                let conversation = value["conversation"].as_str().unwrap_or("conv-1");
                Ok(InboundOutcome::Messages(vec![NormalizedInboundMessage {
                    actor: ExternalActorRef::new("acme_user", "U-1", None::<&str>).expect("actor"),
                    conversation: ExternalConversationRef::new(None, conversation, None, None)
                        .expect("conversation"),
                    event_id: ExternalEventId::new(event).expect("event id"),
                    text,
                    trigger: ProductTriggerReason::DirectChat,
                    attachments: Vec::new(),
                    reply_context: matches!(self.mode, AdapterMode::MessageWithReplyContext)
                        .then(|| b"opaque-reply-route".to_vec()),
                }]))
            }
        }
    }

    async fn deliver(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn ironclaw_host_api::RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}

struct ScriptedSecrets {
    candidates: Vec<VerificationCandidate>,
    calls: Arc<AtomicUsize>,
    fail: bool,
}

#[async_trait]
impl IngressSecretsPort for ScriptedSecrets {
    async fn verification_candidates(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        _handle: Option<&SecretHandle>,
    ) -> Result<Vec<VerificationCandidate>, IngressPortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(IngressPortError {
                reason: "scripted secrets outage".to_string(),
            });
        }
        Ok(self.candidates.clone())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SinkMode {
    Accept,
    Duplicate,
    FailRetryable,
    FailPermanent,
    Hang,
}

struct RecordingSink {
    mode: SinkMode,
    admitted: Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
}

#[async_trait]
impl InboundSink for RecordingSink {
    async fn admit(
        &self,
        admission: InboundAdmission,
    ) -> Result<InboundAdmissionAck, InboundSinkError> {
        if self.mode == SinkMode::Hang {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        match self.mode {
            SinkMode::FailRetryable => Err(InboundSinkError {
                retryable: true,
                reason: "scripted retryable failure".to_string(),
            }),
            SinkMode::FailPermanent => Err(InboundSinkError {
                retryable: false,
                reason: "scripted permanent rejection".to_string(),
            }),
            _ => {
                self.admitted.lock().expect("admitted lock").push((
                    admission.extension_id,
                    admission.installation_id,
                    admission.message.event_id.as_str().to_string(),
                ));
                Ok(if self.mode == SinkMode::Duplicate {
                    InboundAdmissionAck::Duplicate
                } else {
                    InboundAdmissionAck::Accepted
                })
            }
        }
    }
}

// ── Harness ─────────────────────────────────────────────────────────────────

struct FixedLoader {
    adapter: Arc<ScriptedChannelAdapter>,
}

#[async_trait]
impl ExtensionLoader for FixedLoader {
    async fn load(
        &self,
        _ctx: &LoadContext,
    ) -> Result<LoadedExtension, ironclaw_extension_host::BindError> {
        struct Entry {
            adapter: Arc<ScriptedChannelAdapter>,
        }
        impl ExtensionEntrypoint for Entry {
            fn bind(
                &self,
                _ctx: ironclaw_extension_host::BindContext,
            ) -> Result<ExtensionBindings, ironclaw_extension_host::BindError> {
                Ok(ExtensionBindings {
                    tools: None,
                    channel: Some(Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>),
                })
            }
        }
        Ok(LoadedExtension::new(Box::new(Entry {
            adapter: Arc::clone(&self.adapter),
        })))
    }
}

struct Harness {
    host: Arc<ExtensionHost>,
    router: ExtensionIngressRouter,
    adapter_calls: Arc<AtomicUsize>,
    adapter_seen: Arc<std::sync::Mutex<Vec<SeenInbound>>>,
    secrets_calls: Arc<AtomicUsize>,
    admitted: Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
    reply_context: Arc<TestReplyContextStore>,
}

/// Process-local reply-context fake for router contract tests (production
/// wires the filesystem-backed store in composition).
#[derive(Default)]
struct TestReplyContextStore {
    entries: std::sync::Mutex<Vec<(ReplyContextKey, Vec<u8>)>>,
}

#[async_trait::async_trait]
impl ReplyContextStore for TestReplyContextStore {
    async fn put(&self, key: ReplyContextKey, context: Vec<u8>) -> Result<(), IngressPortError> {
        let mut entries = self.entries.lock().expect("reply-context fake lock");
        entries.retain(|(existing, _)| existing != &key);
        entries.push((key, context));
        Ok(())
    }

    async fn get(&self, key: &ReplyContextKey) -> Result<Option<Vec<u8>>, IngressPortError> {
        let entries = self.entries.lock().expect("reply-context fake lock");
        Ok(entries
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, context)| context.clone()))
    }
}

struct HarnessOptions {
    adapter_mode: AdapterMode,
    sink_mode: SinkMode,
    candidates: Vec<VerificationCandidate>,
    secrets_fail: bool,
    config: IngressRouterConfig,
    reserved_routes: std::collections::BTreeSet<String>,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self {
            adapter_mode: AdapterMode::Message,
            sink_mode: SinkMode::Accept,
            candidates: vec![VerificationCandidate {
                installation_id: format!("{EXTENSION_ID}-install"),
                secret: SECRET.to_vec(),
            }],
            secrets_fail: false,
            config: IngressRouterConfig {
                rate_limit: IngressRateLimitConfig {
                    max_requests: 1000,
                    window: Duration::from_secs(60),
                },
                request_deadline: Duration::from_millis(500),
            },
            reserved_routes: Default::default(),
        }
    }
}

async fn harness(options: HarnessOptions) -> Harness {
    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let adapter_seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let adapter = Arc::new(ScriptedChannelAdapter {
        mode: options.adapter_mode,
        inbound_calls: Arc::clone(&adapter_calls),
        seen: Arc::clone(&adapter_seen),
    });
    let store = Arc::new(RehydratedInstallationRecordStore::default());
    let host = Arc::new(
        ExtensionHost::new(ExtensionHostDeps {
            store: Arc::clone(&store) as Arc<dyn InstallationRecordStore>,
            loader: Arc::new(FixedLoader {
                adapter: Arc::clone(&adapter),
            }),
            drain: Arc::new(ironclaw_extension_host::test_support::RecordingDrain::default()),
            egress: Arc::new(ironclaw_extension_host::test_support::FakeEgressFactory),
            reserved_capability_ids: Default::default(),
            reserved_ingress_routes: options.reserved_routes,
            hook_deadline: Duration::from_secs(5),
        })
        .await,
    );
    let secrets_calls = Arc::new(AtomicUsize::new(0));
    let admitted = Arc::new(std::sync::Mutex::new(Vec::new()));
    let reply_context = Arc::new(TestReplyContextStore::default());
    let router = ExtensionIngressRouter::new(
        host.snapshot_watch(),
        ExtensionIngressRouterDeps {
            secrets: Arc::new(ScriptedSecrets {
                candidates: options.candidates,
                calls: Arc::clone(&secrets_calls),
                fail: options.secrets_fail,
            }),
            sink: Arc::new(RecordingSink {
                mode: options.sink_mode,
                admitted: Arc::clone(&admitted),
            }),
            reply_context: Arc::clone(&reply_context) as Arc<dyn ReplyContextStore>,
        },
        options.config,
    );
    Harness {
        host,
        router,
        adapter_calls,
        adapter_seen,
        secrets_calls,
        admitted,
        reply_context,
    }
}

async fn activate(harness: &Harness) {
    harness
        .host
        .install(InstallationRecord {
            extension_id: EXTENSION_ID.to_string(),
            installation_id: format!("{EXTENSION_ID}-install"),
            state: InstallationState::Installed,
            resolved: Arc::new(manifest()),
            config: Vec::new(),
            last_error: None,
        })
        .await
        .expect("install");
    harness.host.activate(EXTENSION_ID).await.expect("activate");
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

fn sign(timestamp: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET).expect("hmac key");
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("v0={hex}")
}

fn signed_request(body: &[u8]) -> IngressRequest {
    let timestamp = now_unix().to_string();
    let signature = sign(&timestamp, body);
    IngressRequest {
        method: "POST".to_string(),
        extension_id: EXTENSION_ID.to_string(),
        route_suffix: SUFFIX.to_string(),
        headers: vec![
            ("X-Acme-Signature".to_string(), signature.into_bytes()),
            (
                "X-Acme-Request-Timestamp".to_string(),
                timestamp.into_bytes(),
            ),
            ("Content-Type".to_string(), b"application/json".to_vec()),
        ],
        body: body.to_vec(),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// ING-1: the route table is the active snapshot — activation serves the
/// route, removal 404s it, with no router rebuild in between.
#[tokio::test]
async fn route_table_follows_snapshot_swaps_without_router_rebuild() {
    let harness = harness(HarnessOptions::default()).await;
    let body = br#"{"text":"hi","event":"ev-1","conversation":"C-1"}"#;

    // Before activation: no route.
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        404
    );

    activate(&harness).await;
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        200
    );

    // Wrong suffix and unknown extension stay unmatched.
    let mut wrong_suffix = signed_request(body);
    wrong_suffix.route_suffix = "other".to_string();
    assert_eq!(harness.router.handle(wrong_suffix).await.status, 404);
    let mut wrong_extension = signed_request(body);
    wrong_extension.extension_id = "unknown-ext".to_string();
    assert_eq!(harness.router.handle(wrong_extension).await.status, 404);

    // Deactivation unpublishes the route through the same router value.
    harness
        .host
        .deactivate(EXTENSION_ID)
        .await
        .expect("deactivate");
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        404
    );
}

/// ING-1: a canonical route colliding with a fixed host route fails
/// activation with a typed conflict.
#[tokio::test]
async fn activation_rejects_collision_with_fixed_host_routes() {
    let mut options = HarnessOptions::default();
    options
        .reserved_routes
        .insert(canonical_ingress_path(EXTENSION_ID, SUFFIX));
    let harness = harness(options).await;
    harness
        .host
        .install(InstallationRecord {
            extension_id: EXTENSION_ID.to_string(),
            installation_id: format!("{EXTENSION_ID}-install"),
            state: InstallationState::Installed,
            resolved: Arc::new(manifest()),
            config: Vec::new(),
            last_error: None,
        })
        .await
        .expect("install");
    let error = harness
        .host
        .activate(EXTENSION_ID)
        .await
        .expect_err("reserved route must fail activation");
    assert!(matches!(
        error,
        LifecycleError::Conflict(SnapshotConflict::ReservedRoute { .. })
    ));
}

/// ING-2: method, body limit, and rate limit are enforced BEFORE any
/// verification (secrets untouched) or adapter work.
#[tokio::test]
async fn method_body_and_rate_limits_run_before_verification_and_adapter() {
    let mut options = HarnessOptions::default();
    options.config.rate_limit = IngressRateLimitConfig {
        max_requests: 2,
        window: Duration::from_secs(3600),
    };
    let harness = harness(options).await;
    activate(&harness).await;
    let body = br#"{"text":"hi"}"#;

    // Wrong method → 405, nothing else runs.
    let mut request = signed_request(body);
    request.method = "GET".to_string();
    assert_eq!(harness.router.handle(request).await.status, 405);

    // Oversized body (limit 512) → 413, nothing else runs.
    let mut request = signed_request(&vec![b'x'; 513]);
    request.body = vec![b'x'; 513];
    assert_eq!(harness.router.handle(request).await.status, 413);

    assert_eq!(harness.secrets_calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.adapter_calls.load(Ordering::SeqCst), 0);

    // Rate limit (2 per window): the third POST is rejected before
    // verification — secrets consulted exactly twice.
    let body = br#"{"text":"hi","event":"ev-rate","conversation":"C-1"}"#;
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        200
    );
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        200
    );
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        429
    );
    assert_eq!(harness.secrets_calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.adapter_calls.load(Ordering::SeqCst), 2);
}

/// ING-3 (router leg): bad, missing, stale, and replay-window signatures are
/// rejected 401 before the adapter runs; a genuine signature passes.
#[tokio::test]
async fn verification_rejects_bad_missing_and_stale_signatures_before_the_adapter() {
    let harness = harness(HarnessOptions::default()).await;
    activate(&harness).await;
    let body = br#"{"text":"hi","event":"ev-2","conversation":"C-1"}"#;

    // Missing signature.
    let mut request = signed_request(body);
    request
        .headers
        .retain(|(name, _)| name != "X-Acme-Signature");
    assert_eq!(harness.router.handle(request).await.status, 401);

    // Tampered body under a valid-for-other-bytes signature.
    let mut request = signed_request(body);
    request.body = br#"{"text":"tampered"}"#.to_vec();
    assert_eq!(harness.router.handle(request).await.status, 401);

    // Stale timestamp outside the 300s window (correctly signed replay).
    let stale_ts = (now_unix() - 301).to_string();
    let stale_sig = sign(&stale_ts, body);
    let request = IngressRequest {
        method: "POST".to_string(),
        extension_id: EXTENSION_ID.to_string(),
        route_suffix: SUFFIX.to_string(),
        headers: vec![
            ("X-Acme-Signature".to_string(), stale_sig.into_bytes()),
            (
                "X-Acme-Request-Timestamp".to_string(),
                stale_ts.into_bytes(),
            ),
        ],
        body: body.to_vec(),
    };
    assert_eq!(harness.router.handle(request).await.status, 401);

    assert_eq!(harness.adapter_calls.load(Ordering::SeqCst), 0);
    assert!(harness.admitted.lock().expect("admitted").is_empty());

    // The genuine request still verifies.
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        200
    );
    assert_eq!(harness.adapter_calls.load(Ordering::SeqCst), 1);
}

/// ING-5 + ING-7: the adapter sees bounded input with the verification
/// headers consumed — the signing secret is not observable anywhere in its
/// inputs.
#[tokio::test]
async fn adapter_never_observes_verification_headers_or_secret_material() {
    let harness = harness(HarnessOptions::default()).await;
    activate(&harness).await;
    let body = br#"{"text":"hi","event":"ev-3","conversation":"C-1"}"#;
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        200
    );

    let seen = harness.adapter_seen.lock().expect("seen");
    let (headers, seen_body, installation_id) = &seen[0];
    assert_eq!(installation_id, &format!("{EXTENSION_ID}-install"));
    assert_eq!(seen_body.as_slice(), body);
    assert!(
        headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("X-Acme-Signature")
                && !name.eq_ignore_ascii_case("X-Acme-Request-Timestamp")),
        "verification headers must be consumed by the host, got {headers:?}"
    );
    // Non-verification headers are forwarded.
    assert!(headers.iter().any(|(name, _)| name == "Content-Type"));
    // The secret bytes appear nowhere in the adapter's observable inputs.
    let secret_text = String::from_utf8_lossy(SECRET).into_owned();
    let rendered = format!("{headers:?}{}", String::from_utf8_lossy(seen_body));
    assert!(!rendered.contains(&secret_text));
}

/// ING-6 (router leg): with multiple candidate installations the request
/// resolves the one whose secret verifies; two verifying candidates are
/// ambiguous and fail closed.
#[tokio::test]
async fn multi_candidate_verification_resolves_exactly_one_installation() {
    let options = HarnessOptions {
        candidates: vec![
            VerificationCandidate {
                installation_id: "other-install".to_string(),
                secret: b"other-secret".to_vec(),
            },
            VerificationCandidate {
                installation_id: format!("{EXTENSION_ID}-install"),
                secret: SECRET.to_vec(),
            },
        ],
        ..HarnessOptions::default()
    };
    let harness = harness(options).await;
    activate(&harness).await;
    let body = br#"{"text":"hi","event":"ev-4","conversation":"C-1"}"#;
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        200
    );
    let admitted = harness.admitted.lock().expect("admitted").clone();
    assert_eq!(
        admitted,
        vec![(
            EXTENSION_ID.to_string(),
            format!("{EXTENSION_ID}-install"),
            "ev-4".to_string()
        )]
    );

    // Ambiguity: both candidates share the verifying secret → 401.
    let options = HarnessOptions {
        candidates: vec![
            VerificationCandidate {
                installation_id: "install-a".to_string(),
                secret: SECRET.to_vec(),
            },
            VerificationCandidate {
                installation_id: "install-b".to_string(),
                secret: SECRET.to_vec(),
            },
        ],
        ..HarnessOptions::default()
    };
    let ambiguous = harness_with_activation(options).await;
    assert_eq!(
        ambiguous.router.handle(signed_request(body)).await.status,
        401
    );
    assert_eq!(ambiguous.adapter_calls.load(Ordering::SeqCst), 0);
}

async fn harness_with_activation(options: HarnessOptions) -> Harness {
    let harness = harness(options).await;
    activate(&harness).await;
    harness
}

/// ING-7: a panicking adapter is isolated — the request fails 503 and the
/// router keeps serving.
#[tokio::test]
async fn adapter_panic_is_isolated_and_the_router_survives() {
    let harness = harness_with_activation(HarnessOptions {
        adapter_mode: AdapterMode::Panic,
        ..HarnessOptions::default()
    })
    .await;
    let body = br#"{"text":"boom"}"#;
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        503
    );
    assert!(harness.admitted.lock().expect("admitted").is_empty());
    // Still serving afterwards.
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        503
    );
}

/// ING-8 (unit leg): 2xx only after the durable admission commit — a failing
/// sink yields retryable 503 / permanent 400 with nothing acked; a duplicate
/// commit still acks 200.
#[tokio::test]
async fn two_hundred_only_after_durable_admission_commit() {
    let body = br#"{"text":"hi","event":"ev-5","conversation":"C-1"}"#;

    let accept = harness_with_activation(HarnessOptions::default()).await;
    let response = accept.router.handle(signed_request(body)).await;
    assert_eq!(response.status, 200);
    assert_eq!(accept.admitted.lock().expect("admitted").len(), 1);

    let duplicate = harness_with_activation(HarnessOptions {
        sink_mode: SinkMode::Duplicate,
        ..HarnessOptions::default()
    })
    .await;
    assert_eq!(
        duplicate.router.handle(signed_request(body)).await.status,
        200
    );

    let retryable = harness_with_activation(HarnessOptions {
        sink_mode: SinkMode::FailRetryable,
        ..HarnessOptions::default()
    })
    .await;
    assert_eq!(
        retryable.router.handle(signed_request(body)).await.status,
        503
    );

    let permanent = harness_with_activation(HarnessOptions {
        sink_mode: SinkMode::FailPermanent,
        ..HarnessOptions::default()
    })
    .await;
    assert_eq!(
        permanent.router.handle(signed_request(body)).await.status,
        400
    );

    // A secrets-port outage is a retryable 503, never an unauthenticated 401.
    let outage = harness_with_activation(HarnessOptions {
        secrets_fail: true,
        ..HarnessOptions::default()
    })
    .await;
    assert_eq!(outage.router.handle(signed_request(body)).await.status, 503);
    assert_eq!(outage.adapter_calls.load(Ordering::SeqCst), 0);
}

/// ING-2 (deadline leg): a hanging admission exceeds the bounded request
/// deadline and fails 503 instead of holding the connection open.
#[tokio::test]
async fn request_deadline_bounds_verification_through_admission() {
    let harness = harness_with_activation(HarnessOptions {
        sink_mode: SinkMode::Hang,
        ..HarnessOptions::default()
    })
    .await;
    let body = br#"{"text":"hi","event":"ev-6","conversation":"C-1"}"#;
    let started = std::time::Instant::now();
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        503
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// ING-9: a `Respond` outcome answers immediately after verification with no
/// enqueue, and out-of-bounds responses are rejected host-side.
#[tokio::test]
async fn respond_outcome_answers_without_enqueue_within_bounds() {
    let harness = harness_with_activation(HarnessOptions {
        adapter_mode: AdapterMode::Respond,
        ..HarnessOptions::default()
    })
    .await;
    let response = harness.router.handle(signed_request(b"{}")).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"challenge-token");
    assert!(harness.admitted.lock().expect("admitted").is_empty());

    let oversized = harness_with_activation(HarnessOptions {
        adapter_mode: AdapterMode::OversizedRespond,
        ..HarnessOptions::default()
    })
    .await;
    assert_eq!(
        oversized.router.handle(signed_request(b"{}")).await.status,
        500
    );
}

/// Ignore outcome: authenticated no-op acks 200 without admission.
#[tokio::test]
async fn ignore_outcome_acks_without_admission() {
    let harness = harness_with_activation(HarnessOptions {
        adapter_mode: AdapterMode::Ignore,
        ..HarnessOptions::default()
    })
    .await;
    assert_eq!(
        harness.router.handle(signed_request(b"{}")).await.status,
        200
    );
    assert!(harness.admitted.lock().expect("admitted").is_empty());
}

/// ING-11 (storage leg): `reply_context` is stored host-side keyed to the
/// conversation source binding before the admission commit.
#[tokio::test]
async fn reply_context_is_stored_host_side_keyed_by_conversation() {
    let harness = harness_with_activation(HarnessOptions {
        adapter_mode: AdapterMode::MessageWithReplyContext,
        ..HarnessOptions::default()
    })
    .await;
    let body = br#"{"text":"hi","event":"ev-7","conversation":"C-777"}"#;
    assert_eq!(
        harness.router.handle(signed_request(body)).await.status,
        200
    );

    let conversation = ExternalConversationRef::new(None, "C-777", None, None)
        .expect("conversation")
        .conversation_fingerprint();
    let stored = harness
        .reply_context
        .get(&ReplyContextKey {
            extension_id: EXTENSION_ID.to_string(),
            installation_id: format!("{EXTENSION_ID}-install"),
            conversation,
        })
        .await
        .expect("reply context store readable");
    assert_eq!(stored.as_deref(), Some(b"opaque-reply-route".as_slice()));
}
