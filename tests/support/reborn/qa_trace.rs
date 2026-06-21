//! Record/replay support for QA-phrase traces against the Reborn runtime.
//!
//! Recording wraps the real Anthropic provider in the existing
//! `ironclaw_llm::recording::RecordingLlm` (the same recorder v1 live tests
//! use — it sits at the `LlmProvider` seam, underneath Reborn's
//! `LlmProviderModelGateway`, so it is runtime-agnostic) and drives a
//! local-dev Reborn runtime with the production model-gateway conversion
//! layer. The flushed JSON is the recorded `LlmTrace` format that
//! `RebornTraceReplayModelGateway::from_trace` replays deterministically.
//!
//! Tool names recorded at this seam are the model-facing names the Reborn
//! gateway advertises, which equal capability ids (`builtin.trigger_create`)
//! for every first-party tool except `builtin.skill_activate` (advertised as
//! `builtin__skill_activate`); the QA phrases do not exercise that tool.

#![allow(dead_code)] // Shared by the QA recorder/replay test binaries only.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount,
    CredentialAccountSelectionRequest, CredentialAccountStatus, CredentialOwnership,
    GOOGLE_GMAIL_READONLY_SCOPE, NewCredentialAccount, ProviderScope,
};
use ironclaw_first_party_extensions::GoogleCredentialResolver;
use ironclaw_host_api::{
    AgentId, ExtensionId, InvocationId, ResourceScope, SecretHandle, TenantId, UserId,
};
use ironclaw_llm::{
    LlmConfig, LlmProvider, NearAiConfig, ProviderProtocol, RegistryProviderConfig, SessionConfig,
    build_static_provider_chain, create_session_manager,
    recording::{
        HttpExchange, HttpExchangeRequest, HttpExchangeResponse, HttpInterceptor, RecordingLlm,
        ReplayingHttpInterceptor,
    },
};
use ironclaw_loop_support::HostManagedModelGateway;
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    PolicyNetworkHttpEgress, ReqwestNetworkTransport,
};
use ironclaw_product_workflow::RebornOutboundDeliveryTargetId;
use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_reborn_composition::{
    AssistantReply, RebornCompositionProfile, RebornLocalRuntimeProfileOptions,
    RebornProductAuthServices, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornTurnDriveOutcome, TriggerPollerSettings, build_reborn_runtime, build_reborn_services,
    local_runtime_build_input_with_options,
};
use ironclaw_reborn_config::{RebornConfigFile, RebornHome};
use ironclaw_triggers::TriggerPollerWorkerConfig;
use ironclaw_turns::{ReplyTargetBindingRef, TurnStatus, run_profile::ModelProfileId};
use secrecy::{ExposeSecret, SecretString};

use crate::support::trace_llm::{LlmTrace, TraceResponse};

pub const QA_RECORD_KEY_ENV: &str = "ANTHROPIC_API_KEY";
pub const QA_RECORD_MODEL_ENV: &str = "IRONCLAW_QA_RECORD_MODEL";
pub const QA_RECORD_DEFAULT_MODEL: &str = "claude-sonnet-4-6";

const QA_TENANT: &str = "qa-trace-tenant";
const QA_USER: &str = "qa-trace-owner";
const QA_AGENT: &str = "qa-trace-agent";
const QA_GOOGLE_ACCESS_HANDLE: &str = "reborn_qa_google_access_token";
const QA_GOOGLE_REFRESH_HANDLE: &str = "reborn_qa_google_refresh_token";
const QA_CREDENTIAL_SOURCE_ROOT_ENV: &str = "IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_ROOT";
const QA_CREDENTIAL_SOURCE_TENANT_ENV: &str = "IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_TENANT";
const QA_CREDENTIAL_SOURCE_USER_ENV: &str = "IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_USER";
const QA_CREDENTIAL_SOURCE_AGENT_ENV: &str = "IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_AGENT";
const LOCAL_DEV_SECRETS_MASTER_KEY_PATH: &str = ".reborn-local-dev-secrets-master-key";

/// Tenant id the QA-trace runtime is composed with — replay assertions need
/// it to query tenant-scoped state (e.g. the trigger repository).
pub fn qa_trace_tenant_id() -> &'static str {
    QA_TENANT
}

/// The model profile id the composed Reborn runtime routes turns through;
/// must match `wrap_swappable_gateway` in `ironclaw_reborn_composition`.
const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

struct LiveCredentialSeed {
    provider: &'static str,
    label: &'static str,
    access_handle: &'static str,
    refresh_handle: Option<&'static str>,
}

const GOOGLE_LIVE_CREDENTIAL: LiveCredentialSeed = LiveCredentialSeed {
    provider: "google",
    label: "qa google",
    access_handle: QA_GOOGLE_ACCESS_HANDLE,
    refresh_handle: Some(QA_GOOGLE_REFRESH_HANDLE),
};

#[derive(Debug)]
struct TraceHttpNetworkEgress {
    interceptor: Arc<dyn HttpInterceptor>,
    inner: PolicyNetworkHttpEgress<ReqwestNetworkTransport>,
    mode: TraceHttpNetworkMode,
}

#[derive(Debug, Clone, Copy)]
enum TraceHttpNetworkMode {
    Recording,
    Replay,
}

impl TraceHttpNetworkEgress {
    fn new(interceptor: Arc<dyn HttpInterceptor>, mode: TraceHttpNetworkMode) -> Self {
        Self {
            interceptor,
            inner: PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::default()),
            mode,
        }
    }
}

#[async_trait]
impl NetworkHttpEgress for TraceHttpNetworkEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let exchange_request = http_exchange_request_from_network(&request);
        if let Some(response) = self.interceptor.before_request(&exchange_request).await {
            return Ok(network_response_from_http_exchange_response(
                response, &request,
            ));
        }

        if matches!(self.mode, TraceHttpNetworkMode::Replay) {
            return Err(NetworkHttpError::Transport {
                reason: format!(
                    "QA HTTP replay fixture did not contain a matching exchange for {} {}",
                    request.method, request.url
                ),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            });
        }

        let response = self.inner.execute(request).await?;
        let exchange_response = HttpExchangeResponse {
            status: response.status,
            headers: response.headers.clone(),
            body: String::from_utf8_lossy(&response.body).to_string(),
        };
        self.interceptor
            .after_response(&exchange_request, &exchange_response)
            .await;
        Ok(response)
    }
}

fn http_exchange_request_from_network(request: &NetworkHttpRequest) -> HttpExchangeRequest {
    HttpExchangeRequest {
        method: request.method.to_string(),
        url: request.url.clone(),
        headers: request.headers.clone(),
        body: (!request.body.is_empty())
            .then(|| String::from_utf8_lossy(&request.body).to_string()),
    }
}

fn network_response_from_http_exchange_response(
    response: HttpExchangeResponse,
    request: &NetworkHttpRequest,
) -> NetworkHttpResponse {
    let body = response.body.into_bytes();
    NetworkHttpResponse {
        status: response.status,
        headers: response.headers,
        usage: NetworkUsage {
            request_bytes: request.body.len() as u64,
            response_bytes: body.len() as u64,
            resolved_ip: None,
        },
        body,
    }
}

pub fn qa_fixture_path(fixture_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/llm_traces/reborn_qa")
        .join(format!("{fixture_name}.json"))
}

pub fn load_qa_trace(fixture_name: &str) -> LlmTrace {
    let path = qa_fixture_path(fixture_name);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "QA trace fixture {} is missing ({error}); record it with the \
             ignored recorder test for this phrase",
            path.display()
        )
    });
    serde_json::from_str(&json).expect("QA trace fixture parses as recorded LlmTrace JSON")
}

