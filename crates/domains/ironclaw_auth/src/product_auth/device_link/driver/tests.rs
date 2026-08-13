//! Driver tests: the whole point is the *sequence*, so every case asserts
//! what the vendor was asked, not only what the record ended up saying.

use std::sync::Arc;

use ironclaw_extension_contracts::recipe::VendorAuthRecipe;
use ironclaw_host_api::{ids::InvocationId, resource::ResourceScope};
use secrecy::SecretString;

use super::*;
use crate::{
    AuthSurface, DeviceLinkDriverCall, InMemoryAuthProductServices, RecordingDeviceLinkDriver,
    ResolvedVendorAuthRecipe, StaticAuthRecipeResolver,
};
use ironclaw_host_api::ids::UserId;

const VENDOR: &str = "acmevendor";
const EXTENSION: &str = "acme-personal";

fn device_link_recipe() -> ResolvedVendorAuthRecipe {
    let recipe: VendorAuthRecipe = toml::from_str(
        "method = \"device_link\"\n\
         display_name = \"Acme personal account\"\n",
    )
    .expect("device-link recipe parses");
    ResolvedVendorAuthRecipe {
        vendor: VENDOR.to_string(),
        recipe,
        token_exchange_resource: None,
        protected_resource_metadata_url: None,
    }
}

struct Fixture {
    driver: DeviceLinkFlowDriver,
    vendor: Arc<RecordingDeviceLinkDriver>,
    services: Arc<InMemoryAuthProductServices>,
    scope: AuthProductScope,
}

impl Fixture {
    fn new() -> Self {
        Self::with_recipe(device_link_recipe())
    }

    fn with_recipe(recipe: ResolvedVendorAuthRecipe) -> Self {
        let services = Arc::new(InMemoryAuthProductServices::new());
        let vendor = Arc::new(RecordingDeviceLinkDriver::new());
        let flows: Arc<dyn AuthFlowManager> = services.clone();
        Self {
            driver: DeviceLinkFlowDriver::new(
                vendor.clone(),
                flows,
                Arc::new(StaticAuthRecipeResolver::new(vec![recipe])),
            ),
            vendor,
            services,
            scope: AuthProductScope::new(
                ResourceScope::local_default(
                    UserId::new("user-alpha").expect("user id"),
                    InvocationId::new(),
                )
                .expect("resource scope"),
                AuthSurface::Web,
            ),
        }
    }

    fn start_request(&self) -> DeviceLinkStartRequest {
        DeviceLinkStartRequest {
            scope: self.scope.clone(),
            provider: AuthProviderId::new(VENDOR).expect("provider"),
            extension_id: ExtensionId::new(EXTENSION).expect("extension id"),
            continuation: AuthContinuationRef::SetupOnly,
            mode: DeviceLinkMode::Default,
            resume: None,
        }
    }

    async fn start(&self) -> AuthFlowRecord {
        self.driver
            .start(self.start_request())
            .await
            .expect("start must succeed")
    }
}

fn frame(flow: &AuthFlowRecord) -> &DeviceLinkStep {
    flow.device_link_step()
        .expect("flow carries a device-link frame")
}

