//! Driver tests.
//!
//! Every case drives a **real** `ExtensionHost` activation — manifest → bind →
//! `check_binding` → published snapshot — so the resolution leg is the
//! production one rather than a hand-built snapshot. What is faked is the
//! vendor (a scripted [`FakeDeviceLinkAdapter`] that answers instantly and
//! limits nothing) and the credential domain (a recording material seam), which
//! is exactly the pair the host is supposed to be defending against.

use std::sync::atomic::AtomicUsize;

use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::device_link::{
    DeviceLinkDisplayKind, DeviceLinkPayload, MAX_DEVICE_LINK_IDENTIFIER_BYTES,
};
use ironclaw_extension_contracts::linked_session::LinkedSessionVersion;
use ironclaw_extension_contracts::state::InstallationState;
use ironclaw_extension_contracts::tool_adapter::ToolAdapter;
use secrecy::SecretString;

use super::*;
use crate::entrypoint::ExtensionBindings;
use crate::lifecycle::{ExtensionHost, ExtensionHostDeps};
use crate::linked_session_custody::{
    LinkedSessionKey, LinkedSessionMaterialStore, StoredLinkedSession,
};
use crate::store::{
    InstallationRecord, InstallationRecordStore, RehydratedInstallationRecordStore,
};
use crate::test_support::{
    FakeChannelAdapter, FakeDeviceLinkAdapter, FakeDeviceLinkCallKind, FakeEgressFactory,
    FakeLoader, RecordingDrain, channel_only_manifest, device_link_channel_manifest,
};

const EXTENSION: &str = "acme-link";
const USER: &str = "user-1";
const FLOW: &str = "flow-1";

/// A material seam that answers "this account exists, nothing stored yet" for
/// every key and records what it was asked about. Named away from
/// `InMemory*Store` on purpose (the extensions family bans that name in `src/`).
#[derive(Default)]
struct RecordingMaterial {
    keys: Mutex<Vec<LinkedSessionKey>>,
}

impl RecordingMaterial {
    fn keys(&self) -> Vec<LinkedSessionKey> {
        self.keys.lock().expect("recorded keys").clone()
    }
}

#[async_trait::async_trait]
impl LinkedSessionMaterialStore for RecordingMaterial {
    async fn load(
        &self,
        key: &LinkedSessionKey,
    ) -> Result<
        Option<StoredLinkedSession>,
        ironclaw_extension_contracts::linked_session::LinkedSessionError,
    > {
        self.keys.lock().expect("recorded keys").push(key.clone());
        Ok(Some(StoredLinkedSession {
            link_revision: PENDING_LINK_REVISION,
            snapshot: None,
        }))
    }

    async fn replace(
        &self,
        _key: &LinkedSessionKey,
        _expected: LinkedSessionVersion,
        _blob: ironclaw_extension_contracts::linked_session::SessionBytes,
    ) -> Result<
        LinkedSessionVersion,
        ironclaw_extension_contracts::linked_session::LinkedSessionError,
    > {
        LinkedSessionVersion::new("v1").map_err(|_| {
            ironclaw_extension_contracts::linked_session::LinkedSessionError::Unavailable {
                reason: "test version token",
            }
        })
    }
}

struct Harness {
    _host: ExtensionHost,
    driver: SnapshotDeviceLinkDriver,
    adapter: Arc<FakeDeviceLinkAdapter>,
    material: Arc<RecordingMaterial>,
}

async fn harness(limits: DeviceLinkLimits) -> Harness {
    harness_with(limits, FakeDeviceLinkAdapter::default(), true).await
}