pub fn canonical_recorded_tool_name(name: &str) -> String {
    name.strip_prefix("builtin__")
        .map(|suffix| format!("builtin.{suffix}"))
        .unwrap_or_else(|| name.to_string())
}

/// All tool calls in the fixture as canonicalized (name, serialized arguments)
/// pairs. Reborn advertises built-ins to the model as `builtin__foo`; the QA
/// contracts assert the capability-style spelling (`builtin.foo`) so they stay
/// stable if the model-facing escape changes.
pub fn recorded_tool_calls(trace: &LlmTrace) -> Vec<(String, String)> {
    trace
        .turns
        .iter()
        .flat_map(|turn| turn.steps.iter())
        .filter_map(|step| match &step.response {
            TraceResponse::ToolCalls { tool_calls, .. } => Some(tool_calls.iter().map(|call| {
                (
                    canonical_recorded_tool_name(&call.name),
                    normalized_argument_text(&call.arguments),
                )
            })),
            _ => None,
        })
        .flatten()
        .collect()
}

fn normalized_argument_text(arguments: &serde_json::Value) -> String {
    serde_json::to_string(arguments)
        .unwrap_or_default()
        .replace("\\/", "/")
}

fn recorded_trace_has_tool_call(trace: &LlmTrace, tool: &str, argument_fragments: &[&str]) -> bool {
    recorded_tool_calls(trace).iter().any(|(name, arguments)| {
        name == tool
            && argument_fragments
                .iter()
                .all(|fragment| arguments.contains(fragment))
    })
}

/// Clear `expected_tool_results` on every step so a runtime replay re-executes
/// the recorded capability calls against today's runtime without exact-matching
/// nondeterministic tool output (trigger ids, timestamps) captured at record
/// time. Tool-choice contracts are asserted on the raw fixture instead.
pub fn strip_expected_tool_results(trace: &mut LlmTrace) {
    for turn in &mut trace.turns {
        for step in &mut turn.steps {
            step.expected_tool_results.clear();
        }
    }
}

/// Build the local-dev yolo Reborn runtime the QA traces are recorded and
/// replayed against. No trigger poller: the phrases under test create
/// routines, they don't need them to fire.
pub async fn build_qa_trace_runtime(
    root: &tempfile::TempDir,
    gateway: Arc<dyn HostManagedModelGateway>,
) -> RebornRuntime {
    build_qa_trace_runtime_with_http_interceptor(
        root,
        gateway,
        Some((
            Arc::new(ReplayingHttpInterceptor::new(Vec::new())) as Arc<dyn HttpInterceptor>,
            TraceHttpNetworkMode::Replay,
        )),
    )
    .await
}

pub async fn build_qa_trace_runtime_with_http_exchanges(
    root: &tempfile::TempDir,
    gateway: Arc<dyn HostManagedModelGateway>,
    exchanges: Vec<HttpExchange>,
) -> RebornRuntime {
    build_qa_trace_runtime_with_http_interceptor(
        root,
        gateway,
        Some((
            Arc::new(ReplayingHttpInterceptor::new(exchanges)) as Arc<dyn HttpInterceptor>,
            TraceHttpNetworkMode::Replay,
        )),
    )
    .await
}

pub async fn build_qa_trace_runtime_with_http_exchanges_and_trigger_poller(
    root: &tempfile::TempDir,
    gateway: Arc<dyn HostManagedModelGateway>,
    exchanges: Vec<HttpExchange>,
) -> RebornRuntime {
    build_qa_trace_runtime_with_http_interceptor_and_trigger_poller(
        root,
        gateway,
        Some((
            Arc::new(ReplayingHttpInterceptor::new(exchanges)) as Arc<dyn HttpInterceptor>,
            TraceHttpNetworkMode::Replay,
        )),
        true,
    )
    .await
}

async fn build_qa_trace_runtime_with_http_interceptor(
    root: &tempfile::TempDir,
    gateway: Arc<dyn HostManagedModelGateway>,
    http_interceptor: Option<(Arc<dyn HttpInterceptor>, TraceHttpNetworkMode)>,
) -> RebornRuntime {
    build_qa_trace_runtime_with_http_interceptor_and_trigger_poller(
        root,
        gateway,
        http_interceptor,
        false,
    )
    .await
}

async fn build_qa_trace_runtime_with_http_interceptor_and_trigger_poller(
    root: &tempfile::TempDir,
    gateway: Arc<dyn HostManagedModelGateway>,
    http_interceptor: Option<(Arc<dyn HttpInterceptor>, TraceHttpNetworkMode)>,
    trigger_poller_enabled: bool,
) -> RebornRuntime {
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let mut input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        QA_USER,
        root.path().join("local-dev"),
        RebornLocalRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);
    if let Some((interceptor, mode)) = http_interceptor {
        input = input.with_network_http_egress_for_test(Arc::new(TraceHttpNetworkEgress::new(
            interceptor,
            mode,
        )));
    }
    let mut input = RebornRuntimeInput::from_services(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: QA_TENANT.to_string(),
            agent_id: QA_AGENT.to_string(),
            source_binding_id: "qa-trace-source".to_string(),
            reply_target_binding_id: "qa-trace-reply".to_string(),
        })
        .with_model_gateway_override(gateway);
    if trigger_poller_enabled {
        input = input.with_trigger_poller_settings(
            TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test()
                .with_worker_config(TriggerPollerWorkerConfig {
                    poll_interval: Duration::from_millis(20),
                    ..Default::default()
                }),
        );
    }
    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    seed_static_outbound_delivery_targets(&runtime);
    runtime
}

fn seed_static_outbound_delivery_targets(runtime: &RebornRuntime) {
    runtime
        .register_static_outbound_delivery_target_for_test(
            "qa-trace-slack",
            RebornOutboundDeliveryTargetId::new("slack:qa-trace-dm").expect("QA Slack target id"),
            "slack",
            "Slack DM",
            Some("QA trace Slack direct message"),
            ReplyTargetBindingRef::new("reply:qa-trace:slack-dm").expect("QA Slack reply binding"),
        )
        .expect("seed QA Slack delivery target");
    runtime
        .register_static_outbound_delivery_target_for_test(
            "qa-trace-email",
            RebornOutboundDeliveryTargetId::new("email:qa-trace-inbox")
                .expect("QA email target id"),
            "email",
            "Email",
            Some("QA trace email inbox"),
            ReplyTargetBindingRef::new("reply:qa-trace:email").expect("QA email reply binding"),
        )
        .expect("seed QA email delivery target");
}

/// Send one phrase through a fresh conversation and wait for the terminal
/// reply.
pub async fn send_qa_phrase(runtime: &RebornRuntime, phrase: &str) -> AssistantReply {
    let conversation = runtime
        .new_conversation()
        .await
        .expect("new QA conversation");
    runtime
        .send_user_message(&conversation, phrase)
        .await
        .expect("QA phrase turn reaches a terminal state")
}

fn live_credentials_for_fixture(fixture_name: &str) -> &'static [&'static LiveCredentialSeed] {
    match fixture_name {
        "routine_meeting_prep" | "routine_crm_inbox" => &[&GOOGLE_LIVE_CREDENTIAL],
        _ => &[],
    }
}

