//! Driver tests.
//!
//! Every case drives a **real** `ExtensionHost` activation — manifest → bind →
//! `check_binding` → published snapshot — so the resolution leg is the
//! production one rather than a hand-built snapshot. What is faked is the
//! vendor (a scripted [`FakeDeviceLinkAdapter`] that answers instantly and
//! limits nothing) and the credential domain (the in-memory auth fake plus a
//! recording material seam), which is exactly the pair the host is supposed to
//! be defending against.

use std::sync::atomic::AtomicUsize;

use ironclaw_auth::{
    CredentialAccountRecordSource, CredentialAccountService, InMemoryAuthProductServices,
};
use ironclaw_extension_contracts::device_link::{
    DeviceLinkDisplayKind, DeviceLinkPayload, MAX_DEVICE_LINK_IDENTIFIER_BYTES,
};
use ironclaw_extension_contracts::linked_session::{LinkedSessionSnapshot, LinkedSessionVersion};
use ironclaw_extension_contracts::state::InstallationState;
use ironclaw_extension_contracts::tool_adapter::ToolAdapter;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::ids::InvocationId;
use ironclaw_host_api::ids::TenantId;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_host_api::user_identity::{
    RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
    RebornUserIdentityBindingStore, RebornUserIdentityLookup,
};
use secrecy::SecretString;

use super::*;
use crate::entrypoint::ExtensionBindings;
use crate::lifecycle::{ExtensionHost, ExtensionHostDeps};
use crate::linked_session_custody::{LinkedSessionMaterialKey, LinkedSessionMaterialStore};
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

/// A material seam that answers "nothing stored yet" for every key and records
/// what it was asked about. Named away from `InMemory*Store` on purpose (the
/// extensions family bans that name in `src/`). Provisional (revision-0)
/// custody never reaches it — the store parks those in process memory — so a
/// key recorded here is always a durable-custody access.
#[derive(Default)]
struct RecordingMaterial {
    keys: Mutex<Vec<LinkedSessionMaterialKey>>,
}

impl RecordingMaterial {
    fn keys(&self) -> Vec<LinkedSessionMaterialKey> {
        self.keys.lock().expect("recorded keys").clone()
    }
}

#[async_trait::async_trait]
impl LinkedSessionMaterialStore for RecordingMaterial {
    async fn load(
        &self,
        key: &LinkedSessionMaterialKey,
    ) -> Result<
        Option<LinkedSessionSnapshot>,
        ironclaw_extension_contracts::linked_session::LinkedSessionError,
    > {
        self.keys.lock().expect("recorded keys").push(key.clone());
        Ok(None)
    }

