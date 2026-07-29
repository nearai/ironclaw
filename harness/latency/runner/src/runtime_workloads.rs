use super::*;

const WEBUI_SESSION_TOKEN: &str = "latency-webui-token";
const WEBUI_SESSION_TENANT: &str = "latency-webui-tenant";
const WEBUI_SESSION_RUNTIME_USER: &str = "latency-webui-user-0";
const WEBUI_SESSION_USER_PREFIX: &str = "latency-webui-user-";
const WEBUI_SESSION_AGENT: &str = "latency-webui-agent";
const WEBUI_SESSION_USER_BUCKETS: usize = 64;

struct LatencyWebuiAuthenticator;

#[async_trait::async_trait]
impl WebuiAuthenticator for LatencyWebuiAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        let user = token.strip_prefix(WEBUI_SESSION_TOKEN)?;
        let user = user.strip_prefix('-')?;
        let user_id = UserId::new(format!("{WEBUI_SESSION_USER_PREFIX}{user}")).ok()?;
        Some(WebuiAuthentication::user(user_id))
    }
}

pub(super) async fn webui_session(
    backend_context: BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let webui = ensure_webui_runtime_context(&backend_context, backend, postgres_pool_size).await?;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/webchat/v2/session")
        .header(
            header::AUTHORIZATION,
            format!(
                "Bearer {WEBUI_SESSION_TOKEN}-{}",
                sample % WEBUI_SESSION_USER_BUCKETS
            ),
        )
        .body(Body::empty())?;
    let response = webui
        .router
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| format!("webui session request failed: {error}"))?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    if status != StatusCode::OK {
        return Err(format!(
            "webui session returned {status}: {}",
            String::from_utf8_lossy(&bytes)
        )
        .into());
    }
    let response: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure_json_field(&response, "tenant_id", WEBUI_SESSION_TENANT)?;
    ensure_json_field(
        &response,
        "user_id",
        &format!(
            "{WEBUI_SESSION_USER_PREFIX}{}",
            sample % WEBUI_SESSION_USER_BUCKETS
        ),
    )?;
    let mut state = stable_hash_bytes(status.as_u16() as u64, &bytes);
    state = state.wrapping_add(option_code(
        response
            .get("features")
            .and_then(|features| features.get("global_auto_approve"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    ));
    Ok(state)
}

pub(super) async fn ensure_webui_runtime_context(
    backend_context: &BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
) -> Result<&WebuiRuntimeContext, Box<dyn std::error::Error + Send + Sync>> {
    let postgres_pool = backend_context.webui_postgres_pool.clone();
    backend_context
        .webui_session
        .get_or_try_init(|| async move {
            build_webui_runtime_context(backend, postgres_pool_size, postgres_pool).await
        })
        .await
}

async fn build_webui_runtime_context(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    postgres_pool: Option<deadpool_postgres::Pool>,
) -> Result<WebuiRuntimeContext, Box<dyn std::error::Error + Send + Sync>> {
    let root = tempfile::tempdir()?;
    let storage_root = root.path().join(format!(
        "webui-{}-{}",
        backend.as_str(),
        uuid::Uuid::new_v4().simple()
    ));
    let workspace_root = root.path().join("workspace");
    let mut build_input = match backend {
        BackendName::Libsql => local_runtime_build_input(
            RebornCompositionProfile::HostedSingleTenantVolume,
            WEBUI_SESSION_RUNTIME_USER,
            storage_root,
        )?,
        BackendName::Postgres => {
            let pool = postgres_pool.ok_or_else(|| {
                format!(
                    "webui session postgres backend missing pool for size {:?}",
                    postgres_pool_size
                )
            })?;
            RebornHostBindings::hosted_single_tenant_postgres(
                RebornCompositionProfile::HostedSingleTenant,
                WEBUI_SESSION_RUNTIME_USER,
                storage_root,
                pool,
                latency_secret_master_key(),
            )?
            .with_runtime_policy(hosted_single_tenant_runtime_policy()?)
        }
    }
    .with_local_runtime_workspace_root(workspace_root);
    let tenant_id = TenantId::new(WEBUI_SESSION_TENANT)?;
    let agent_id = AgentId::new(WEBUI_SESSION_AGENT)?;
    build_input = build_input.with_local_runtime_identity(tenant_id.clone(), agent_id.clone());
    let runtime_input = RebornRuntimeInput::from_build_input(build_input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: WEBUI_SESSION_TENANT.to_string(),
            agent_id: WEBUI_SESSION_AGENT.to_string(),
            source_binding_id: "latency-webui-source".to_string(),
            reply_target_binding_id: "latency-webui-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(10),
        });
    let runtime = build_reborn_runtime(runtime_input).await?;
    let product_surface = runtime.product_surface(None)?;
    let config = WebuiServeConfig::new(
        tenant_id,
        Arc::new(LatencyWebuiAuthenticator),
        vec![HeaderValue::from_static("http://localhost:0")],
    )
    .with_default_agent_id(agent_id);
    let router = webui_v2_app(product_surface, config)?;
    Ok(WebuiRuntimeContext {
        router,
        _runtime: runtime,
        _tempdir: root,
    })
}