async fn seed_live_credentials_for_fixture(
    runtime: &RebornRuntime,
    fixture_name: &str,
) -> Vec<(&'static str, String)> {
    let seeds = live_credentials_for_fixture(fixture_name);
    if seeds.is_empty() {
        return Vec::new();
    }
    let source = RebornQaCredentialSource::resolve();
    eprintln!(
        "[RebornQaTrace] importing {} credential account(s) from Reborn source root {} \
         tenant={} user={} agent={}",
        seeds.len(),
        source.local_dev_root.display(),
        source.tenant,
        source.user,
        source.agent
    );
    let source_services = source.build_services().await;
    let source_product_auth = source_services
        .product_auth
        .as_ref()
        .expect("Reborn source runtime exposes product auth services");
    let source_secret_store = source_services.secret_store_for_test();
    let source_auth_scope = AuthProductScope::credential_owner(&source.scope(), AuthSurface::Api);

    let services = runtime.services();
    let product_auth = services
        .product_auth
        .as_ref()
        .expect("QA runtime exposes product auth services");
    let secret_store = services.secret_store_for_test();
    let scope = qa_recording_resource_scope();
    let auth_scope = AuthProductScope::credential_owner(&scope, AuthSurface::Api);
    let mut live_secret_values = Vec::new();
    for seed in seeds {
        let source_account = select_source_credential_account(
            source_product_auth.as_ref(),
            &source,
            &source_auth_scope,
            AuthProviderId::new(seed.provider).expect("provider id"),
            fixture_name,
        )
        .await;
        let source_access = source_account.access_secret.as_ref().unwrap_or_else(|| {
            panic!(
                "configured Reborn product-auth account {:?} for provider {:?} has no access secret",
                source_account.id, seed.provider
            )
        });
        let access_secret_scope =
            resolve_source_secret_scope(&source, &source_account, source_access, "access").await;
        let access_material = consume_source_secret(
            &source,
            source_secret_store.as_ref(),
            &access_secret_scope,
            source_access,
            "access",
            &source_account,
        )
        .await;
        live_secret_values.push((seed.label, access_material.expose_secret().to_string()));
        let access_handle = SecretHandle::new(seed.access_handle).expect("access handle");
        secret_store
            .put(scope.clone(), access_handle.clone(), access_material, None)
            .await
            .expect("seed Reborn access secret");

        let refresh_handle = match (source_account.refresh_secret.as_ref(), seed.refresh_handle) {
            (Some(source_refresh), Some(refresh_handle)) => {
                let refresh_secret_scope = resolve_source_secret_scope(
                    &source,
                    &source_account,
                    source_refresh,
                    "refresh",
                )
                .await;
                let refresh_material = consume_source_secret(
                    &source,
                    source_secret_store.as_ref(),
                    &refresh_secret_scope,
                    source_refresh,
                    "refresh",
                    &source_account,
                )
                .await;
                live_secret_values.push((seed.label, refresh_material.expose_secret().to_string()));
                let handle = SecretHandle::new(refresh_handle).expect("refresh handle");
                secret_store
                    .put(scope.clone(), handle.clone(), refresh_material, None)
                    .await
                    .expect("seed Reborn refresh secret");
                Some(handle)
            }
            (None, Some(_)) => {
                eprintln!(
                    "[RebornQaTrace] source account {:?} for provider {:?} has no refresh \
                     secret; seeded credential will rely on the stored access secret",
                    source_account.id, seed.provider
                );
                None
            }
            _ => None,
        };

        let credential_binding = qa_runtime_credential_binding(&source_account, fixture_name);
        let created_account = product_auth
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: source_account.provider,
                label: source_account.label,
                status: CredentialAccountStatus::Configured,
                ownership: credential_binding.ownership,
                owner_extension: credential_binding.owner_extension,
                granted_extensions: credential_binding.granted_extensions,
                access_secret: Some(access_handle),
                refresh_secret: refresh_handle,
                scopes: source_account.scopes,
            })
            .await
            .expect("seed Reborn credential account");
        preflight_seeded_qa_credential(
            fixture_name,
            product_auth,
            secret_store.as_ref(),
            &created_account,
        )
        .await;
    }
    live_secret_values
}

async fn preflight_seeded_qa_credential(
    fixture_name: &str,
    product_auth: &RebornProductAuthServices,
    secret_store: &dyn ironclaw_secrets::SecretStore,
    account: &CredentialAccount,
) {
    let Some(access_secret) = account.access_secret.as_ref() else {
        panic!(
            "recording fixture {fixture_name:?} imported credential account {:?} for provider \
             {:?} without an access secret",
            account.id, account.provider
        );
    };
    let metadata = secret_store
        .metadata(&account.scope.resource, access_secret)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "recording fixture {fixture_name:?} could not inspect imported access secret {} \
                 for credential account {:?}: {error}",
                access_secret.as_str(),
                account.id
            )
        });
    assert!(
        metadata.is_some(),
        "recording fixture {fixture_name:?} imported credential account {:?} with access secret \
         {} but the QA runtime secret store has no matching secret at that account scope",
        account.id,
        access_secret.as_str()
    );
    if let Some(refresh_secret) = account.refresh_secret.as_ref() {
        let metadata = secret_store
            .metadata(&account.scope.resource, refresh_secret)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "recording fixture {fixture_name:?} could not inspect imported refresh \
                     secret {} for credential account {:?}: {error}",
                    refresh_secret.as_str(),
                    account.id
                )
            });
        assert!(
            metadata.is_some(),
            "recording fixture {fixture_name:?} imported credential account {:?} with refresh \
             secret {} but the QA runtime secret store has no matching secret at that account \
             scope",
            account.id,
            refresh_secret.as_str()
        );
    }

    if account.provider.as_str() != "google" {
        return;
    }
    preflight_seeded_google_credential(fixture_name, product_auth, secret_store, account).await;
}

async fn preflight_seeded_google_credential(
    fixture_name: &str,
    product_auth: &RebornProductAuthServices,
    secret_store: &dyn ironclaw_secrets::SecretStore,
    account: &CredentialAccount,
) {
    let required_extension_scopes = match fixture_name {
        "routine_crm_inbox" => &[GOOGLE_GMAIL_READONLY_SCOPE][..],
        "routine_meeting_prep" => &["https://www.googleapis.com/auth/calendar.readonly"][..],
        _ => &[][..],
    };
    for required_scope in required_extension_scopes {
        assert!(
            account
                .scopes
                .iter()
                .any(|scope| scope.as_str() == *required_scope),
            "recording fixture {fixture_name:?} imported Google credential account {:?} but it \
             does not include required provider scope {required_scope:?}; configured scopes: {:?}",
            account.id,
            account.scopes
        );
    }

    if fixture_name != "routine_crm_inbox" {
        return;
    }

    let gmail = ExtensionId::new("gmail").expect("gmail extension id");
    let readonly = ProviderScope::new(GOOGLE_GMAIL_READONLY_SCOPE)
        .expect("valid Gmail readonly provider scope");
    let resolver = GoogleCredentialResolver::new(
        product_auth.credential_account_service(),
        product_auth.credential_account_record_source_for_test(),
    );
    let credential = resolver
        .resolve(&account.scope.resource, &gmail, &[readonly])
        .await
        .unwrap_or_else(|error| {
            panic!(
                "recording fixture {fixture_name:?} imported Google credential account {:?}, \
                 but first-party Gmail could not resolve it: {error:?}",
                account.id
            )
        });
    let metadata = secret_store
        .metadata(&credential.access_secret_scope, &credential.access_secret)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "recording fixture {fixture_name:?} resolved Gmail access secret {} for account \
                 {:?}, but QA runtime secret metadata lookup failed: {error}",
                credential.access_secret.as_str(),
                credential.account_id
            )
        });
    assert!(
        metadata.is_some(),
        "recording fixture {fixture_name:?} resolved Gmail access secret {} for account {:?}, \
         but QA runtime secret store has no secret at the resolved scope",
        credential.access_secret.as_str(),
        credential.account_id
    );
}