    async fn replace(
        &self,
        key: &LinkedSessionMaterialKey,
        _expected: LinkedSessionVersion,
        _blob: ironclaw_extension_contracts::linked_session::SessionBytes,
    ) -> Result<
        LinkedSessionVersion,
        ironclaw_extension_contracts::linked_session::LinkedSessionError,
    > {
        self.keys.lock().expect("recorded keys").push(key.clone());
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
    accounts: Arc<InMemoryAuthProductServices>,
    identities: Arc<crate::FilesystemChannelIdentityStore>,
}

async fn harness(limits: DeviceLinkLimits) -> Harness {
    harness_with(limits, FakeDeviceLinkAdapter::default(), true).await
}

async fn harness_with(
    limits: DeviceLinkLimits,
    adapter: FakeDeviceLinkAdapter,
    declares_device_link: bool,
) -> Harness {
    harness_with_account_service(limits, adapter, declares_device_link, None).await
}

async fn harness_with_account_service(
    limits: DeviceLinkLimits,
    adapter: FakeDeviceLinkAdapter,
    declares_device_link: bool,
    accounts_override: Option<Arc<dyn ironclaw_auth::CredentialAccountService>>,
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
        channel: FakeChannelAdapter::all_halves(),
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
        linked_accounts: Arc::new(
            crate::linked_account_resolution::UnavailableLinkedAccountResolution,
        ),
        admin_secrets: None,
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
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let account_service = accounts_override.unwrap_or_else(|| {
        Arc::clone(&accounts) as Arc<dyn ironclaw_auth::CredentialAccountService>
    });
    let identities = Arc::new(crate::FilesystemChannelIdentityStore::new(
        Arc::new(InMemoryBackend::new()),
        TenantId::new("tenant-test").expect("tenant"),
        UserId::new("operator").expect("operator"),
    ));
    let driver = SnapshotDeviceLinkDriver::new(
        host.snapshot_watch(),
        sessions,
        limits,
        account_service,
        Arc::clone(&identities),
    )
    .expect("valid limits");
    Harness {
        _host: host,
        driver,
        adapter,
        material,
        accounts,
        identities,
    }
}

struct FailingCompletionAccounts;

fn unsupported_accounts() -> ironclaw_auth::AuthProductError {
    ironclaw_auth::AuthProductError::UnsupportedOperation {
        operation: "test failing completion account service",
    }
}

#[async_trait::async_trait]
impl ironclaw_auth::CredentialAccountService for FailingCompletionAccounts {
    async fn create_account(
        &self,
        _request: ironclaw_auth::NewCredentialAccount,
    ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn get_account(
        &self,
        _request: ironclaw_auth::CredentialAccountLookupRequest,
    ) -> Result<Option<ironclaw_auth::CredentialAccount>, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn list_accounts(
        &self,
        _request: ironclaw_auth::CredentialAccountListRequest,
    ) -> Result<ironclaw_auth::CredentialAccountListPage, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn update_status(
        &self,
        _scope: &ironclaw_auth::AuthProductScope,
        _account_id: ironclaw_auth::CredentialAccountId,
        _status: ironclaw_auth::CredentialAccountStatus,
    ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn select_unique_configured_account(
        &self,
        _request: ironclaw_auth::CredentialAccountSelectionRequest,
    ) -> Result<ironclaw_auth::CredentialAccountProjection, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn project_credential_recovery(
        &self,
        _request: ironclaw_auth::CredentialRecoveryRequest,
    ) -> Result<ironclaw_auth::CredentialRecoveryProjection, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn select_configured_account(
        &self,
        _request: ironclaw_auth::CredentialAccountChoiceRequest,
    ) -> Result<ironclaw_auth::CredentialAccountProjection, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn refresh_account(
        &self,
        _request: ironclaw_auth::CredentialRefreshRequest,
    ) -> Result<ironclaw_auth::CredentialRefreshReport, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }

    async fn complete_linked_device_link(
        &self,
        _request: ironclaw_auth::LinkedDeviceLinkCompletion,
    ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
        Err(unsupported_accounts())
    }
}

fn product_scope(user: &str) -> ironclaw_auth::AuthProductScope {
    ironclaw_auth::AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).expect("user id"), InvocationId::new())
            .expect("resource scope"),
        ironclaw_auth::AuthSurface::Web,
    )
}

