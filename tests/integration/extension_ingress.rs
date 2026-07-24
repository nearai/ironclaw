//! Reborn integration test — the generic channel ingress router (P4, ING).
//!
//! Drives the REAL production mount (`extension_ingress_route_mount` over the
//! composed runtime's snapshot watch — the same `PublicRouteMount` `serve`
//! installs) with the invented-vendor acme fixture: the extension installs and
//! activates through the production lifecycle tools, its manifest's
//! `hmac_sha256` recipe verifies the signed vendor POST host-side, the
//! fixture's real channel adapter normalizes the payload, and the generic
//! inbound sink commits durable dedupe + admission through the REAL
//! `DefaultProductSurface` (idempotency ledger → conversation binding → turn
//! submission) before the 2xx leaves the router.
//!
//! Pinned here: ING-1 (route from the active snapshot on the production
//! mount), ING-3 (bad/stale signatures rejected on the wire), ING-8
//! (ack-after-durable-commit; duplicate + fresh-sink "restart" replay
//! converge exactly once, matrixed over libSQL and PostgreSQL), ING-9
//! (challenge response), ING-10 (signed vendor POST → turn admitted through
//! the existing workflow), and the acme half of ING-12.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::BodyExt;
use ironclaw_host_api::ChannelInboundProductSurface;
use ironclaw_reborn_composition::{
    ChannelInboundSinkConfig, ChannelIngressRegistration, ExtensionIngressParts,
    GenericChannelInboundSink, PostAdmissionObserver, StaticIngressSecrets, VerifiedEvidenceMint,
    extension_ingress_route_mount,
};
use reborn_support::builder::StorageMode;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use rstest::rstest;
use serde_json::json;
use sha2::Sha256;
use tower::ServiceExt;

const ACME_ROUTE: &str = "/webhooks/extensions/acme-messenger/events";
const ACME_INSTALLATION: &str = "acme-install-1";
const ACME_SECRET: &[u8] = b"itest-acme-signing-secret";

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

/// Sign a body exactly as the acme manifest's recipe declares:
/// hex HMAC-SHA256 over `v0:{timestamp}:{body}` with a `v0=` prefix.
fn acme_signature(timestamp: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(ACME_SECRET).expect("hmac key");
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body.as_bytes());
    let digest = mac.finalize().into_bytes();
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("v0={hex}")
}

