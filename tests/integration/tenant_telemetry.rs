//! Production trigger settlement is durably projected into tenant-scoped BI
//! telemetry, and remains isolated after a real embedded-libSQL reopen.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::AutoApproveSettingInput;
use ironclaw_composition::{
    RebornCompositionProfile, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornRuntimeProfileOptions, TriggerPollerSettings, build_reborn_runtime,
    local_runtime_build_input_with_options,
};
use ironclaw_host_api::{
    action::NetworkPolicy,
    capability::{CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints},
    ids::{AgentId, CapabilityGrantId, CapabilityId, ExtensionId, RunId, TenantId, UserId},
    mount::MountView,
    resource::{ResourceEstimate, ResourceScope},
    runtime::{RuntimeKind, TrustClass},
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{RuntimeCapabilityOutcome, TRIGGER_CREATE_CAPABILITY_ID};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_telemetry::TelemetryPageRequest;
use ironclaw_triggers::{TriggerId, TriggerPollerWorkerConfig, TriggerRunStatus, TriggerState};
use serde_json::{Value, json};
use tokio::sync::Mutex;

const TENANT: &str = "tenant-telemetry-proof";
const OTHER_TENANT: &str = "tenant-telemetry-other";
const USER: &str = "owner-telemetry-proof";
const AGENT: &str = "agent-telemetry-proof";
const TEST_SECRET_MASTER_KEY: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Debug, Default)]
struct RecordingGateway {
    requests: Mutex<Vec<HostManagedModelRequest>>,
    fail: bool,
}

impl RecordingGateway {
    fn failing() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            fail: true,
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().await.push(request);
        if self.fail {
            return Ok(HostManagedModelResponse::assistant_reply(""));
        }
        Ok(HostManagedModelResponse::assistant_reply(
            "telemetry proof ok",
        ))
    }
}

async fn build_runtime(root: &tempfile::TempDir) -> (RebornRuntime, Arc<RecordingGateway>) {
    build_runtime_with_gateway(root, Arc::new(RecordingGateway::default())).await
}

async fn build_runtime_with_gateway(
    root: &tempfile::TempDir,
    gateway: Arc<RecordingGateway>,
) -> (RebornRuntime, Arc<RecordingGateway>) {
    seed_test_secret_master_key(root.path());
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::StandaloneUnrestricted,
        USER,
        root.path().join("local-dev"),
        RebornRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local runtime input")
    .with_local_runtime_confirmed_host_home_root(host_home_root);
    let input = RebornRuntimeInput::from_build_input(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "tenant-telemetry-source".to_string(),
            reply_target_binding_id: "tenant-telemetry-reply".to_string(),
        })
        .with_trigger_poller_settings(
            TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test()
                .with_worker_config(TriggerPollerWorkerConfig {
                    poll_interval: Duration::from_millis(20),
                    ..Default::default()
                }),
        )
        .with_model_gateway_override(gateway.clone());
    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    runtime
        .standalone_auto_approve_settings_for_test()
        .expect("auto-approve settings")
        .set(AutoApproveSettingInput {
            scope: trigger_execution_context().resource_scope,
            enabled: true,
            updated_by: Principal::User(UserId::new(USER).expect("user id")),
        })
        .await
        .expect("seed auto-approve");
    (runtime, gateway)
}

fn seed_test_secret_master_key(root: &Path) {
    let storage_root = root.join("local-dev");
    std::fs::create_dir_all(&storage_root).expect("local-dev root");
    let key_path = storage_root.join(".reborn-local-dev-secrets-master-key");
    if !key_path.exists() {
        std::fs::write(key_path, TEST_SECRET_MASTER_KEY).expect("seed test secret master key");
    }
}

fn execution_contract() -> Value {
    json!({
        "version": 1,
        "goal": "record one completed automation settlement",
        "success_criteria": ["Complete the requested routine task"],
        "output_instructions": "Return a concise result",
        "no_result_text": "No result",
        "policy": { "result_delivery": "deliver" }
    })
}

async fn create_trigger(runtime: &RebornRuntime) -> TriggerId {
    let due_at = (Utc::now() + chrono::Duration::seconds(1))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let host_runtime = runtime.host_runtime_for_test().expect("host runtime");
    let outcome = host_runtime
        .invoke_capability((
            trigger_execution_context(),
            CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
            ResourceEstimate::default(),
            json!({
                "name": "Tenant telemetry proof",
                "execution_contract": execution_contract(),
                "schedule": {
                    "kind": "once",
                    "at": due_at,
                    "timezone": "UTC"
                }
            }),
        ))
        .await
        .expect("trigger create invocation");
    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected trigger create completion, got {outcome:?}");
    };
    TriggerId::parse(
        completed.output["trigger"]["trigger_id"]
            .as_str()
            .expect("trigger id"),
    )
    .expect("valid trigger id")
}

fn trigger_execution_context() -> ExecutionContext {
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let extension_id = ExtensionId::new("tenant-telemetry-caller").expect("extension id");
    let mut context = ExecutionContext::local_default(
        user_id,
        extension_id.clone(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::DispatchCapability,
                        EffectKind::ExternalWrite,
                    ],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::default(),
    )
    .expect("execution context");
    context.tenant_id = tenant_id.clone();
    context.agent_id = Some(agent_id.clone());
    context.project_id = None;
    context.resource_scope.tenant_id = tenant_id;
    context.resource_scope.agent_id = Some(agent_id);
    context.resource_scope.project_id = None;
    context.run_id = Some(RunId::new());
    context
}