/// PR 2's acceptance walk: display → input → complete, driven entirely
/// through the port, with the durable record reflecting each frame.
#[tokio::test]
async fn the_driver_walks_a_fake_adapter_from_display_to_completion() {
    let fixture = Fixture::new();

    let displayed = fixture.start().await;
    assert_eq!(displayed.status, AuthFlowStatus::AwaitingVendor);
    assert!(matches!(frame(&displayed), DeviceLinkStep::Display { .. }));
    assert_eq!(
        displayed.step_revision(),
        1,
        "the first vendor step advances the revision off zero"
    );

    let waiting = fixture
        .driver
        .poll(&fixture.scope, displayed.id)
        .await
        .expect("poll");
    assert_eq!(waiting.status, AuthFlowStatus::AwaitingVendor);
    assert!(matches!(
        frame(&waiting),
        DeviceLinkStep::AwaitingVendor { .. }
    ));

    let asked = fixture
        .driver
        .poll(&fixture.scope, displayed.id)
        .await
        .expect("poll");
    assert_eq!(
        asked.status,
        AuthFlowStatus::AwaitingUser,
        "an input frame waits on the human, not the vendor"
    );
    assert!(matches!(
        frame(&asked),
        DeviceLinkStep::InputRequired { .. }
    ));

    let completed = fixture
        .driver
        .submit_input(
            &fixture.scope,
            displayed.id,
            asked.step_revision(),
            DeviceLinkInput::Password(SecretString::from("hunter2")),
        )
        .await
        .expect("submit");
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert_eq!(
        completed.credential_account_id,
        Some(fixture.vendor.account_id()),
        "auth reports completion against the account the driver minted, never a bare step"
    );

    let DeviceLinkStep::Completed {
        vendor_user_ref, ..
    } = frame(&completed)
    else {
        panic!("expected a completed frame");
    };
    assert!(
        !vendor_user_ref.is_empty(),
        "the resolved identity is the only control that makes a substituted login visible"
    );
}

/// The idempotency contract: a duplicated poll that loses the compare-and-swap
/// gets `Ok` with the already-advanced record, and the adapter is **not**
/// invoked a second time for that transition.
#[tokio::test]
async fn a_duplicated_step_advance_does_not_reinvoke_the_adapter() {
    let fixture = Fixture::new();
    let flow = fixture.start().await;
    let stale_revision = flow.step_revision();

    let advanced = fixture
        .driver
        .poll(&fixture.scope, flow.id)
        .await
        .expect("first poll");
    assert!(advanced.step_revision() > stale_revision);

    // The loser replays the SAME transition it already computed.
    let outcome = fixture
        .services
        .advance_flow_step(
            &fixture.scope,
            AuthFlowStepAdvanceInput {
                flow_id: flow.id,
                expected_revision: stale_revision,
                challenge: advanced.challenge.clone().expect("challenge"),
                status: AuthFlowStatus::AwaitingVendor,
                step_kind: DeviceLinkStepKind::AwaitingVendor,
                step_expires_at: Utc::now(),
                flow_expires_at: None,
                polled_at: Some(Utc::now()),
                error: None,
                credential_account_id: None,
            },
        )
        .await
        .expect("a lost compare-and-swap is Ok, not an error");

    assert!(
        !outcome.applied,
        "the loser must be told its write did not apply"
    );
    assert_eq!(
        outcome.record.step_revision(),
        advanced.step_revision(),
        "the loser is handed the winner's record so it can render instead of retrying"
    );

    let calls = fixture.vendor.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, DeviceLinkDriverCall::Poll(_)))
            .count(),
        1,
        "losing the CAS must never make a second non-idempotent vendor call: {calls:?}"
    );
}

/// The step clock re-mints; it does not terminalize. A lapsed frame on a
/// still-live flow produces a fresh `begin`, and the flow keeps its identity.
#[tokio::test]
async fn a_lapsed_step_clock_remints_the_frame_without_ending_the_flow() {
    let fixture = Fixture::new();
    let flow = fixture.start().await;

    // Age the frame past its step clock while leaving the flow clock alone.
    let stale = fixture
        .services
        .advance_flow_step(
            &fixture.scope,
            AuthFlowStepAdvanceInput {
                flow_id: flow.id,
                expected_revision: flow.step_revision(),
                challenge: expired_display_challenge(&flow),
                status: AuthFlowStatus::AwaitingVendor,
                step_kind: DeviceLinkStepKind::Display,
                step_expires_at: Utc::now() - ChronoDuration::seconds(5),
                flow_expires_at: None,
                polled_at: None,
                error: None,
                credential_account_id: None,
            },
        )
        .await
        .expect("seed a lapsed frame")
        .record;
    assert!(!is_terminal_status(stale.status));

    let reminted = fixture
        .driver
        .poll(&fixture.scope, stale.id)
        .await
        .expect("poll re-mints");

    assert_eq!(reminted.id, stale.id, "a re-mint keeps the same attempt");
    assert!(!is_terminal_status(reminted.status));
    assert!(matches!(frame(&reminted), DeviceLinkStep::Display { .. }));
    let begins = fixture
        .vendor
        .calls()
        .into_iter()
        .filter(|call| matches!(call, DeviceLinkDriverCall::Begin(_)))
        .count();
    assert_eq!(begins, 2, "the lapsed frame is re-minted through `begin`");
}