fn ensure_json_field(
    value: &serde_json::Value,
    field: &'static str,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let actual = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("webui session response missing `{field}`"))?;
    if actual == expected {
        return Ok(());
    }
    Err(format!("webui session `{field}` was `{actual}`, expected `{expected}`").into())
}

fn stable_hash_bytes(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed ^ 0xcbf29ce484222325, |state, byte| {
        state.wrapping_mul(0x100000001b3) ^ u64::from(*byte)
    })
}

pub(super) async fn hosted_substrate_build(
    backend: BackendName,
    sample: usize,
    postgres_pool_size: Option<usize>,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        BackendName::Libsql => hosted_libsql_substrate_build(sample).await,
        BackendName::Postgres => {
            hosted_postgres_substrate_build(sample, postgres_pool_size.unwrap_or(2)).await
        }
    }
}

async fn hosted_libsql_substrate_build(
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempfile::tempdir()?;
    let state_db_path = dir.path().join(format!("state-{sample}.db"));
    let events_db_path = dir.path().join(format!("events-{sample}.db"));
    let database = Arc::new(
        libsql::Builder::new_local(state_db_path.display().to_string())
            .build()
            .await?,
    );
    let services = build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database,
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: events_db_path.display().to_string(),
            auth_token: None,
        },
        process_local_resource_governor_singleton: true,
        secret_master_key: Some(latency_secret_master_key()),
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        runtime_policy: production_runtime_policy()?,
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: latency_surface_version(sample)?,
    })
    .await?;
    services
        .validate_production_wiring(&hosted_substrate_wiring_config())
        .map_err(|report| format!("hosted libSQL substrate wiring failed: {report:?}"))?;
    Ok(0x71_00_u64 ^ sample as u64)
}

async fn hosted_postgres_substrate_build(
    sample: usize,
    postgres_pool_size: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let url = env::var("IRONCLAW_REBORN_POSTGRES_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5432/ironclaw_latency".to_string()
    });
    let config = url.parse::<tokio_postgres::Config>()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(postgres_pool_size)
        .build()?;
    let services =
        build_postgres_production_host_runtime_services(PostgresProductionSubstrateConfig {
            pool,
            event_store: RebornEventStoreConfig::Postgres {
                url: ironclaw_secrets::SecretMaterial::from(url),
                tls_options: Default::default(),
            },
            process_local_resource_governor_singleton: true,
            secret_master_key: Some(latency_secret_master_key()),
            trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
            runtime_policy: production_runtime_policy()?,
            turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
            surface_version: latency_surface_version(sample)?,
        })
        .await?;
    services
        .validate_production_wiring(&hosted_substrate_wiring_config())
        .map_err(|report| format!("hosted Postgres substrate wiring failed: {report:?}"))?;
    Ok(0x71_00_u64 ^ sample as u64)
}

fn hosted_substrate_wiring_config() -> ProductionWiringConfig {
    ProductionWiringConfig::new([])
        .require_runtime_http_egress()
        .require_credential_broker()
}

fn production_runtime_policy()
-> Result<RebornProductionRuntimePolicy, Box<dyn std::error::Error + Send + Sync>> {
    let policy = EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedSafe,
        resolved_profile: RuntimeProfile::HostedSafe,
        filesystem_backend: FilesystemBackendKind::TenantWorkspace,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::Brokered,
        secret_mode: SecretMode::TenantBroker,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::Standard,
    };
    Ok(
        RebornProductionRuntimePolicy::with_tenant_sandbox_process_port(
            policy,
            Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
                Arc::new(RecordingSandboxTransport),
            )),
        )?,
    )
}

pub(super) fn latency_secret_master_key() -> ironclaw_secrets::SecretMaterial {
    ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901")
}

fn latency_surface_version(
    sample: usize,
) -> Result<CapabilitySurfaceVersion, Box<dyn std::error::Error + Send + Sync>> {
    Ok(CapabilitySurfaceVersion::new(format!("latency-{sample}"))?)
}

#[derive(Debug)]
struct RecordingSandboxTransport;

#[async_trait::async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::ZERO,
        })
    }
}

#[derive(Debug)]
struct RecordingSchedulerWakeNotifier;

impl TurnRunWakeNotifier for RecordingSchedulerWakeNotifier {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        Ok(())
    }
}