async fn harness_with(
    limits: DeviceLinkLimits,
    adapter: FakeDeviceLinkAdapter,
    declares_device_link: bool,
) -> Harness {
    let adapter = Arc::new(adapter);
    let resolved = if declares_device_link {
        device_link_channel_manifest()
    } else {
        channel_only_manifest()
    };
    let bindings = ExtensionBindings {
        tools: declares_device_link
            .then(|| Arc::new(crate::test_support::FakeToolAdapter) as Arc<dyn ToolAdapter>),
        channel: Some(Arc::new(FakeChannelAdapter::default()) as Arc<dyn ChannelAdapter>),
        device_link: declares_device_link
            .then(|| Arc::clone(&adapter) as Arc<dyn DeviceLinkAdapter>),
    };
    let store = Arc::new(RehydratedInstallationRecordStore::default());
    let extension_id = resolved.id.as_str().to_string();
    let host = ExtensionHost::new(ExtensionHostDeps {
        store: Arc::clone(&store) as Arc<dyn InstallationRecordStore>,
        loader: Arc::new(FakeLoader {
            bindings,
            load_calls: Arc::new(AtomicUsize::new(0)),
            fail_load: false,
        }),
        drain: Arc::new(RecordingDrain::default()),
        egress: Arc::new(FakeEgressFactory),
        reserved_capability_ids: Default::default(),
        reserved_ingress_routes: Default::default(),
        hook_deadline: Duration::from_secs(5),
        linked_sessions: LinkedSessionStore::unavailable(),
    })
    .await;
    host.install(InstallationRecord {
        extension_id: extension_id.clone(),
        installation_id: format!("{extension_id}-install"),
        state: InstallationState::Installed,
        resolved: Arc::new(resolved),
        config: Vec::new(),
        last_error: None,
    })
    .await
    .expect("install");
    host.activate(&extension_id).await.expect("activate");

    let material = Arc::new(RecordingMaterial::default());
    let sessions =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let driver = SnapshotDeviceLinkDriver::new(host.snapshot_watch(), sessions, limits)
        .expect("valid limits");
    Harness {
        _host: host,
        driver,
        adapter,
        material,
    }
}

fn request(flow: &str) -> DeviceLinkRequest {
    DeviceLinkRequest {
        flow_id: DeviceLinkFlowId::new(flow).expect("flow id"),
        extension_id: ExtensionId::new(EXTENSION).expect("extension id"),
        user_id: UserId::new(USER).expect("user id"),
        account: None,
    }
}

fn display(expires_in: Duration) -> DeviceLinkStep {
    DeviceLinkStep::Display {
        kind: DeviceLinkDisplayKind::QrCode,
        payload: DeviceLinkPayload::new("tg://login?token=abc").expect("payload"),
        expires_in,
    }
}

// -------------------------------------------------------------------------
// Resolution and scoping
// -------------------------------------------------------------------------

#[tokio::test]
async fn begin_hands_the_adapter_a_context_scoped_to_this_flow() {
    let h = harness(DeviceLinkLimits::default()).await;
    let request = request(FLOW);

    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("begin");

    let calls = h.adapter.recorded();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(
        call.kind,
        FakeDeviceLinkCallKind::Begin(DeviceLinkMode::Default)
    );
    assert_eq!(call.flow_id, request.flow_id);
    assert_eq!(call.extension_id, request.extension_id);
    assert_eq!(call.user_id, request.user_id);
    assert!(
        call.account.is_none(),
        "no credential account exists until custody is durable"
    );
    assert!(
        call.session_loaded,
        "the adapter receives a usable custody handle"
    );

    let keys = h.material.keys();
    assert_eq!(keys.len(), 1, "one handle, one key");
    assert_eq!(keys[0].extension().as_str(), EXTENSION);
    assert_eq!(
        keys[0].account().as_str(),
        format!("{PENDING_ACCOUNT_PREFIX}{FLOW}"),
        "the pre-mint handle is scoped to this flow's provisional account"
    );
}

#[tokio::test]
async fn an_extension_that_binds_no_adapter_has_no_driver() {
    let h = harness_with(
        DeviceLinkLimits::default(),
        FakeDeviceLinkAdapter::default(),
        false,
    )
    .await;
    let request = DeviceLinkRequest {
        flow_id: DeviceLinkFlowId::new(FLOW).expect("flow id"),
        extension_id: ExtensionId::new("acme-chat").expect("extension id"),
        user_id: UserId::new(USER).expect("user id"),
        account: None,
    };

    let error = h
        .driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect_err("nothing is bound");

    assert!(matches!(error, DeviceLinkError::Internal { .. }));
    assert_eq!(h.adapter.call_count(), 0);
}

#[tokio::test]
async fn an_undeclared_alternate_mode_never_reaches_the_adapter() {
    let h = harness(DeviceLinkLimits::default()).await;

    let error = h
        .driver
        .begin(&request(FLOW), DeviceLinkMode::Alternate)
        .await
        .expect_err("the fixture recipe declares no alternate path");

    assert_eq!(
        error,
        DeviceLinkError::UnsupportedMode {
            mode: DeviceLinkMode::Alternate
        }
    );
    assert_eq!(h.adapter.call_count(), 0);
}