fn request(flow: &str) -> DeviceLinkRequest {
    DeviceLinkRequest {
        flow_id: DeviceLinkFlowId::new(flow).expect("flow id"),
        extension_id: ExtensionId::new(EXTENSION).expect("extension id"),
        scope: product_scope(USER),
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

fn completed() -> DeviceLinkStep {
    DeviceLinkStep::Completed {
        account_label: "Linked personal account".to_string(),
        vendor_user_ref: "+15550000000".to_string(),
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
    assert_eq!(&call.user_id, request.user_id());
    assert!(
        call.account.is_none(),
        "no credential account exists until custody is durable"
    );
    assert!(
        call.session_loaded,
        "the adapter receives a usable custody handle"
    );
    assert!(
        h.material.keys().is_empty(),
        "a pre-mint handle parks in the provisional space and never reaches \
         the durable material seam"
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
        scope: product_scope(USER),
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

/// Superseded assertion, deliberately rewritten rather than deleted.
///
/// This test used to pin an outright refusal of a second `begin`, on the
/// reasoning that it would start a parallel vendor conversation. The refusal
/// was wrong — the auth tier re-mints a lapsed frame through exactly that call,
/// so refusing it terminalized every link whose first frame lapsed — but the
/// invariant underneath it was right and still holds: an adapter must never be
/// running two conversations for one flow. That is now enforced by ordering
/// (cancel, then begin) instead of by refusal, and this test pins the ordering.
#[tokio::test]
async fn a_second_begin_never_leaves_two_vendor_conversations_running() {
    let h = harness(DeviceLinkLimits::default()).await;
    let request = request(FLOW);
    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("first begin");

    h.driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect("the re-mint is admitted");

    // Every `Begin` after the first is preceded by a `Cancel`, so at no point
    // are two conversations live for one flow.
    let kinds: Vec<_> = h.adapter.recorded().into_iter().map(|c| c.kind).collect();
    for (index, kind) in kinds.iter().enumerate() {
        if index > 0 && matches!(kind, FakeDeviceLinkCallKind::Begin(_)) {
            assert_eq!(
                kinds[index - 1],
                FakeDeviceLinkCallKind::Cancel,
                "a begin that is not the first must be preceded by a cancel: {kinds:?}"
            );
        }
    }
    assert_eq!(kinds.len(), 3, "begin, cancel, begin: {kinds:?}");
}

#[tokio::test]
async fn a_failed_vendor_transition_is_cancelled_before_the_host_forgets_it() {
    let adapter = FakeDeviceLinkAdapter::default();
    *adapter.fail_with.lock().expect("fake device-link failure") = Some(DeviceLinkError::Vendor {
        code: DeviceLinkErrorCode::AccountUnavailable,
        restartable: false,
    });
    let h = harness_with(DeviceLinkLimits::default(), adapter, true).await;

    h.driver
        .begin(&request(FLOW), DeviceLinkMode::Default)
        .await
        .expect_err("the scripted vendor transition fails");

    assert_eq!(
        h.adapter
            .recorded()
            .into_iter()
            .map(|call| call.kind)
            .collect::<Vec<_>>(),
        vec![
            FakeDeviceLinkCallKind::Begin(DeviceLinkMode::Default),
            FakeDeviceLinkCallKind::Cancel,
        ],
        "a possibly authorized device must be torn down immediately, even when cancel also fails",
    );
}

// -------------------------------------------------------------------------
// Completion mints the credential account
// -------------------------------------------------------------------------

#[tokio::test]
async fn a_completion_mints_a_pinned_account_and_reports_it() {
    let h = harness_with(
        DeviceLinkLimits::default(),
        FakeDeviceLinkAdapter::scripted([completed()]),
        true,
    )
    .await;
    let request = request(FLOW);

    let settled = h
        .driver
        .begin_at(&request, DeviceLinkMode::Default, Instant::now())
        .await
        .expect("completion settles");

    assert!(matches!(settled.step, DeviceLinkStep::Completed { .. }));
    let minted = settled
        .account
        .expect("a completed step carries the minted account");
    assert_eq!(
        h.adapter
            .recorded()
            .into_iter()
            .map(|call| call.kind)
            .collect::<Vec<_>>(),
        vec![
            FakeDeviceLinkCallKind::Begin(DeviceLinkMode::Default),
            FakeDeviceLinkCallKind::Finalize,
        ],
        "the vendor session becomes established only after host custody and identity commit",
    );
    assert_eq!(minted.link_revision, 1, "a first link is revision 1");
    assert_eq!(minted.vendor_user_ref.as_str(), "+15550000000");
    assert_eq!(
        h.identities
            .resolve_user_identity(
                "acme-link",
                &crate::channel_identity::ChannelIdentityKeyspace::DeviceLinkV1.provider_user_id(
                    &ironclaw_host_api::product_adapter::AdapterInstallationId::new(
                        "acme-link-install",
                    )
                    .expect("installation"),
                    "+15550000000",
                ),
            )
            .await
            .expect("resolve linked channel identity"),
        Some(UserId::new(USER).expect("user")),
        "device-link completion must establish the caller's channel identity",
    );

    // The account is real, pinned, and the session blob is durable custody —
    // store → mint → report held.
    let accounts = h
        .accounts
        .accounts_for_owner(&request.scope)
        .await
        .expect("list accounts");
    assert_eq!(accounts.len(), 1);
    let account = &accounts[0];
    assert_eq!(account.id, minted.account_id);
    assert!(account.is_linked_device());
    assert!(account.linked_device_ownership_is_pinned());
    let stored = h
        .accounts
        .load_opaque_material(ironclaw_auth::OpaqueMaterialRequest {
            scope: request.scope.clone(),
            account_id: account.id,
            requester_extension: Some(request.extension_id.clone()),
            link_revision: 1,
        })
        .await
        .expect("custody read")
        .expect("the handshake blob is durable before completion is reported");
    assert_eq!(stored.material.expose(), b"fake-linked-session");

    // The flow is over: the provisional blob is discarded and the flow is
    // forgotten, so a stale card cannot re-drive it.
    assert_eq!(
        h.driver
            .poll(&request)
            .await
            .expect_err("a completed flow is forgotten"),
        DeviceLinkError::UnknownFlow
    );
}

#[tokio::test]
async fn a_completion_without_stored_custody_fails_and_mints_nothing() {
    let adapter = FakeDeviceLinkAdapter::scripted([completed()]);
    *adapter
        .skip_completion_persist
        .lock()
        .expect("persist flag") = true;
    let h = harness_with(DeviceLinkLimits::default(), adapter, true).await;
    let request = request(FLOW);

    let error = h
        .driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect_err("a completion the custody store cannot back is refused");

    assert!(
        matches!(error, DeviceLinkError::Custody(_)),
        "expected a custody failure, got {error:?}"
    );
    let accounts = h
        .accounts
        .accounts_for_owner(&request.scope)
        .await
        .expect("list accounts");
    assert!(
        accounts.is_empty(),
        "a refused completion must not leave a minted account behind"
    );
}

#[tokio::test]
async fn credential_completion_failure_rolls_back_the_new_channel_identity() {
    let h = harness_with_account_service(
        DeviceLinkLimits::default(),
        FakeDeviceLinkAdapter::scripted([completed()]),
        true,
        Some(Arc::new(FailingCompletionAccounts)),
    )
    .await;
    let request = request(FLOW);

    let error = h
        .driver
        .begin(&request, DeviceLinkMode::Default)
        .await
        .expect_err("credential completion must fail");
    assert!(matches!(error, DeviceLinkError::Custody(_)));
    assert_eq!(
        h.adapter
            .recorded()
            .into_iter()
            .map(|call| call.kind)
            .collect::<Vec<_>>(),
        vec![
            FakeDeviceLinkCallKind::Begin(DeviceLinkMode::Default),
            FakeDeviceLinkCallKind::Cancel,
        ],
        "a vendor-authorized provisional session must be cancelled when credential completion fails",
    );
    assert_eq!(
        h.identities
            .resolve_user_identity(
                "acme-link",
                &crate::channel_identity::ChannelIdentityKeyspace::DeviceLinkV1.provider_user_id(
                    &ironclaw_host_api::product_adapter::AdapterInstallationId::new(
                        "acme-link-install",
                    )
                    .expect("installation"),
                    "+15550000000",
                ),
            )
            .await
            .expect("resolve after failed completion"),
        None,
        "failed credential custody must compensate the newly created channel identity"
    );
}

#[tokio::test]
async fn identity_conflict_cancels_the_provisional_vendor_session_and_names_the_remedy() {
    let h = harness_with(
        DeviceLinkLimits::default(),
        FakeDeviceLinkAdapter::scripted([completed()]),
        true,
    )
    .await;
    h.identities
        .bind_user_identity(RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("acme-link").expect("provider"),
            provider_user_id: RebornIdentityProviderUserId::new(
                crate::channel_identity::ChannelIdentityKeyspace::DeviceLinkV1.provider_user_id(
                    &ironclaw_host_api::product_adapter::AdapterInstallationId::new(
                        "acme-link-install",
                    )
                    .expect("installation"),
                    "+15550000000",
                ),
            )
            .expect("provider user"),
            user_id: UserId::new("another-user").expect("other user"),
        })
        .await
        .expect("pre-bind identity to another user");

    let error = h
        .driver
        .begin(&request(FLOW), DeviceLinkMode::Default)
        .await
        .expect_err("an identity cannot belong to two users");

    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::IdentityConflict,
            restartable: false,
        }
    );
    assert_eq!(
        h.adapter
            .recorded()
            .into_iter()
            .map(|call| call.kind)
            .collect::<Vec<_>>(),
        vec![
            FakeDeviceLinkCallKind::Begin(DeviceLinkMode::Default),
            FakeDeviceLinkCallKind::Cancel,
        ],
        "identity rejection must revoke the just-authorized provisional device",
    );
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

    let settled = h
        .driver
        .poll_at(&request, start + Duration::from_millis(500))
        .await
        .expect("second poll");

    match settled.step {
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

    let settled = h
        .driver
        .poll_at(&request, start + limits.flow_ttl)
        .await
        .expect("expiry is a step, not an error");

    assert_eq!(
        settled.step,
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
    assert!(matches!(
        h.driver
            .poll_at(&request, start + limits.flow_ttl)
            .await
            .expect_err("reaped"),
        DriverFailure::Link(DeviceLinkError::UnknownFlow)
    ));
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
            code: DeviceLinkErrorCode::HostThrottled,
            restartable: true
        }
    );
    assert_eq!(h.adapter.call_count(), 1);
}

/// The defect this pins: the auth tier re-mints a lapsed frame by calling
/// `begin` again on the SAME flow id (its `mint_step` says so in its own doc),
/// and this driver used to refuse that with a non-restartable `Internal` — so
/// every link whose first frame lapsed terminalized with "cannot be completed
/// for this account". The host flow TTL outlives the step TTL by an order of
/// magnitude, so at a lapse the flow is ALWAYS still live and the refusal
/// ALWAYS fired.
#[tokio::test]
async fn a_second_begin_on_a_live_flow_re_mints_instead_of_failing() {
    let h = harness(DeviceLinkLimits::default()).await;
    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("first begin");

    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("a re-mint of a live flow must succeed, not terminalize the link");

    // The hazard the old refusal guarded is closed a better way: the stale
    // vendor conversation is cancelled before the fresh one starts, so the
    // adapter never runs two at once for one flow.
    let kinds: Vec<_> = h
        .adapter
        .recorded()
        .into_iter()
        .map(|call| call.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            FakeDeviceLinkCallKind::Begin(DeviceLinkMode::Default),
            FakeDeviceLinkCallKind::Cancel,
            FakeDeviceLinkCallKind::Begin(DeviceLinkMode::Default),
        ],
        "a re-mint cancels the stale conversation before starting a fresh one"
    );
}

/// A re-mint is the same attempt asking for a fresh frame, so it must not
/// spend the budget that bounds how many attempts a user may start. With a 60s
/// step clock inside a 600s flow clock, charging re-mints would let one
/// attended link burn an entire hourly budget.
#[tokio::test]
async fn a_re_mint_does_not_spend_the_begin_budget() {
    let limits = DeviceLinkLimits {
        max_begins_per_user: 1,
        ..DeviceLinkLimits::default()
    };
    let h = harness(limits).await;
    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("first begin");

    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("the re-mint rides the attempt that is already paid for");

    // The budget is still spent for a genuinely NEW attempt, which is what it
    // exists to bound.
    let error = h
        .driver
        .begin(&request("flow-b"), DeviceLinkMode::Default)
        .await
        .expect_err("a second distinct attempt is still refused");
    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::HostThrottled,
            restartable: true
        }
    );
}