struct QaRuntimeCredentialBinding {
    ownership: CredentialOwnership,
    owner_extension: Option<ExtensionId>,
    granted_extensions: Vec<ExtensionId>,
}

fn qa_runtime_credential_binding(
    source_account: &CredentialAccount,
    fixture_name: &str,
) -> QaRuntimeCredentialBinding {
    if source_account.provider.as_str() != "google" {
        return QaRuntimeCredentialBinding {
            ownership: source_account.ownership,
            owner_extension: source_account.owner_extension.clone(),
            granted_extensions: source_account.granted_extensions.clone(),
        };
    }

    let granted = match fixture_name {
        "routine_crm_inbox" => &["gmail", "google-sheets"][..],
        "routine_meeting_prep" => &["gmail", "google-calendar", "google-drive"][..],
        _ => &[][..],
    };
    if granted.is_empty() {
        return QaRuntimeCredentialBinding {
            ownership: source_account.ownership,
            owner_extension: source_account.owner_extension.clone(),
            granted_extensions: source_account.granted_extensions.clone(),
        };
    }

    let mut granted_extensions = source_account.granted_extensions.clone();
    for extension_id in granted {
        let extension_id = ExtensionId::new(*extension_id).expect("QA extension id");
        if !granted_extensions.contains(&extension_id) {
            granted_extensions.push(extension_id);
        }
    }

    QaRuntimeCredentialBinding {
        ownership: CredentialOwnership::SharedAdminManaged,
        owner_extension: None,
        granted_extensions,
    }
}

struct RebornQaCredentialSource {
    local_dev_root: PathBuf,
    tenant: String,
    user: String,
    agent: String,
}

impl RebornQaCredentialSource {
    fn resolve() -> Self {
        let home = RebornHome::resolve_from_env()
            .unwrap_or_else(|error| panic!("resolve Reborn QA credential source home: {error}"));
        let config_file = RebornConfigFile::load(&home.config_file_path())
            .unwrap_or_else(|error| panic!("load Reborn QA credential source config: {error}"));
        let identity = config_file.as_ref().and_then(|file| file.identity.as_ref());
        let default_identity = RebornRuntimeIdentity::reborn_cli();
        let local_dev_root = std::env::var_os(QA_CREDENTIAL_SOURCE_ROOT_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| home.path().join("local-dev"));
        let tenant = env_or_config_identity(
            QA_CREDENTIAL_SOURCE_TENANT_ENV,
            identity.and_then(|identity| identity.tenant.as_deref()),
            &default_identity.tenant_id,
        );
        let user = env_or_config_identity(
            QA_CREDENTIAL_SOURCE_USER_ENV,
            identity.and_then(|identity| identity.default_owner.as_deref()),
            "reborn-cli",
        );
        let agent = env_or_config_identity(
            QA_CREDENTIAL_SOURCE_AGENT_ENV,
            identity.and_then(|identity| identity.default_agent.as_deref()),
            &default_identity.agent_id,
        );
        Self {
            local_dev_root,
            tenant,
            user,
            agent,
        }
    }

    fn scope(&self) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(&self.tenant).expect("source tenant id"),
            user_id: UserId::new(&self.user).expect("source user id"),
            agent_id: Some(AgentId::new(&self.agent).expect("source agent id")),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    async fn build_services(&self) -> ironclaw_reborn_composition::RebornServices {
        let input = local_runtime_build_input_with_options(
            RebornCompositionProfile::LocalDev,
            &self.user,
            self.local_dev_root.clone(),
            RebornLocalRuntimeProfileOptions::default(),
        )
        .expect("Reborn QA credential source input")
        .with_local_runtime_identity(
            TenantId::new(&self.tenant).expect("source tenant id"),
            AgentId::new(&self.agent).expect("source agent id"),
        );
        build_reborn_services(input)
            .await
            .expect("build Reborn QA credential source services")
    }

    fn matches_account_owner(&self, account: &CredentialAccount) -> bool {
        let resource = &account.scope.resource;
        resource.tenant_id.as_str() == self.tenant
            && resource.user_id.as_str() == self.user
            && resource
                .agent_id
                .as_ref()
                .is_some_and(|agent_id| agent_id.as_str() == self.agent)
            && resource.project_id.is_none()
    }

    fn matches_secret_owner(&self, scope: &ResourceScope) -> bool {
        scope.tenant_id.as_str() == self.tenant
            && scope.user_id.as_str() == self.user
            && scope
                .agent_id
                .as_ref()
                .is_some_and(|agent_id| agent_id.as_str() == self.agent)
            && scope.project_id.is_none()
    }
}

async fn select_source_credential_account(
    product_auth: &ironclaw_reborn_composition::RebornProductAuthServices,
    source: &RebornQaCredentialSource,
    source_auth_scope: &AuthProductScope,
    provider: AuthProviderId,
    fixture_name: &str,
) -> CredentialAccount {
    let record_source = product_auth.credential_account_record_source_for_test();
    match record_source
        .select_unique_configured_account_for_owner(CredentialAccountSelectionRequest::new(
            source_auth_scope.clone(),
            provider.clone(),
        ))
        .await
    {
        Ok(account) => account,
        Err(selection_error) => {
            let visible_accounts = record_source
                .accounts_for_owner(source_auth_scope)
                .await
                .unwrap_or_else(|accounts_error| {
                    panic!(
                        "recording fixture {fixture_name:?} could not list visible Reborn \
                         product-auth accounts for provider {:?} after selection failed: \
                         {selection_error}; list error: {accounts_error}",
                        provider.as_str()
                    )
                });
            if let Some(account) =
                select_unique_visible_source_account(visible_accounts.clone(), source, &provider)
            {
                return account;
            }
            #[cfg(feature = "libsql")]
            match scan_local_dev_db_for_source_account(source, &provider).await {
                Ok(Some(account)) => {
                    eprintln!(
                        "[RebornQaTrace] product-auth record source did not select provider {} \
                         ({selection_error}); using matching local-dev account record from {}",
                        provider.as_str(),
                        source.local_dev_root.display()
                    );
                    return account;
                }
                Ok(None) => {}
                Err(error) => {
                    panic!(
                        "recording fixture {fixture_name:?} could not scan local-dev product-auth \
                         accounts for provider {:?} in {} after selection failed: \
                         {selection_error}; scan error: {error}",
                        provider.as_str(),
                        source.local_dev_root.display()
                    );
                }
            }
            panic!(
                "recording fixture {fixture_name:?} requires exactly one configured \
                 Reborn product-auth account for provider {:?} in source root {} \
                 tenant={} user={} agent={}: {selection_error}. Visible accounts: {}",
                provider.as_str(),
                source.local_dev_root.display(),
                source.tenant,
                source.user,
                source.agent,
                format_account_summaries(&visible_accounts)
            );
        }
    }
}

fn select_unique_visible_source_account(
    accounts: Vec<CredentialAccount>,
    source: &RebornQaCredentialSource,
    provider: &AuthProviderId,
) -> Option<CredentialAccount> {
    let mut matching = accounts
        .into_iter()
        .filter(|account| {
            source.matches_account_owner(account)
                && account.provider == *provider
                && account.status == CredentialAccountStatus::Configured
        })
        .collect::<Vec<_>>();
    match matching.len() {
        1 => matching.pop(),
        _ => None,
    }
}