/// Records every post-admission workflow ack — the turn-admission seam
/// (`ProductInboundAck::Accepted { submitted_run_id, .. }` is the proof the
/// message entered the existing binding + turn-submission pipeline).
#[derive(Default)]
struct RecordingAdmissionObserver {
    acks: Mutex<Vec<ironclaw_product::ProductInboundAck>>,
    errors: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PostAdmissionObserver for RecordingAdmissionObserver {
    async fn observe_ack(
        &self,
        _envelope: ironclaw_product::ProductInboundEnvelope,
        ack: ironclaw_product::ProductInboundAck,
    ) {
        self.acks.lock().expect("acks lock").push(ack);
    }

    async fn observe_error(
        &self,
        _envelope: ironclaw_product::ProductInboundEnvelope,
        error: ironclaw_product::ProductAdapterError,
    ) {
        self.errors
            .lock()
            .expect("errors lock")
            .push(format!("{error:?}"));
    }
}

struct AcmeIngress {
    parts: ExtensionIngressParts,
    mount: ironclaw_reborn_composition::PublicRouteMount,
    observer: Arc<RecordingAdmissionObserver>,
}

impl AcmeIngress {
    /// Register the acme inbound wiring (static verification secret + the
    /// generic sink over THIS thread harness's real workflow) and build the
    /// production route mount. Re-registering with a fresh sink over another
    /// workflow instance models a process restart over the same durable
    /// ledger.
    fn register(
        parts: ExtensionIngressParts,
        harness: &reborn_support::builder::RebornIntegrationHarness,
    ) -> Self {
        let observer = Arc::new(RecordingAdmissionObserver::default());
        let surface = harness.product_workflow_for_test() as Arc<dyn ChannelInboundProductSurface>;
        let sink = Arc::new(GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id: ironclaw_product::ProductAdapterId::new("acme-messenger")
                .expect("adapter id"),
            evidence: VerifiedEvidenceMint::RequestSignature {
                signature_header: "X-Acme-Signature".to_string(),
                timestamp_header: Some("X-Acme-Request-Timestamp".to_string()),
            },
            surface,
            observer: Some(Arc::clone(&observer) as Arc<dyn PostAdmissionObserver>),
        }));
        parts.registry.register(
            "acme-messenger",
            ChannelIngressRegistration {
                secrets: Arc::new(StaticIngressSecrets::new(vec![
                    ironclaw_extension_host::ingress::VerificationCandidate {
                        installation_id: ACME_INSTALLATION.to_string(),
                        secret: ACME_SECRET.to_vec(),
                    },
                ])),
                sink: sink.clone() as Arc<dyn ironclaw_extension_host::ingress::InboundSink>,
                drain: Some(sink as Arc<dyn ironclaw_reborn_composition::ChannelIngressDrain>),
            },
        );
        let mount = extension_ingress_route_mount(&parts).expect("production mount builds");
        Self {
            parts,
            mount,
            observer,
        }
    }

    async fn post(
        &self,
        body: &str,
        headers: Vec<(&'static str, String)>,
    ) -> (StatusCode, Vec<u8>) {
        let mut builder = Request::builder().method("POST").uri(ACME_ROUTE);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        let response = self
            .mount
            .router
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).expect("request"))
            .await
            .expect("router responds");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        (status, bytes.to_vec())
    }

    async fn post_signed(&self, body: &str) -> (StatusCode, Vec<u8>) {
        let timestamp = now_unix().to_string();
        let signature = acme_signature(&timestamp, body);
        self.post(
            body,
            vec![
                ("X-Acme-Signature", signature),
                ("X-Acme-Request-Timestamp", timestamp),
            ],
        )
        .await
    }

    async fn drain(&self) {
        self.parts.registry.drain().await;
    }

    fn accepted_count(&self) -> usize {
        self.observer
            .acks
            .lock()
            .expect("acks lock")
            .iter()
            .filter(|ack| matches!(ack, ironclaw_product::ProductInboundAck::Accepted { .. }))
            .count()
    }

    fn duplicate_count(&self) -> usize {
        self.observer
            .acks
            .lock()
            .expect("acks lock")
            .iter()
            .filter(|ack| matches!(ack, ironclaw_product::ProductInboundAck::Duplicate { .. }))
            .count()
    }
}

/// Install the acme fixture through the production lifecycle tool (which
/// completes readiness internally), then return the runtime's REAL ingress.
async fn activate_acme(group: &RebornIntegrationGroup) -> ExtensionIngressParts {
    let lifecycle = group
        .thread("conv-acme-ingress-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "acme-messenger"}),
            ),
            RebornScriptedReply::text("installed and ready"),
        ])
        .build()
        .await
        .expect("lifecycle thread builds");
    lifecycle
        .seed_capability_credential_account("acme", "acme ingress account", &["notes:write"])
        .await
        .expect("seed acme account");
    lifecycle
        .submit_turn("install the acme messenger extension")
        .await
        .expect("install turn completes");
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await
        .expect("install completed readiness and publication");

    group
        .capability_harness()
        .expect("host-runtime capability harness")
        .reborn_services_for_test()
        .expect("composed reborn services")
        .extension_ingress_parts()
        .expect("composition built the generic ingress")
}

fn message_body(event_id: &str, text: &str) -> String {
    json!({
        "type": "message",
        "event_id": event_id,
        "conversation": "C-ACME-INGRESS",
        "user": "U-ACME-1",
        "text": text,
    })
    .to_string()
}