#[tokio::test]
async fn a_second_begin_does_not_re_invoke_a_transition_that_already_ran() {
    let h = harness(DeviceLinkLimits::default()).await;
    let request = request(FLOW);
    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("first begin");

    let error = h
        .driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect_err("a live flow must not start a parallel vendor conversation");

    assert!(matches!(error, DeviceLinkError::Internal { .. }));
    assert_eq!(h.adapter.call_count(), 1);
}

// -------------------------------------------------------------------------
// Poll floor and TTLs
// -------------------------------------------------------------------------

#[tokio::test]
async fn a_poll_inside_the_floor_answers_without_calling_the_adapter() {
    let limits = DeviceLinkLimits::default();
    let h = harness(limits).await;
    let request = request(FLOW);
    let start = Instant::now();
    h.driver
        .begin_at(&request, DeviceLinkMode::Default, start)
        .await
        .expect("begin");
    h.driver
        .poll_at(&request, start)
        .await
        .expect("first poll is admitted");
    let after_first = h.adapter.call_count();

    let step = h
        .driver
        .poll_at(&request, start + Duration::from_millis(500))
        .await
        .expect("second poll");

    match step {
        DeviceLinkStep::AwaitingVendor { retry_in } => {
            assert!(retry_in <= limits.min_poll_interval && !retry_in.is_zero());
        }
        other => panic!("expected AwaitingVendor, got {other:?}"),
    }
    assert_eq!(
        h.adapter.call_count(),
        after_first,
        "a hot-looping card must not become vendor traffic"
    );
}

#[tokio::test]
async fn a_poll_past_the_floor_reaches_the_adapter() {
    let limits = DeviceLinkLimits::default();
    let h = harness(limits).await;
    let request = request(FLOW);
    let start = Instant::now();
    h.driver
        .begin_at(&request, DeviceLinkMode::Default, start)
        .await
        .expect("begin");
    h.driver.poll_at(&request, start).await.expect("first poll");
    let after_first = h.adapter.call_count();

    h.driver
        .poll_at(&request, start + limits.min_poll_interval)
        .await
        .expect("second poll");

    assert_eq!(h.adapter.call_count(), after_first + 1);
}

#[tokio::test]
async fn an_expired_flow_is_cancelled_before_it_is_reported() {
    let limits = DeviceLinkLimits::default();
    let h = harness(limits).await;
    let request = request(FLOW);
    let start = Instant::now();
    h.driver
        .begin_at(&request, DeviceLinkMode::Default, start)
        .await
        .expect("begin");

    let step = h
        .driver
        .poll_at(&request, start + limits.flow_ttl)
        .await
        .expect("expiry is a step, not an error");

    assert_eq!(
        step,
        DeviceLinkStep::Failed {
            code: DeviceLinkErrorCode::Expired,
            restartable: true
        }
    );
    assert!(
        h.adapter
            .recorded()
            .iter()
            .any(|call| call.kind == FakeDeviceLinkCallKind::Cancel),
        "a vendor authorization must not outlive the flow that obtained it"
    );
    // The flow is gone, so the next call cannot resurrect it.
    assert_eq!(
        h.driver
            .poll_at(&request, start + limits.flow_ttl)
            .await
            .expect_err("reaped"),
        DriverFailure::Link(DeviceLinkError::UnknownFlow)
    );
}

#[tokio::test]
async fn an_adapter_cannot_extend_the_step_clock() {
    let limits = DeviceLinkLimits::default();
    let h = harness_with(
        limits,
        FakeDeviceLinkAdapter::scripted([display(Duration::from_secs(60 * 60))]),
        true,
    )
    .await;

    let step = h
        .driver
        .begin(&request(FLOW), DeviceLinkMode::Default)
        .await
        .expect("begin");

    match step {
        DeviceLinkStep::Display { expires_in, .. } => {
            assert_eq!(expires_in, limits.step_ttl, "clamped to the host's clock");
        }
        other => panic!("expected Display, got {other:?}"),
    }
}