#[cfg(feature = "libsql")]
async fn scan_local_dev_db_for_source_account(
    source: &RebornQaCredentialSource,
    provider: &AuthProviderId,
) -> Result<Option<CredentialAccount>, String> {
    let db_path = source.local_dev_root.join("reborn-local-dev.db");
    if !db_path.exists() {
        return Ok(None);
    }
    let db = libsql::Builder::new_local(db_path.clone())
        .build()
        .await
        .map_err(|error| format!("open local-dev DB {} failed: {error}", db_path.display()))?;
    let conn = db
        .connect()
        .map_err(|error| format!("connect local-dev DB {} failed: {error}", db_path.display()))?;
    let mut rows = conn
        .query(
            "SELECT contents FROM root_filesystem_entries \
             WHERE path LIKE '%/product-auth/%/accounts/%.json' \
             ORDER BY path",
            (),
        )
        .await
        .map_err(|error| {
            format!(
                "query local-dev credential account records in {} failed: {error}",
                db_path.display()
            )
        })?;
    let mut accounts = Vec::new();
    while let Some(row) = rows.next().await.map_err(|error| {
        format!(
            "iterate local-dev credential account records in {} failed: {error}",
            db_path.display()
        )
    })? {
        let contents: Vec<u8> = row.get(0).map_err(|error| {
            format!(
                "read local-dev credential account record contents from {} failed: {error}",
                db_path.display()
            )
        })?;
        let account = serde_json::from_slice::<CredentialAccount>(&contents).map_err(|error| {
            format!(
                "deserialize local-dev credential account record from {} failed: {error}",
                db_path.display()
            )
        })?;
        accounts.push(account);
    }
    Ok(select_unique_visible_source_account(
        accounts, source, provider,
    ))
}

#[derive(serde::Deserialize)]
struct StoredSecretScopeRecord {
    scope: ResourceScope,
    handle: SecretHandle,
}

#[cfg(feature = "libsql")]
#[derive(serde::Deserialize)]
struct StoredSecretMaterialRecord {
    scope: ResourceScope,
    handle: SecretHandle,
    encrypted_value: Vec<u8>,
    key_salt: Vec<u8>,
}

async fn resolve_source_secret_scope(
    source: &RebornQaCredentialSource,
    account: &CredentialAccount,
    handle: &SecretHandle,
    kind: &str,
) -> ResourceScope {
    #[cfg(feature = "libsql")]
    match scan_local_dev_db_for_secret_scope(source, handle).await {
        Ok(Some(scope)) => return scope,
        Ok(None) => {}
        Err(error) => {
            panic!(
                "scan local-dev DB for {kind} secret {} scope on source account {:?} failed: \
                 {error}",
                handle.as_str(),
                account.id
            );
        }
    }

    eprintln!(
        "[RebornQaTrace] could not find exact local-dev scope for {kind} secret {} on source \
         account {:?}; falling back to account resource scope",
        handle.as_str(),
        account.id
    );
    account.scope.resource.without_thread_and_mission()
}

#[cfg(feature = "libsql")]
async fn scan_local_dev_db_for_secret_scope(
    source: &RebornQaCredentialSource,
    handle: &SecretHandle,
) -> Result<Option<ResourceScope>, String> {
    let db_path = source.local_dev_root.join("reborn-local-dev.db");
    if !db_path.exists() {
        return Ok(None);
    }
    let db = libsql::Builder::new_local(db_path.clone())
        .build()
        .await
        .map_err(|error| format!("open local-dev DB {} failed: {error}", db_path.display()))?;
    let conn = db
        .connect()
        .map_err(|error| format!("connect local-dev DB {} failed: {error}", db_path.display()))?;
    let secret_path_pattern = format!("%/secrets/{}.json", handle.as_str());
    let mut rows = conn
        .query(
            "SELECT contents FROM root_filesystem_entries \
             WHERE path LIKE ?1 \
             ORDER BY path",
            libsql::params![secret_path_pattern],
        )
        .await
        .map_err(|error| {
            format!(
                "query local-dev secret scope records for {} in {} failed: {error}",
                handle.as_str(),
                db_path.display()
            )
        })?;
    let mut matching = Vec::new();
    while let Some(row) = rows.next().await.map_err(|error| {
        format!(
            "iterate local-dev secret scope records for {} in {} failed: {error}",
            handle.as_str(),
            db_path.display()
        )
    })? {
        let contents: Vec<u8> = row.get(0).map_err(|error| {
            format!(
                "read local-dev secret scope record contents for {} from {} failed: {error}",
                handle.as_str(),
                db_path.display()
            )
        })?;
        let record =
            serde_json::from_slice::<StoredSecretScopeRecord>(&contents).map_err(|error| {
                format!(
                    "deserialize local-dev secret scope record for {} from {} failed: {error}",
                    handle.as_str(),
                    db_path.display()
                )
            })?;
        if record.handle == *handle && source.matches_secret_owner(&record.scope) {
            matching.push(record.scope);
        }
    }
    Ok(match matching.len() {
        1 => matching.pop(),
        _ => None,
    })
}

#[cfg(feature = "libsql")]
async fn read_local_dev_db_secret_material(
    source: &RebornQaCredentialSource,
    handle: &SecretHandle,
) -> Result<ironclaw_secrets::SecretMaterial, String> {
    let record = scan_local_dev_db_for_secret_material_record(source, handle)
        .await?
        .ok_or_else(|| {
            format!(
                "no matching encrypted local-dev secret metadata for handle {} in {}",
                handle.as_str(),
                source.local_dev_root.display()
            )
        })?;
    let key = read_local_dev_secret_master_key(source)?;
    let crypto = ironclaw_secrets::SecretsCrypto::new(SecretString::from(key))
        .map_err(|error| format!("local-dev secrets master key is invalid: {error}"))?;
    let aad = ironclaw_secrets::filesystem_secret_aad(&record.scope, &record.handle);
    let decrypted = crypto
        .decrypt(&record.encrypted_value, &record.key_salt, &aad)
        .map_err(|error| {
            format!(
                "decrypt local-dev secret {} with stored scope failed: {error}",
                handle.as_str()
            )
        })?;
    Ok(ironclaw_secrets::SecretMaterial::from(
        decrypted.expose().to_string(),
    ))
}

#[cfg(feature = "libsql")]
async fn scan_local_dev_db_for_secret_material_record(
    source: &RebornQaCredentialSource,
    handle: &SecretHandle,
) -> Result<Option<StoredSecretMaterialRecord>, String> {
    let db_path = source.local_dev_root.join("reborn-local-dev.db");
    if !db_path.exists() {
        return Ok(None);
    }
    let db = libsql::Builder::new_local(db_path.clone())
        .build()
        .await
        .map_err(|error| format!("open local-dev DB {} failed: {error}", db_path.display()))?;
    let conn = db
        .connect()
        .map_err(|error| format!("connect local-dev DB {} failed: {error}", db_path.display()))?;
    let secret_path_pattern = format!("%/secrets/{}.json", handle.as_str());
    let mut rows = conn
        .query(
            "SELECT contents FROM root_filesystem_entries \
             WHERE path LIKE ?1 \
             ORDER BY path",
            libsql::params![secret_path_pattern],
        )
        .await
        .map_err(|error| {
            format!(
                "query local-dev encrypted secret records for {} in {} failed: {error}",
                handle.as_str(),
                db_path.display()
            )
        })?;
    let mut matching = Vec::new();
    while let Some(row) = rows.next().await.map_err(|error| {
        format!(
            "iterate local-dev encrypted secret records for {} in {} failed: {error}",
            handle.as_str(),
            db_path.display()
        )
    })? {
        let contents: Vec<u8> = row.get(0).map_err(|error| {
            format!(
                "read local-dev encrypted secret record contents for {} from {} failed: {error}",
                handle.as_str(),
                db_path.display()
            )
        })?;
        let record =
            serde_json::from_slice::<StoredSecretMaterialRecord>(&contents).map_err(|error| {
                format!(
                    "deserialize local-dev encrypted secret record for {} from {} failed: {error}",
                    handle.as_str(),
                    db_path.display()
                )
            })?;
        if record.handle == *handle && source.matches_secret_owner(&record.scope) {
            matching.push(record);
        }
    }
    Ok(match matching.len() {
        1 => matching.pop(),
        _ => None,
    })
}