/// What a re-mint may never buy: forgiveness. The secret-attempt counter is
/// per-flow, so resetting it on re-mint would turn a bounded password-guessing
/// budget into an unbounded one.
#[tokio::test]
async fn a_re_mint_carries_the_secret_attempt_count_forward() {
    let limits = DeviceLinkLimits {
        max_secret_attempts_per_flow: 1,
        ..DeviceLinkLimits::default()
    };
    let h = harness(limits).await;
    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("begin");
    h.driver
        .submit_input(
            &request("flow-a"),
            DeviceLinkInput::Password(SecretString::from("first")),
        )
        .await
        .expect("the one permitted attempt");

    h.driver
        .begin(&request("flow-a"), DeviceLinkMode::Default)
        .await
        .expect("re-mint");

    let error = h
        .driver
        .submit_input(
            &request("flow-a"),
            DeviceLinkInput::Password(SecretString::from("second")),
        )
        .await
        .expect_err("a re-mint must not hand back a fresh guessing budget");
    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::HostThrottled,
            restartable: true
        }
    );
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
    second.scope = product_scope("user-2");
    let error = h
        .driver
        .begin(&second, DeviceLinkMode::Default)
        .await
        .expect_err("the deployment budget is spent");

    assert_eq!(
        error,
        DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::HostThrottled,
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
            code: DeviceLinkErrorCode::HostThrottled,
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
            code: DeviceLinkErrorCode::HostThrottled,
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
            code: DeviceLinkErrorCode::LimitReached,
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
        DeviceLinkDriverError, DeviceLinkPollRequest,
    };

    use super::*;

    fn binding(extension: &str) -> DeviceLinkBinding {
        DeviceLinkBinding {
            provider: AuthProviderId::new("acme-link").expect("provider id"),
            extension_id: ExtensionId::new(extension).expect("extension id"),
            scope: product_scope(USER),
        }
    }

    /// The cross-half suite the port owns, run against the production
    /// implementation.
    ///
    /// This is the check whose absence let `begin` mean two different things
    /// on either side of the seam: both halves were tested, neither test
    /// crossed. Every case lives in `ironclaw_auth` beside the port it
    /// constrains, so a future implementation of `DeviceLinkDriver` inherits
    /// it instead of re-deriving it.
    #[tokio::test]
    async fn the_production_driver_satisfies_the_port_conformance_suite() {
        let h = harness(DeviceLinkLimits::default()).await;
        ironclaw_auth::test_support::conformance::assert_device_link_driver_conformance(
            &h.driver,
            &binding(EXTENSION),
            AuthFlowId::new(),
        )
        .await;
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
        assert!(
            outcome.account.is_none(),
            "no account exists before completion"
        );
        assert_eq!(h.adapter.call_count(), 1);
    }

    /// The port's completion carries the minted account — the fact that lets
    /// the auth-side driver report `Completed` at all (its own contract
    /// terminalizes a completion with `account: None`).
    #[tokio::test]
    async fn the_port_reports_a_completion_with_its_minted_account() {
        let h = harness_with(
            DeviceLinkLimits::default(),
            FakeDeviceLinkAdapter::scripted([super::completed()]),
            true,
        )
        .await;
        let flow_id = AuthFlowId::new();

        let outcome = DeviceLinkDriver::begin(
            &h.driver,
            DeviceLinkBeginRequest {
                flow_id,
                binding: binding(EXTENSION),
                mode: DeviceLinkMode::Default,
            },
        )
        .await
        .expect("completion through the port");

        assert!(matches!(outcome.step, DeviceLinkStep::Completed { .. }));
        let account = outcome
            .account
            .expect("a completed outcome carries the minted account");
        assert_eq!(account.link_revision, 1);
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

    /// A poll after the port reported completion answers `UnknownFlow` — the
    /// auth driver reads a finished flow from its own record, never by
    /// re-driving the vendor.
    #[tokio::test]
    async fn a_poll_after_completion_is_unknown_to_the_vendor_half() {
        let h = harness_with(
            DeviceLinkLimits::default(),
            FakeDeviceLinkAdapter::scripted([super::completed()]),
            true,
        )
        .await;
        let flow_id = AuthFlowId::new();
        DeviceLinkDriver::begin(
            &h.driver,
            DeviceLinkBeginRequest {
                flow_id,
                binding: binding(EXTENSION),
                mode: DeviceLinkMode::Default,
            },
        )
        .await
        .expect("completion through the port");

        let error = DeviceLinkDriver::poll(
            &h.driver,
            DeviceLinkPollRequest {
                flow_id,
                binding: binding(EXTENSION),
            },
        )
        .await
        .expect_err("the completed flow is forgotten");

        assert!(matches!(error, DeviceLinkDriverError::UnknownFlow));
    }
}