#[tokio::test]
async fn an_adapter_cannot_ask_to_be_polled_faster_than_the_floor() {
    let limits = DeviceLinkLimits::default();
    let h = harness_with(
        limits,
        FakeDeviceLinkAdapter::scripted([DeviceLinkStep::AwaitingVendor {
            retry_in: Duration::from_millis(1),
        }]),
        true,
    )
    .await;

    let step = h
        .driver
        .begin(&request(FLOW), DeviceLinkMode::Default)
        .await
        .expect("begin");

    assert_eq!(
        step,
        DeviceLinkStep::AwaitingVendor {
            retry_in: limits.min_poll_interval
        }
    );
}

#[tokio::test]
async fn a_terminal_step_ends_the_flow() {
    let h = harness_with(
        DeviceLinkLimits::default(),
        FakeDeviceLinkAdapter::scripted([DeviceLinkStep::Failed {
            code: DeviceLinkErrorCode::Declined,
            restartable: false,
        }]),
        true,
    )
    .await;
    let request = request(FLOW);

    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("begin");

    assert_eq!(
        h.driver.poll(&request).await.expect_err("flow is over"),
        DeviceLinkError::UnknownFlow
    );
}

// -------------------------------------------------------------------------
// Budgets
// -------------------------------------------------------------------------

#[tokio::test]
async fn begins_are_bounded_per_user() {
    let limits = DeviceLinkLimits {
        max_begins_per_user: 1,
        ..DeviceLinkLimits::default()
    };
    let h = harness(limits).await;
    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("first begin");

    let error = h
        .driver
        .begin(&request("flow-b"), DeviceLinkMode::Default)
        .await
        .expect_err("the per-user budget is spent");

    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::RateLimited,
            restartable: true
        }
    );
    assert_eq!(h.adapter.call_count(), 1);
}

#[tokio::test]
async fn begins_are_bounded_per_deployment() {
    let limits = DeviceLinkLimits {
        max_begins_per_deployment: 1,
        ..DeviceLinkLimits::default()
    };
    let h = harness(limits).await;
    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("first begin");

    let mut second = request("flow-b");
    second.user_id = UserId::new("user-2").expect("second user");
    let error = h
        .driver
        .begin(&second, DeviceLinkMode::Default)
        .await
        .expect_err("the deployment budget is spent");

    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::RateLimited,
            restartable: true
        }
    );
}

#[tokio::test]
async fn distinct_identifiers_are_bounded_per_user() {
    let limits = DeviceLinkLimits {
        max_identifiers_per_user: 1,
        ..DeviceLinkLimits::default()
    };
    let h = harness(limits).await;
    let request = request(FLOW);
    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("begin");
    h.driver
        .submit_input(
            &request,
            DeviceLinkInput::Identifier("+15550000001".to_string()),
        )
        .await
        .expect("first identifier");
    let after_first = h.adapter.call_count();

    // The same value again is not a new target.
    h.driver
        .submit_input(
            &request,
            DeviceLinkInput::Identifier("+15550000001".to_string()),
        )
        .await
        .expect("a retry against the same number is not a new target");
    assert_eq!(h.adapter.call_count(), after_first + 1);

    let error = h
        .driver
        .submit_input(
            &request,
            DeviceLinkInput::Identifier("+15550000002".to_string()),
        )
        .await
        .expect_err("spraying login codes at fresh numbers is the vector this bounds");

    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::RateLimited,
            restartable: true
        }
    );
    assert_eq!(h.adapter.call_count(), after_first + 1);
}

#[tokio::test]
async fn secret_attempts_are_bounded_per_flow() {
    let limits = DeviceLinkLimits {
        max_secret_attempts_per_flow: 1,
        ..DeviceLinkLimits::default()
    };
    let h = harness(limits).await;
    let request = request(FLOW);
    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("begin");
    h.driver
        .submit_input(&request, DeviceLinkInput::Password(SecretString::from("a")))
        .await
        .expect("first attempt");
    let after_first = h.adapter.call_count();

    let error = h
        .driver
        .submit_input(&request, DeviceLinkInput::Password(SecretString::from("b")))
        .await
        .expect_err("an unbounded password retry is an account-lockout vector");

    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::RateLimited,
            restartable: true
        }
    );
    assert_eq!(h.adapter.call_count(), after_first);
}