#[cfg(feature = "libsql")]
fn read_local_dev_secret_master_key(source: &RebornQaCredentialSource) -> Result<String, String> {
    let key_path = source
        .local_dev_root
        .join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
    let key = match std::fs::read_to_string(&key_path) {
        Ok(existing) => existing.trim().to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::env::var(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV)
                .map(|value| value.trim().to_string())
                .map_err(|_| {
                    format!(
                        "local-dev secrets master key file {} is missing and env var {} is not set",
                        key_path.display(),
                        ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV
                    )
                })?
        }
        Err(error) => {
            return Err(format!(
                "local-dev secrets master key file {} could not be read: {error}",
                key_path.display()
            ));
        }
    };
    ironclaw_secrets::validate_master_key_material(key.as_bytes()).map_err(|error| {
        format!(
            "local-dev secrets master key from {} is malformed: {error}",
            key_path.display()
        )
    })?;
    Ok(key)
}

fn format_account_summaries(accounts: &[CredentialAccount]) -> String {
    if accounts.is_empty() {
        return "<none>".to_string();
    }
    accounts
        .iter()
        .map(|account| {
            format!(
                "id={} provider={} status={:?} tenant={} user={} agent={} project={} thread={} surface={:?} access={} refresh={}",
                account.id,
                account.provider.as_str(),
                account.status,
                account.scope.resource.tenant_id.as_str(),
                account.scope.resource.user_id.as_str(),
                account
                    .scope
                    .resource
                    .agent_id
                    .as_ref()
                    .map(|id| id.as_str())
                    .unwrap_or("<none>"),
                account
                    .scope
                    .resource
                    .project_id
                    .as_ref()
                    .map(|id| id.as_str())
                    .unwrap_or("<none>"),
                account
                    .scope
                    .resource
                    .thread_id
                    .as_ref()
                    .map(|id| id.as_str())
                    .unwrap_or("<none>"),
                account.scope.surface,
                account.access_secret.is_some(),
                account.refresh_secret.is_some()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn env_or_config_identity(name: &str, config_value: Option<&str>, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| config_value.map(ToOwned::to_owned))
        .unwrap_or_else(|| default.to_string())
}

async fn consume_source_secret(
    source: &RebornQaCredentialSource,
    store: &dyn ironclaw_secrets::SecretStore,
    scope: &ResourceScope,
    handle: &SecretHandle,
    kind: &str,
    account: &CredentialAccount,
) -> ironclaw_secrets::SecretMaterial {
    let lease = match store.lease_once(scope, handle).await {
        Ok(lease) => lease,
        Err(lease_error) => {
            #[cfg(feature = "libsql")]
            {
                eprintln!(
                    "[RebornQaTrace] source secret store could not lease {kind} secret {} for \
                     account {:?} ({lease_error}); reading encrypted local-dev secret record \
                     directly",
                    handle.as_str(),
                    account.id
                );
                return read_local_dev_db_secret_material(source, handle)
                    .await
                    .unwrap_or_else(|fallback_error| {
                        panic!(
                            "lease {kind} secret for source Reborn credential account {:?}: \
                             {lease_error}; local-dev fallback failed: {fallback_error}",
                            account.id
                        )
                    });
            }
            #[cfg(not(feature = "libsql"))]
            panic!(
                "lease {kind} secret for source Reborn credential account {:?}: {lease_error}",
                account.id
            )
        }
    };
    match store.consume(scope, lease.id).await {
        Ok(material) => material,
        Err(consume_error) => {
            #[cfg(feature = "libsql")]
            {
                eprintln!(
                    "[RebornQaTrace] source secret store could not consume {kind} secret {} for \
                     account {:?} ({consume_error}); reading encrypted local-dev secret record \
                     directly",
                    handle.as_str(),
                    account.id
                );
                read_local_dev_db_secret_material(source, handle)
                    .await
                    .unwrap_or_else(|fallback_error| {
                        panic!(
                            "consume {kind} secret for source Reborn credential account {:?}: \
                             {consume_error}; local-dev fallback failed: {fallback_error}",
                            account.id
                        )
                    })
            }
            #[cfg(not(feature = "libsql"))]
            panic!(
                "consume {kind} secret for source Reborn credential account {:?}: {consume_error}",
                account.id
            )
        }
    }
}

fn qa_recording_resource_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(QA_TENANT).expect("QA tenant id"),
        user_id: UserId::new(QA_USER).expect("QA user id"),
        agent_id: Some(AgentId::new(QA_AGENT).expect("QA agent id")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn assert_fixture_does_not_contain_live_secret_values(
    fixture_path: &Path,
    secret_values: &[(&'static str, String)],
) {
    if secret_values.is_empty() {
        return;
    }
    let fixture = std::fs::read_to_string(fixture_path).unwrap_or_else(|error| {
        panic!(
            "recorded QA fixture {} could not be read for secret leak check: {error}",
            fixture_path.display()
        )
    });
    for (source_label, secret) in secret_values {
        if secret.len() >= 8 && fixture.contains(secret) {
            panic!(
                "recorded QA fixture {} contains live secret material from {}; \
                 delete the fixture, rotate the credential if needed, and re-record after fixing \
                 redaction",
                fixture_path.display(),
                source_label
            );
        }
    }
}

/// Record one QA phrase against the live Anthropic API and flush the trace to
/// the fixture path. Panics with a clear message when the API key is absent —
/// recorder tests are `#[ignore]`d and only run when explicitly invoked.
pub async fn record_qa_phrase(fixture_name: &str, phrase: &str) {
    let api_key = std::env::var(QA_RECORD_KEY_ENV).unwrap_or_else(|_| {
        panic!("{QA_RECORD_KEY_ENV} must be set to record QA traces against the live API")
    });
    let model = std::env::var(QA_RECORD_MODEL_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| QA_RECORD_DEFAULT_MODEL.to_string());

    let mut live_secret_values = Vec::new();
    live_secret_values.push((QA_RECORD_KEY_ENV, api_key.clone()));

    let config = anthropic_llm_config(api_key, &model);
    let session = create_session_manager(config.session.clone()).await;
    let provider = build_static_provider_chain(&config, session)
        .await
        .expect("anthropic provider chain builds");

    let fixture_path = qa_fixture_path(fixture_name);
    if let Some(parent) = fixture_path.parent() {
        std::fs::create_dir_all(parent).expect("fixture directory");
    }
    let recorder = Arc::new(RecordingLlm::new(
        provider,
        fixture_path.clone(),
        format!("recorded-qa-{fixture_name}"),
    ));

    let profile = ModelProfileId::new(INTERACTIVE_MODEL_PROFILE).expect("model profile id");
    let policy = LlmModelProfilePolicy::new().allow_model_profile(profile, None);
    let gateway =
        LlmProviderModelGateway::new(Arc::clone(&recorder) as Arc<dyn LlmProvider>, policy);

    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_qa_trace_runtime_with_http_interceptor(
        &root,
        Arc::new(gateway),
        Some((recorder.http_interceptor(), TraceHttpNetworkMode::Recording)),
    )
    .await;
    live_secret_values.extend(seed_live_credentials_for_fixture(&runtime, fixture_name).await);
    // Drive the phrase to a terminal status *or* the first gate it raises.
    // Using `send_user_message_until_gate` (not `send_user_message`) means an
    // OAuth/approval-gated phrase records the agent's decisions up to the gate
    // and reports the pause, instead of parking in the non-terminal
    // `BlockedAuth` state until `RunTimeout`. Resolving the gate to record the
    // post-auth turns is a deliberate follow-up that goes through the WebUI
    // facade with a seeded credential — not wired here.
    let conversation = runtime
        .new_conversation()
        .await
        .expect("new QA conversation");
    let outcome = runtime
        .send_user_message_until_gate(&conversation, phrase)
        .await
        .expect("QA phrase reaches a terminal status or a gate");
    runtime.shutdown().await.expect("runtime shutdown");

    recorder.flush().await.expect("flush recorded QA trace");
    assert_fixture_does_not_contain_live_secret_values(&fixture_path, &live_secret_values);
    assert_recorded_fixture_matches_expected_result(fixture_name, &fixture_path, &outcome);
    match outcome {
        RebornTurnDriveOutcome::Terminal(reply) => {
            assert!(
                reply.is_successful_final_reply(),
                "recorded QA phrase {fixture_name:?} did not complete successfully \
                 (status {:?}); trace still flushed to {} for inspection — scrub and \
                 re-record before committing",
                reply.status,
                fixture_path.display()
            );
            println!(
                "recorded QA trace {} (reply: {})",
                fixture_path.display(),
                reply.text.as_deref().unwrap_or("<none>")
            );
        }
        RebornTurnDriveOutcome::BlockedOnGate {
            status,
            gate_ref,
            partial_text,
            ..
        } => {
            // A gate pause is the expected recordable outcome for phrases that
            // require interactive auth/approval (e.g. "connect to Gmail"): the
            // agent routed to the gate, which is exactly what the contract for
            // those phrases pins. The trace is flushed up to the gate.
            println!(
                "recorded QA trace {} (paused at gate: status {:?}, gate_ref {}, partial reply: {})",
                fixture_path.display(),
                status,
                gate_ref.as_str(),
                partial_text.as_deref().unwrap_or("<none>")
            );
        }
    }

    // Give the recording a 2s settle so background turn-state writes finish
    // before the tempdir drops.
    tokio::time::sleep(Duration::from_secs(2)).await;
}

fn assert_recorded_fixture_matches_expected_result(
    fixture_name: &str,
    fixture_path: &Path,
    outcome: &RebornTurnDriveOutcome,
) {
    let trace = load_recorded_trace_from_path(fixture_path);
    let requires_terminal_reply = !matches!(fixture_name, "connect_gmail");
    if requires_terminal_reply {
        match outcome {
            RebornTurnDriveOutcome::Terminal(reply) if reply.is_successful_final_reply() => {}
            RebornTurnDriveOutcome::Terminal(reply) => {
                panic!(
                    "recorded QA fixture {fixture_name:?} ended with non-success terminal \
                     status {:?}; trace was flushed to {} for inspection",
                    reply.status,
                    fixture_path.display()
                );
            }
            RebornTurnDriveOutcome::BlockedOnGate {
                status, gate_ref, ..
            } => {
                panic!(
                    "recorded QA fixture {fixture_name:?} paused at gate status {:?} \
                     ({}) but this fixture is expected to complete a real action; trace was \
                     flushed to {} for inspection",
                    status,
                    gate_ref.as_str(),
                    fixture_path.display()
                );
            }
        }
    }

    match fixture_name {
        "connect_gmail" => {
            assert_blocked_auth_outcome(fixture_name, fixture_path, outcome);
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.extension_install",
                &["gmail"],
            );
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.extension_activate",
                &["gmail"],
            );
        }
        "routine_health_ping" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.trigger_create",
                &["*/5 * * * *", "cloud-api.near.ai/health"],
            );
        }
        "routine_meeting_prep" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.trigger_create",
                &["*/30 * * * *"],
            );
        }
        "routine_release_watch" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.trigger_create",
                &["*/5 * * * *", "github.com/nearai/ironclaw"],
            );
        }
        "routine_crm_inbox" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.trigger_create",
                &["*/30 * * * *", "near.ai", "ABC"],
            );
        }
        "routine_hn_monitor" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.trigger_create",
                &["0 * * * *", "Hacker News"],
            );
        }
        "web_status_check" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.http",
                &["api.github.com"],
            );
        }
        "web_release_summary" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.http",
                &["nearai/ironclaw"],
            );
        }
        "web_hn_search" => {
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.http",
                &["IronClaw"],
            );
            assert_recorded_tool_call(
                fixture_name,
                fixture_path,
                &trace,
                "builtin.http",
                &["NEAR"],
            );
        }
        _ => {}
    }
}