/// The flow clock terminalizes — and tears down vendor-side first, because an
/// abandoned link that was already accepted leaves a live authorization.
#[tokio::test]
async fn a_lapsed_flow_clock_terminalizes_and_tears_down_vendor_side() {
    let fixture = Fixture::new();
    let flow = fixture.start().await;

    // Reach past the driver to age the FLOW clock; nothing in the public
    // surface can move a deadline backwards, which is itself the invariant.
    fixture
        .services
        .expire_flow_for_tests(flow.id, Utc::now() - ChronoDuration::seconds(1));

    let terminal = fixture
        .driver
        .poll(&fixture.scope, flow.id)
        .await
        .expect("poll terminalizes");

    assert_eq!(terminal.status, AuthFlowStatus::Expired);
    assert!(is_terminal_status(terminal.status));
    assert!(
        fixture.vendor.calls().iter().any(|call| matches!(
            call,
            DeviceLinkDriverCall::Cancel(_, DeviceLinkCancelReason::FlowExpired)
        )),
        "an expiring flow must be torn down vendor-side, not merely forgotten"
    );
}

/// The capped extension: a step advance pushes the flow clock out, but never
/// past the creation-anchored ceiling.
#[test]
fn the_flow_clock_extension_is_capped_at_the_creation_anchored_ceiling() {
    let mut flow = seed_record();
    let now = Utc::now();
    flow.created_at = now - ChronoDuration::seconds(DEVICE_LINK_FLOW_MAX_TTL_SECONDS - 30);
    flow.expires_at = now + ChronoDuration::seconds(5);

    let extended = extended_flow_deadline(&flow, now).expect("a live flow extends");
    assert!(
        extended <= flow.created_at + ChronoDuration::seconds(DEVICE_LINK_FLOW_MAX_TTL_SECONDS),
        "an extension must never push past the cap"
    );
    assert!(extended > flow.expires_at, "and it must be an extension");

    // Already at the ceiling: no further extension at all.
    flow.created_at = now - ChronoDuration::seconds(DEVICE_LINK_FLOW_MAX_TTL_SECONDS);
    flow.expires_at = now + ChronoDuration::seconds(5);
    assert_eq!(
        extended_flow_deadline(&flow, now),
        None,
        "a flow at its ceiling stops renewing itself"
    );
}

// The `flow ceiling ≥ flow ≥ step` ordering is asserted at compile time in
// `driver.rs`; there is deliberately no runtime test for it.

/// A card rendered from a stale revision must not push a credential into a
/// frame that has already moved on.
#[tokio::test]
async fn a_stale_revision_submit_is_refused() {
    let fixture = Fixture::new();
    let flow = fixture.start().await;
    let stale_revision = flow.step_revision();
    fixture
        .driver
        .poll(&fixture.scope, flow.id)
        .await
        .expect("advance past the card's revision");

    let error = fixture
        .driver
        .submit_input(
            &fixture.scope,
            flow.id,
            stale_revision,
            DeviceLinkInput::Password(SecretString::from("hunter2")),
        )
        .await
        .expect_err("a stale submit must be refused");
    assert_eq!(error, AuthProductError::BackendConflict);
    assert!(
        !fixture
            .vendor
            .calls()
            .iter()
            .any(|call| matches!(call, DeviceLinkDriverCall::Submit(..))),
        "a stale submit must never reach the vendor"
    );
}