#[tokio::test]
async fn an_oversized_identifier_never_reaches_the_adapter() {
    let h = harness(DeviceLinkLimits::default()).await;
    let request = request(FLOW);
    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("begin");
    let before = h.adapter.call_count();

    let error = h
        .driver
        .submit_input(
            &request,
            DeviceLinkInput::Identifier("9".repeat(MAX_DEVICE_LINK_IDENTIFIER_BYTES + 1)),
        )
        .await
        .expect_err("an unbounded paste is rejected at the boundary");

    assert!(matches!(error, DeviceLinkError::InvalidInput { .. }));
    assert_eq!(h.adapter.call_count(), before);
}

#[tokio::test]
async fn active_flows_are_bounded() {
    let limits = DeviceLinkLimits {
        max_active_flows: 1,
        ..DeviceLinkLimits::default()
    };
    let h = harness(limits).await;
    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("first begin");

    let error = h
        .driver
        .begin(&request("flow-b"), DeviceLinkMode::Default)
        .await
        .expect_err("the driver is bounded");

    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::RateLimited,
            restartable: true
        }
    );
}

// -------------------------------------------------------------------------
// Limits validation
// -------------------------------------------------------------------------

#[test]
fn the_three_clocks_must_agree() {
    let inverted = DeviceLinkLimits {
        step_ttl: Duration::from_secs(600),
        flow_ttl: Duration::from_secs(60),
        ..DeviceLinkLimits::default()
    };
    assert_eq!(
        inverted.validate(),
        Err(DeviceLinkLimitsError::StepTtlAboveFlowTtl)
    );

    let fast_poll = DeviceLinkLimits {
        min_poll_interval: Duration::from_secs(120),
        step_ttl: Duration::from_secs(60),
        ..DeviceLinkLimits::default()
    };
    assert_eq!(
        fast_poll.validate(),
        Err(DeviceLinkLimitsError::PollFloorAboveStepTtl)
    );

    let zeroed = DeviceLinkLimits {
        max_begins_per_user: 0,
        ..DeviceLinkLimits::default()
    };
    assert_eq!(zeroed.validate(), Err(DeviceLinkLimitsError::ZeroBound));

    DeviceLinkLimits::default()
        .validate()
        .expect("the shipped defaults satisfy their own ordering");
}

// -------------------------------------------------------------------------
// The `ironclaw_auth::DeviceLinkDriver` port
// -------------------------------------------------------------------------

mod auth_port {
    use ironclaw_auth::{
        AuthFlowId, AuthProviderId, DeviceLinkBeginRequest, DeviceLinkBinding, DeviceLinkDriver,
        DeviceLinkDriverError,
    };

    use super::*;

    fn binding(extension: &str) -> DeviceLinkBinding {
        DeviceLinkBinding {
            provider: AuthProviderId::new("acme-link").expect("provider id"),
            extension_id: ExtensionId::new(extension).expect("extension id"),
            user_id: UserId::new(USER).expect("user id"),
        }
    }

    #[tokio::test]
    async fn the_port_walks_the_adapter_and_reports_its_step() {
        let h = harness(DeviceLinkLimits::default()).await;

        let outcome = DeviceLinkDriver::begin(
            &h.driver,
            DeviceLinkBeginRequest {
                flow_id: AuthFlowId::new(),
                binding: binding(EXTENSION),
                mode: DeviceLinkMode::Default,
            },
        )
        .await
        .expect("begin through the port");

        assert!(matches!(
            outcome.step,
            DeviceLinkStep::AwaitingVendor { .. }
        ));
        assert_eq!(h.adapter.call_count(), 1);
    }

    /// "What does it return when no binding exists" — the question PLAN PR 2
    /// called out. Not restartable: installing an extension is not a retry.
    #[tokio::test]
    async fn an_unbound_provider_answers_no_binding_by_name() {
        let h = harness_with(
            DeviceLinkLimits::default(),
            FakeDeviceLinkAdapter::default(),
            false,
        )
        .await;

        let error = DeviceLinkDriver::begin(
            &h.driver,
            DeviceLinkBeginRequest {
                flow_id: AuthFlowId::new(),
                binding: binding("acme-chat"),
                mode: DeviceLinkMode::Default,
            },
        )
        .await
        .expect_err("nothing is bound");

        assert!(
            matches!(&error, DeviceLinkDriverError::NoBinding { provider } if provider.as_str() == "acme-link"),
            "{error:?}"
        );
        assert!(!error.restartable());
    }
}