// -------------------------------------------------------------------------
// The `ironclaw_auth::LinkedDeviceRevoker` port
// -------------------------------------------------------------------------

/// Unlink teardown, as `ironclaw_auth`'s cleanup drives it. The ordering
/// (logout before unbind) is the auth side's; what these cases pin is that the
/// call reaches the right adapter, scoped to the right account and revision,
/// and that nothing else is talked into logging a device out.
mod linked_device_revoker {
    use ironclaw_auth::{
        CredentialAccountId, LinkedDeviceRevokeError, LinkedDeviceRevokeRequest,
        LinkedDeviceRevoker,
    };

    use super::*;

    fn revoke_request(
        extension: &str,
        account_id: CredentialAccountId,
        link_revision: u64,
    ) -> LinkedDeviceRevokeRequest {
        LinkedDeviceRevokeRequest {
            scope: product_scope(USER),
            extension_id: ExtensionId::new(extension).expect("extension id"),
            account_id,
            link_revision,
        }
    }

    /// The teardown reaches the bound adapter with a grant naming the real
    /// account at its current revision — never the pre-mint provisional ref a
    /// link in progress uses — and the custody handle it opens addresses the
    /// registered account coordinates.
    #[tokio::test]
    async fn revoking_a_linked_device_calls_the_adapter_scoped_to_that_account() {
        let h = harness(DeviceLinkLimits::default()).await;
        let account_id = CredentialAccountId::new();

        h.driver
            .revoke_linked_device(revoke_request(EXTENSION, account_id, 3))
            .await
            .expect("revoke through the port");

        let calls = h.adapter.recorded();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].kind, FakeDeviceLinkCallKind::Revoke);
        assert_eq!(calls[0].user_id.as_str(), USER);
        let grant = calls[0]
            .account
            .as_ref()
            .expect("a revoke always names an established account");
        assert_eq!(grant.account().as_str(), account_id.to_string());
        assert_eq!(grant.link_revision(), 3);

        let keys = h.material.keys();
        assert_eq!(keys.len(), 1, "one handle, one durable-custody access");
        assert_eq!(keys[0].requester_extension.as_str(), EXTENSION);
        assert_eq!(keys[0].account_id, account_id);
        assert_eq!(
            keys[0].link_revision, 3,
            "the custody access is pinned to the revision being revoked"
        );
    }

    /// Revision 0 is the provisional grant a link in progress holds. There is
    /// no device behind it, so the vendor is never asked to log one out.
    #[tokio::test]
    async fn a_provisional_revision_is_refused_without_reaching_the_adapter() {
        let h = harness(DeviceLinkLimits::default()).await;

        let error = h
            .driver
            .revoke_linked_device(revoke_request(EXTENSION, CredentialAccountId::new(), 0))
            .await
            .expect_err("revision 0 names no established device");

        assert_eq!(error, LinkedDeviceRevokeError::Unavailable);
        assert_eq!(h.adapter.call_count(), 0);
    }

    /// "Nothing is bound" stays distinguishable from "the vendor refused" —
    /// both quarantine, and an operator reading the two apart is the point.
    #[tokio::test]
    async fn an_extension_that_binds_no_adapter_answers_no_binding() {
        let h = harness_with(
            DeviceLinkLimits::default(),
            FakeDeviceLinkAdapter::default(),
            false,
        )
        .await;

        let error = h
            .driver
            .revoke_linked_device(revoke_request("acme-chat", CredentialAccountId::new(), 1))
            .await
            .expect_err("nothing is bound");

        assert_eq!(error, LinkedDeviceRevokeError::NoBinding);
        assert_eq!(h.adapter.call_count(), 0);
    }

    /// A vendor that refuses the logout is reported as a vendor failure, which
    /// is what the auth side quarantines on — never a silent success.
    #[tokio::test]
    async fn a_vendor_failure_is_reported_rather_than_swallowed() {
        let adapter = FakeDeviceLinkAdapter::default();
        *adapter.fail_with.lock().expect("fake device-link failure") =
            Some(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::VendorUnavailable,
                restartable: true,
            });
        let h = harness_with(DeviceLinkLimits::default(), adapter, true).await;

        let error = h
            .driver
            .revoke_linked_device(revoke_request(EXTENSION, CredentialAccountId::new(), 1))
            .await
            .expect_err("the vendor refused the logout");

        assert_eq!(error, LinkedDeviceRevokeError::Vendor);
        assert_eq!(h.adapter.call_count(), 1);
    }
}