fn load_recorded_trace_from_path(fixture_path: &Path) -> LlmTrace {
    let json = std::fs::read_to_string(fixture_path).unwrap_or_else(|error| {
        panic!(
            "recorded QA fixture {} could not be read for result validation: {error}",
            fixture_path.display()
        )
    });
    serde_json::from_str(&json).unwrap_or_else(|error| {
        panic!(
            "recorded QA fixture {} could not be parsed for result validation: {error}",
            fixture_path.display()
        )
    })
}

fn assert_blocked_auth_outcome(
    fixture_name: &str,
    fixture_path: &Path,
    outcome: &RebornTurnDriveOutcome,
) {
    match outcome {
        RebornTurnDriveOutcome::BlockedOnGate {
            status: TurnStatus::BlockedAuth,
            ..
        } => {}
        RebornTurnDriveOutcome::BlockedOnGate {
            status, gate_ref, ..
        } => {
            panic!(
                "recorded QA fixture {fixture_name:?} paused at unexpected gate status {:?} \
                 ({}); trace was flushed to {} for inspection",
                status,
                gate_ref.as_str(),
                fixture_path.display()
            );
        }
        RebornTurnDriveOutcome::Terminal(reply) => {
            panic!(
                "recorded QA fixture {fixture_name:?} should pause at an auth gate but ended \
                 terminal with status {:?}; trace was flushed to {} for inspection",
                reply.status,
                fixture_path.display()
            );
        }
    }
}

fn assert_recorded_tool_call(
    fixture_name: &str,
    fixture_path: &Path,
    trace: &LlmTrace,
    tool: &str,
    argument_fragments: &[&str],
) {
    if recorded_trace_has_tool_call(trace, tool, argument_fragments) {
        return;
    }
    panic!(
        "recorded QA fixture {fixture_name:?} did not perform expected action {tool} with \
         arguments containing {argument_fragments:?}; recorded calls: {:#?}; trace was \
         flushed to {} for inspection",
        recorded_tool_calls(trace),
        fixture_path.display()
    );
}