fn telemetry_scope(tenant: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).expect("tenant id"),
        user_id: UserId::new(USER).expect("user id"),
        agent_id: Some(AgentId::new(AGENT).expect("agent id")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
    }
}

#[tokio::test]
async fn production_trigger_settlement_survives_libsql_reopen_and_stays_tenant_scoped() {
    let root = tempfile::tempdir().expect("tempdir");
    let (runtime, gateway) = build_runtime(&root).await;
    let trigger_id = create_trigger(&runtime).await;
    let trigger_repo = runtime.trigger_repository();
    let tenant_id = TenantId::new(TENANT).expect("tenant id");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut settled = None;
    while Instant::now() < deadline {
        let current = trigger_repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("read settled trigger")
            .expect("settled trigger");
        if current.last_status == Some(TriggerRunStatus::Ok)
            && current.last_fired_slot.is_some()
            && current.last_run_at.is_some()
            && current.active_fire_slot.is_none()
            && current.state == TriggerState::Completed
        {
            settled = Some(current);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let settled = settled.expect("poller durably settled and cleaned up trigger fire");
    assert_eq!(settled.last_status, Some(TriggerRunStatus::Ok));
    assert_eq!(gateway.requests.lock().await.len(), 1);

    // Runtime shutdown closes telemetry intake and awaits its owned worker,
    // making the following reopen a proof of durable callback completion.
    runtime.shutdown().await.expect("runtime shutdown");

    let telemetry =
        ironclaw_composition::test_support::open_standalone_telemetry_repository_for_test(
            &root.path().join("local-dev"),
        )
        .await
        .expect("fresh telemetry repository");
    let now = Utc::now();
    let request = TelemetryPageRequest::new(
        now - chrono::Duration::hours(1),
        now + chrono::Duration::hours(1),
        now,
        10,
        None,
    )
    .expect("telemetry request")
    .with_include_partial(true);
    let rows = telemetry
        .read_automation_page(&telemetry_scope(TENANT), &request)
        .await
        .expect("read original tenant telemetry");
    assert_eq!(rows.rows().len(), 1, "exactly one automation aggregate");
    let row = &rows.rows()[0];
    assert_eq!(row.tenant_id().as_str(), TENANT);
    assert_eq!(row.user_id().as_str(), USER);
    assert_eq!(
        row.automation_kind(),
        ironclaw_telemetry_contracts::observation::AutomationKind::Once
    );
    assert_eq!(row.run_count(), 1);
    assert_eq!(row.completed_count(), 1);
    assert_eq!(row.failed_count(), 0);
    assert_eq!(row.cancelled_count(), 0);
    assert_eq!(row.recovery_required_count(), 0);

    let foreign_rows = telemetry
        .read_automation_page(&telemetry_scope(OTHER_TENANT), &request)
        .await
        .expect("read foreign tenant telemetry");
    assert!(
        foreign_rows.rows().is_empty(),
        "foreign tenant must see zero rows"
    );
}

#[tokio::test]
async fn failed_production_trigger_settlement_survives_libsql_reopen_and_stays_tenant_scoped() {
    let root = tempfile::tempdir().expect("tempdir");
    let (runtime, gateway) =
        build_runtime_with_gateway(&root, Arc::new(RecordingGateway::failing())).await;
    let trigger_id = create_trigger(&runtime).await;
    let trigger_repo = runtime.trigger_repository();
    let tenant_id = TenantId::new(TENANT).expect("tenant id");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut settled = false;
    while Instant::now() < deadline {
        let current = trigger_repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("read failed settled trigger")
            .expect("failed settled trigger");
        if gateway.requests.lock().await.len() >= 4
            && current.last_fired_slot.is_some()
            && current.active_fire_slot.is_none()
            && current.state == TriggerState::Completed
        {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !settled {
        panic!(
            "poller did not settle failed trigger fire: {:?}; model requests: {}",
            trigger_repo
                .get_trigger(tenant_id.clone(), trigger_id)
                .await
                .expect("read failed trigger after timeout"),
            gateway.requests.lock().await.len()
        );
    }
    assert!(gateway.requests.lock().await.len() >= 4);

    runtime.shutdown().await.expect("runtime shutdown");

    let telemetry =
        ironclaw_composition::test_support::open_standalone_telemetry_repository_for_test(
            &root.path().join("local-dev"),
        )
        .await
        .expect("fresh telemetry repository");
    let now = Utc::now();
    let request = TelemetryPageRequest::new(
        now - chrono::Duration::hours(1),
        now + chrono::Duration::hours(1),
        now,
        10,
        None,
    )
    .expect("telemetry request")
    .with_include_partial(true);
    let rows = telemetry
        .read_automation_page(&telemetry_scope(TENANT), &request)
        .await
        .expect("read original tenant failure telemetry");
    assert_eq!(rows.rows().len(), 1, "exactly one automation aggregate");
    let row = &rows.rows()[0];
    assert_eq!(row.tenant_id().as_str(), TENANT);
    assert_eq!(row.user_id().as_str(), USER);
    assert_eq!(
        row.automation_kind(),
        ironclaw_telemetry_contracts::observation::AutomationKind::Once
    );
    assert_eq!(row.run_count(), 1);
    assert_eq!(row.completed_count(), 0);
    assert_eq!(row.failed_count(), 1);
    assert_eq!(row.cancelled_count(), 0);
    assert_eq!(row.recovery_required_count(), 0);

    let foreign_rows = telemetry
        .read_automation_page(&telemetry_scope(OTHER_TENANT), &request)
        .await
        .expect("read foreign tenant failure telemetry");
    assert!(
        foreign_rows.rows().is_empty(),
        "foreign tenant must see zero rows"
    );
}