/// A second "connect" click while a payload is live reuses the flow instead of
/// superseding it — burning a code the user is mid-scan on is the defect.
#[tokio::test]
async fn resuming_a_live_flow_does_not_remint_its_payload() {
    let fixture = Fixture::new();
    let flow = fixture.start().await;

    let mut request = fixture.start_request();
    request.resume = Some(flow.id);
    let resumed = fixture.driver.start(request).await.expect("resume");

    assert_eq!(resumed.id, flow.id);
    assert_eq!(resumed.step_revision(), flow.step_revision());
    assert_eq!(
        fixture
            .vendor
            .calls()
            .into_iter()
            .filter(|call| matches!(call, DeviceLinkDriverCall::Begin(_)))
            .count(),
        1,
        "a live frame must survive a re-render"
    );
}

/// A vendor failure terminalizes with the mapped code rather than leaving the
/// flow live forever.
#[tokio::test]
async fn a_vendor_failure_terminalizes_with_the_mapped_code() {
    let fixture = Fixture::new();
    let flow = fixture.start().await;
    fixture
        .vendor
        .fail_next_call(DeviceLinkErrorCode::AccountUnavailable, false);

    let failed = fixture
        .driver
        .poll(&fixture.scope, flow.id)
        .await
        .expect("a mapped vendor failure is a terminal record, not a call error");
    assert_eq!(failed.status, AuthFlowStatus::Failed);
    assert_eq!(failed.error, Some(AuthErrorCode::CredentialMissing));
    let DeviceLinkStep::Failed { restartable, .. } = frame(&failed) else {
        panic!("expected a failed frame");
    };
    assert!(
        !restartable,
        "an account that can never link must not offer a retry"
    );
}

/// A vendor whose manifest declares some other auth method must not be driven
/// through this path at all.
#[tokio::test]
async fn a_non_device_link_recipe_is_refused_before_any_vendor_call() {
    let recipe: VendorAuthRecipe = serde_json::from_value(serde_json::json!({
        "method": "api_key",
        "display_name": "Acme key",
        "fields": [{ "handle": "acme_api_key", "label": "API key" }],
    }))
    .expect("api_key recipe parses");
    let fixture = Fixture::with_recipe(ResolvedVendorAuthRecipe {
        vendor: VENDOR.to_string(),
        recipe,
        token_exchange_resource: None,
        protected_resource_metadata_url: None,
    });

    let error = fixture
        .driver
        .start(fixture.start_request())
        .await
        .expect_err("a non-device-link recipe must be refused");
    assert_eq!(error, AuthProductError::MalformedConfig);
    assert!(
        fixture.vendor.calls().is_empty(),
        "nothing may reach the vendor when the recipe does not declare this method"
    );
}

fn expired_display_challenge(flow: &AuthFlowRecord) -> AuthChallenge {
    let AuthChallenge::DeviceLinkStep {
        extension_id,
        display_name,
        default_mode_label,
        alternate_mode_label,
        mode,
        step,
        revision,
        ..
    } = flow.challenge.clone().expect("challenge")
    else {
        panic!("expected a device-link challenge");
    };
    AuthChallenge::DeviceLinkStep {
        extension_id,
        display_name,
        default_mode_label,
        alternate_mode_label,
        mode,
        step,
        revision: revision + 1,
        expires_at: Utc::now() - ChronoDuration::seconds(5),
    }
}

fn seed_record() -> AuthFlowRecord {
    AuthFlowRecord {
        id: AuthFlowId::new(),
        scope: AuthProductScope::new(
            ResourceScope::local_default(
                UserId::new("user-alpha").expect("user id"),
                InvocationId::new(),
            )
            .expect("resource scope"),
            AuthSurface::Web,
        ),
        kind: AuthFlowKind::IntegrationCredential,
        status: AuthFlowStatus::AwaitingVendor,
        provider: AuthProviderId::new(VENDOR).expect("provider"),
        requester_extension: Some(ExtensionId::new(EXTENSION).expect("extension id")),
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
        expires_at: Utc::now(),
    }
}