fn assert_recorded_final_reply_contains(
    fixture_name: &str,
    fixture_path: &Path,
    trace: &LlmTrace,
    fragments: &[&str],
) {
    let final_reply = trace
        .turns
        .iter()
        .flat_map(|turn| turn.steps.iter())
        .rev()
        .find_map(|step| match &step.response {
            TraceResponse::Text { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "recorded QA fixture {fixture_name:?} did not include a final text reply; trace \
                 was flushed to {} for inspection",
                fixture_path.display()
            )
        });
    assert!(
        fragments
            .iter()
            .all(|fragment| final_reply.contains(fragment)),
        "recorded QA fixture {fixture_name:?} final reply did not contain {fragments:?}; reply: \
         {final_reply:?}; trace was flushed to {} for inspection",
        fixture_path.display()
    );
}

fn anthropic_llm_config(api_key: String, model: &str) -> LlmConfig {
    LlmConfig {
        backend: "anthropic".to_string(),
        session: SessionConfig::default(),
        nearai: NearAiConfig {
            model: model.to_string(),
            cheap_model: None,
            base_url: "https://cloud-api.near.ai/v1".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 1,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: Some(RegistryProviderConfig::generic(
            ProviderProtocol::Anthropic,
            "anthropic",
            Some(SecretString::from(api_key)),
            "https://api.anthropic.com",
            model,
        )),
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 120,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 1,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        AgentId, InvocationId, NetworkMethod, NetworkPolicy, ResourceScope, TenantId, UserId,
    };
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelErrorKind, HostManagedModelRequest,
        HostManagedModelResponse,
    };

    struct UnusedModelGateway;

    #[async_trait]
    impl HostManagedModelGateway for UnusedModelGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "QA credential preflight does not call the model",
            ))
        }
    }

    #[derive(Debug)]
    struct StaticHttpInterceptor;

    #[async_trait]
    impl HttpInterceptor for StaticHttpInterceptor {
        async fn before_request(
            &self,
            request: &HttpExchangeRequest,
        ) -> Option<HttpExchangeResponse> {
            assert_eq!(request.method, "get");
            assert_eq!(request.url, "https://api.example.test/data");
            Some(HttpExchangeResponse {
                status: 207,
                headers: vec![("x-fixture".to_string(), "yes".to_string())],
                body: "fixture body".to_string(),
            })
        }

        async fn after_response(
            &self,
            _request: &HttpExchangeRequest,
            _response: &HttpExchangeResponse,
        ) {
            panic!("replayed exchange should not call the real network");
        }
    }

    #[tokio::test]
    async fn trace_http_network_egress_replays_intercepted_response() {
        let egress = TraceHttpNetworkEgress::new(
            Arc::new(StaticHttpInterceptor),
            TraceHttpNetworkMode::Replay,
        );
        let response = egress
            .execute(NetworkHttpRequest {
                scope: ResourceScope {
                    tenant_id: TenantId::new("tenant1").unwrap(),
                    user_id: UserId::new("user1").unwrap(),
                    agent_id: Some(AgentId::new("agent1").unwrap()),
                    project_id: None,
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                method: NetworkMethod::Get,
                url: "https://api.example.test/data".to_string(),
                headers: Vec::new(),
                body: Vec::new(),
                policy: NetworkPolicy::default(),
                response_body_limit: Some(1024),
                timeout_ms: None,
            })
            .await
            .expect("intercepted replay response");

        assert_eq!(response.status, 207);
        assert_eq!(
            response.headers,
            vec![("x-fixture".to_string(), "yes".to_string())]
        );
        assert_eq!(response.body, b"fixture body");
        assert_eq!(response.usage.response_bytes, 12);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn local_dev_db_secret_material_reader_decrypts_record() {
        let dir = tempfile::tempdir().unwrap();
        let source = RebornQaCredentialSource {
            local_dev_root: dir.path().to_path_buf(),
            tenant: "reborn-cli".to_string(),
            user: "reborn-cli".to_string(),
            agent: "reborn-cli-agent".to_string(),
        };
        let master_key = ironclaw_secrets::keychain::generate_master_key_hex();
        std::fs::write(
            dir.path().join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH),
            &master_key,
        )
        .unwrap();

        let handle = SecretHandle::new("google-oauth-access-test").unwrap();
        let scope = source.scope();
        let crypto = ironclaw_secrets::SecretsCrypto::new(SecretString::from(master_key)).unwrap();
        let aad = ironclaw_secrets::filesystem_secret_aad(&scope, &handle);
        let (encrypted_value, key_salt) = crypto
            .encrypt(b"local-dev-secret-value", &aad)
            .expect("encrypt fixture secret");
        let record = serde_json::json!({
            "scope": scope,
            "handle": handle,
            "encrypted_value": encrypted_value,
            "key_salt": key_salt,
        });

        let db = libsql::Builder::new_local(dir.path().join("reborn-local-dev.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute(
            "CREATE TABLE root_filesystem_entries (path TEXT PRIMARY KEY, contents BLOB NOT NULL)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO root_filesystem_entries (path, contents) VALUES (?1, ?2)",
            libsql::params![
                "/tenants/reborn-cli/users/reborn-cli/secrets/agents/reborn-cli-agent/secrets/google-oauth-access-test.json",
                serde_json::to_vec(&record).unwrap(),
            ],
        )
        .await
        .unwrap();

        let material = read_local_dev_db_secret_material(
            &source,
            &SecretHandle::new("google-oauth-access-test").unwrap(),
        )
        .await
        .expect("read encrypted local-dev secret");

        assert_eq!(material.expose_secret(), "local-dev-secret-value");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    #[ignore = "requires IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_ROOT with a live Google credential"]
    async fn reborn_qa_crm_google_credential_preflight_from_source_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_qa_trace_runtime(&root, Arc::new(UnusedModelGateway)).await;

        let live_secret_values =
            seed_live_credentials_for_fixture(&runtime, "routine_crm_inbox").await;

        assert!(
            !live_secret_values.is_empty(),
            "CRM fixture should import Google secret material for preflight"
        );
    }

    #[test]
    fn qa_runtime_credential_binding_grants_crm_google_extensions() {
        let account = CredentialAccount {
            id: ironclaw_auth::CredentialAccountId::new(),
            scope: AuthProductScope::new(qa_recording_resource_scope(), AuthSurface::Api),
            provider: AuthProviderId::new("google").unwrap(),
            label: ironclaw_auth::CredentialAccountLabel::new("qa google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("qa-google-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let binding = qa_runtime_credential_binding(&account, "routine_crm_inbox");

        assert_eq!(binding.ownership, CredentialOwnership::SharedAdminManaged);
        assert_eq!(binding.owner_extension, None);
        assert!(
            binding
                .granted_extensions
                .contains(&ExtensionId::new("gmail").unwrap())
        );
        assert!(
            binding
                .granted_extensions
                .contains(&ExtensionId::new("google-sheets").unwrap())
        );
    }

    #[test]
    fn qa_runtime_credential_binding_grants_meeting_prep_google_extensions() {
        let account = CredentialAccount {
            id: ironclaw_auth::CredentialAccountId::new(),
            scope: AuthProductScope::new(qa_recording_resource_scope(), AuthSurface::Api),
            provider: AuthProviderId::new("google").unwrap(),
            label: ironclaw_auth::CredentialAccountLabel::new("qa google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("qa-google-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let binding = qa_runtime_credential_binding(&account, "routine_meeting_prep");

        assert_eq!(binding.ownership, CredentialOwnership::SharedAdminManaged);
        for extension_id in ["gmail", "google-calendar", "google-drive"] {
            assert!(
                binding
                    .granted_extensions
                    .contains(&ExtensionId::new(extension_id).unwrap()),
                "{extension_id} should be granted"
            );
        }
    }
}