/// The full inbound pipeline on the production mount: signed vendor POST →
/// recipe verification → adapter normalization → durable admission → turn
/// admitted (ING-1/3/9/10 + the acme half of ING-12).
#[tokio::test]
async fn signed_acme_post_flows_through_the_production_mount_into_a_turn() {
    let group = RebornIntegrationGroup::extension_runtime_acme()
        .await
        .expect("acme group builds");
    let parts = activate_acme(&group).await;

    // The inbound thread: its scripted reply is never consulted (the router
    // path ends at turn admission), but its workflow is the REAL per-thread
    // DefaultProductSurface the sink submits through.
    let inbound_thread = group
        .thread("conv-acme-ingress-inbound")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("inbound thread builds");
    let ingress = AcmeIngress::register(parts, &inbound_thread);

    // Unknown extension/suffix stay unmatched on the production mount.
    let (status, _) = ingress.post_signed("{}").await;
    assert_eq!(status, StatusCode::OK, "active extension route serves");
    let mut unknown = Request::builder()
        .method("POST")
        .uri("/webhooks/extensions/unknown-ext/events");
    unknown = unknown.header("X-Acme-Signature", "v0=00");
    let response = ingress
        .mount
        .router
        .clone()
        .oneshot(unknown.body(Body::from("{}")).expect("request"))
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Bad signature and stale timestamp are rejected on the wire, before any
    // adapter or admission work.
    let body = message_body("Ev-acme-reject", "should not land");
    let (status, _) = ingress
        .post(
            &body,
            vec![
                ("X-Acme-Signature", "v0=deadbeef".to_string()),
                ("X-Acme-Request-Timestamp", now_unix().to_string()),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let stale_ts = (now_unix() - 301).to_string();
    let stale_sig = acme_signature(&stale_ts, &body);
    let (status, _) = ingress
        .post(
            &body,
            vec![
                ("X-Acme-Signature", stale_sig),
                ("X-Acme-Request-Timestamp", stale_ts),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    ingress.drain().await;
    assert_eq!(
        ingress.accepted_count(),
        0,
        "rejected requests must not admit"
    );

    // Challenge → immediate bounded response, no admission (ING-9).
    let challenge = json!({"type": "challenge", "challenge": "acme-challenge-token"}).to_string();
    let (status, body_bytes) = ingress.post_signed(&challenge).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_bytes, b"acme-challenge-token");
    ingress.drain().await;
    assert_eq!(ingress.accepted_count(), 0, "challenge must not enqueue");

    // The genuine signed message: verified, normalized, durably admitted, and
    // submitted as a REAL turn through the existing workflow.
    let body = message_body("Ev-acme-1", "hello from the acme vendor");
    let (status, _) = ingress.post_signed(&body).await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;
    assert_eq!(
        ingress.accepted_count(),
        1,
        "the signed vendor POST must be admitted as a turn (errors: {:?})",
        ingress.observer.errors.lock().expect("errors lock")
    );
}

/// ING-8: the durable dedupe/admission commit converges exactly once across
/// a same-sink duplicate delivery AND a fresh-sink replay over the same
/// durable ledger (a process-restart stand-in), matrixed over libSQL and
/// PostgreSQL (provisioning failure is a test failure, never a skip).
#[rstest]
#[case(StorageMode::LibSql)]
#[case(StorageMode::Postgres)]
#[tokio::test]
async fn duplicate_and_restart_replay_converge_exactly_once(#[case] storage: StorageMode) {
    let group = RebornIntegrationGroup::builder()
        .storage(storage)
        .extension_runtime_acme()
        .await
        .expect("acme group builds on this backend");
    let parts = activate_acme(&group).await;

    let inbound_thread = group
        .thread("conv-acme-ingress-dedupe")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("inbound thread builds");
    let ingress = AcmeIngress::register(parts.clone(), &inbound_thread);

    let body = message_body("Ev-acme-dedupe", "deliver exactly once");
    let (status, _) = ingress.post_signed(&body).await;
    assert_eq!(status, StatusCode::OK);
    // Vendor redelivery of the same event through the same sink: 2xx, no
    // second admission.
    let (status, _) = ingress.post_signed(&body).await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;
    assert_eq!(
        ingress.accepted_count(),
        1,
        "duplicate delivery must not admit twice (errors: {:?})",
        ingress.observer.errors.lock().expect("errors lock")
    );
    assert_eq!(ingress.duplicate_count(), 1, "the replay settles Duplicate");

    // "Restart": a FRESH sink over a fresh workflow instance sharing the
    // same durable idempotency ledger — the replayed event still converges.
    let restarted_thread = group
        .thread("conv-acme-ingress-dedupe-restart")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("restart thread builds");
    let restarted = AcmeIngress::register(parts, &restarted_thread);
    let (status, _) = restarted.post_signed(&body).await;
    assert_eq!(status, StatusCode::OK);
    restarted.drain().await;
    assert_eq!(
        restarted.accepted_count(),
        0,
        "restart replay must not re-admit the settled event"
    );
    assert_eq!(
        restarted.duplicate_count(),
        1,
        "restart replay settles Duplicate from the durable ledger"
    );
}
