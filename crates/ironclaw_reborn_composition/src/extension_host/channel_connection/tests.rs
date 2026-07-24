use std::collections::HashMap;
use std::sync::Mutex;

use ironclaw_filesystem::{
    Fault, FaultInjecting, FilesystemOperation, InMemoryBackend, RootFilesystem,
};
use ironclaw_host_api::{AgentId, UserId};
use ironclaw_product::AdapterInstallationId;

use super::*;
use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;
use crate::provider_identity::{
    RebornUserIdentityBindingError, RebornUserIdentityLookupError,
    installation_scoped_provider_user_id,
};

const VENDOR: &str = "acmechat";
const EXTENSION: &str = "acmechat";

fn tenant() -> TenantId {
    TenantId::new("tenant:test").expect("tenant")
}

fn caller() -> ProductSurfaceCaller {
    ProductSurfaceCaller::new(
        tenant(),
        UserId::new("user:alice").expect("user"),
        None::<AgentId>,
        None,
    )
}

fn scope(installation: &str) -> ChannelConnectionScope {
    ChannelConnectionScope {
        installation_id: AdapterInstallationId::new(installation).expect("installation"),
        expected_team_id: Some("T123".to_string()),
        expected_enterprise_id: None,
        expected_app_id: Some("A123".to_string()),
    }
}

fn workflow_state_service() -> Arc<ChannelWorkflowStateService> {
    Arc::new(ChannelWorkflowStateService::new(Arc::new(
        InMemoryBackend::new(),
    )))
}

fn facade(
    scope: Option<ChannelConnectionScope>,
    identity_store: Arc<RecordingIdentityStore>,
    credential_cleanup: Option<Arc<dyn ChannelCredentialCleanup>>,
) -> GenericChannelConnectionFacade {
    GenericChannelConnectionFacade::new(
        tenant(),
        vec![ChannelConnectionEntry {
            extension_id: EXTENSION.to_string(),
            providers: vec![VENDOR.to_string()],
            scope_source: Arc::new(StaticScopeSource(scope)),
        }],
        None,
        identity_store.clone(),
        identity_store,
        credential_cleanup,
        None,
        None,
        workflow_state_service(),
        None,
    )
}

fn bound_identity_store(installation: &str) -> Arc<RecordingIdentityStore> {
    let installation_id = AdapterInstallationId::new(installation).expect("installation");
    Arc::new(RecordingIdentityStore::new([(
        installation_scoped_provider_user_id(&installation_id, "U123"),
        UserId::new("user:alice").expect("user"),
    )]))
}

#[tokio::test]
async fn facade_disconnects_identity_and_credentials_in_order() {
    let identity_store = bound_identity_store("install-alpha");
    let credential_cleanup = Arc::new(RecordingCredentialCleanup::default());
    let facade = facade(
        Some(scope("install-alpha")),
        identity_store.clone(),
        Some(credential_cleanup.clone()),
    );
    let caller = caller();

    assert_eq!(
        facade
            .caller_channel_connections(caller.clone())
            .await
            .expect("connection lookup"),
        HashMap::from([(EXTENSION.to_string(), true)])
    );

    facade
        .disconnect_channel_for_caller(caller.clone(), EXTENSION)
        .await
        .expect("disconnect succeeds");

    // Disconnect must revoke the caller's personal credential through
    // the product-auth lifecycle cleanup port, scoped to exactly this
    // tenant + caller, the extension, and its vendor.
    let requests = credential_cleanup.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].extension_id.as_str(), EXTENSION);
    assert_eq!(
        requests[0].provider.as_ref().map(|p| p.as_str()),
        Some(VENDOR),
        "the provider selector is what reaches the grant-less OAuth account"
    );
    assert_eq!(requests[0].action, SecretCleanupAction::Uninstall);
    assert_eq!(&requests[0].scope.resource.tenant_id, &tenant());
    assert_eq!(&requests[0].scope.resource.user_id, &caller.user_id);

    assert_eq!(
        identity_store.deletes(),
        vec![(
            VENDOR.to_string(),
            caller.user_id.clone(),
            Some("install-alpha:".to_string())
        )]
    );
    assert_eq!(
        facade
            .caller_channel_connections(caller.clone())
            .await
            .expect("connection lookup after disconnect"),
        HashMap::from([(EXTENSION.to_string(), false)])
    );

    // Retry convergence for extension removal: `remove_extension` runs
    // the caller disconnect before `ExtensionRemove`, so a failed
    // removal retries the disconnect for a caller who is already
    // disconnected. That repeat disconnect must stay an idempotent
    // no-op success, not an error that would wedge the removal retry.
    facade
        .disconnect_channel_for_caller(caller.clone(), EXTENSION)
        .await
        .expect("repeat disconnect for a disconnected caller is an idempotent no-op");
    assert_eq!(
        credential_cleanup.requests().len(),
        2,
        "the removal-retry repeat disconnect re-issues the (idempotent) credential cleanup"
    );
}

#[tokio::test]
async fn facade_fails_closed_when_conversation_cleanup_is_unavailable() {
    let identity_store = bound_identity_store("install-alpha");
    let faulted = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::ReadFile)
                .path("/conversations/state.json")
                .backend("conversation storage unavailable"),
        ),
    );
    let workflow_state = Arc::new(ChannelWorkflowStateService::new(
        faulted as Arc<dyn RootFilesystem>,
    ));
    let facade = GenericChannelConnectionFacade::new(
        tenant(),
        vec![ChannelConnectionEntry {
            extension_id: EXTENSION.to_string(),
            providers: vec![VENDOR.to_string()],
            scope_source: Arc::new(StaticScopeSource(Some(scope("install-alpha")))),
        }],
        None,
        identity_store.clone(),
        identity_store.clone(),
        None,
        None,
        None,
        workflow_state,
        None,
    );
    let caller = caller();

    facade
        .disconnect_channel_for_caller(caller.clone(), EXTENSION)
        .await
        .expect_err("unavailable conversation cleanup must fail disconnect");

    assert_eq!(
        facade
            .caller_channel_connections(caller)
            .await
            .expect("identity remains readable"),
        HashMap::from([(EXTENSION.to_string(), true)]),
        "conversation cleanup must succeed before identity deletion commits disconnect"
    );
    assert_eq!(identity_store.deletes(), Vec::new());
}

#[tokio::test]
async fn facade_keeps_identity_when_credential_cleanup_fails() {
    let identity_store = bound_identity_store("install-alpha");
    let facade = facade(
        Some(scope("install-alpha")),
        identity_store.clone(),
        Some(Arc::new(FailingCredentialCleanup)),
    );
    let caller = caller();

    assert!(
        facade
            .disconnect_channel_for_caller(caller.clone(), EXTENSION)
            .await
            .is_err(),
        "credential cleanup failure must fail the disconnect"
    );
    assert_eq!(
        facade
            .caller_channel_connections(caller)
            .await
            .expect("connection lookup after failed disconnect"),
        HashMap::from([(EXTENSION.to_string(), true)]),
        "identity binding must remain until credential cleanup succeeds, so the removal retry re-runs the full disconnect"
    );
    assert_eq!(identity_store.deletes(), Vec::new());
}

#[tokio::test]
async fn facade_requires_current_installation_scope_for_connected() {
    // A binding under a different installation than the current scope
    // must not report connected.
    let identity_store = bound_identity_store("install-beta");
    let facade = facade(Some(scope("install-alpha")), identity_store, None);

    assert_eq!(
        facade
            .caller_channel_connections(caller())
            .await
            .expect("connection lookup"),
        HashMap::from([(EXTENSION.to_string(), false)])
    );
}

#[tokio::test]
async fn facade_disconnects_without_a_connection_scope() {
    // A fresh instance (or one whose setup was deleted) has no
    // connection scope. Uninstall/disconnect must still succeed and
    // clean the caller's own bindings without an installation prefix
    // while staying caller-bound.
    let identity_store = bound_identity_store("install-alpha");
    let credential_cleanup = Arc::new(RecordingCredentialCleanup::default());
    let facade = facade(
        None,
        identity_store.clone(),
        Some(credential_cleanup.clone()),
    );
    let caller = caller();

    assert_eq!(
        facade
            .caller_channel_connections(caller.clone())
            .await
            .expect("connection lookup"),
        HashMap::from([(EXTENSION.to_string(), false)])
    );
    facade
        .disconnect_channel_for_caller(caller.clone(), EXTENSION)
        .await
        .expect("disconnect succeeds without a connection scope");
    assert_eq!(
        credential_cleanup.requests().len(),
        1,
        "no-scope disconnect must still revoke the caller's credential"
    );
    assert_eq!(
        identity_store.deletes(),
        vec![(VENDOR.to_string(), caller.user_id, None)],
        "caller's bindings are cleaned without an installation prefix"
    );
}

#[tokio::test]
async fn facade_removes_user_dm_target_after_admin_scope_disappears() {
    let identity_store = bound_identity_store("install-alpha");
    let filesystem = Arc::new(InMemoryBackend::new());
    let dm_store = Arc::new(FilesystemChannelDmTargetStore::new(
        filesystem as Arc<dyn RootFilesystem>,
        tenant(),
        UserId::new("user:operator").expect("operator"),
    ));
    let caller = caller();
    dm_store
        .upsert(
            EXTENSION,
            &caller.user_id,
            "U123".to_string(),
            crate::extension_host::channel_dm_targets::dm_target_payload(Some("T123"), "DM-9"),
        )
        .await
        .expect("seed user DM target");
    let facade = GenericChannelConnectionFacade::new(
        tenant(),
        vec![ChannelConnectionEntry {
            extension_id: EXTENSION.to_string(),
            providers: vec![VENDOR.to_string()],
            scope_source: Arc::new(StaticScopeSource(None)),
        }],
        None,
        identity_store.clone(),
        identity_store,
        None,
        None,
        Some(Arc::clone(&dm_store)),
        workflow_state_service(),
        None,
    );

    facade
        .disconnect_channel_for_caller(caller.clone(), EXTENSION)
        .await
        .expect("disconnect without admin scope");

    assert!(
        dm_store
            .load(EXTENSION, &caller.user_id)
            .await
            .expect("load user DM target")
            .is_none(),
        "user-scoped cleanup must not depend on installation/admin scope"
    );
}

#[tokio::test]
async fn facade_ignores_foreign_tenants_and_unknown_channels() {
    let identity_store = bound_identity_store("install-alpha");
    let credential_cleanup = Arc::new(RecordingCredentialCleanup::default());
    let facade = facade(
        Some(scope("install-alpha")),
        identity_store.clone(),
        Some(credential_cleanup.clone()),
    );
    let foreign_caller = ProductSurfaceCaller::new(
        TenantId::new("tenant:other").expect("tenant"),
        UserId::new("user:alice").expect("user"),
        None::<AgentId>,
        None,
    );

    assert_eq!(
        facade
            .caller_channel_connections(foreign_caller.clone())
            .await
            .expect("connection lookup"),
        HashMap::from([(EXTENSION.to_string(), false)])
    );
    facade
        .disconnect_channel_for_caller(foreign_caller, EXTENSION)
        .await
        .expect("foreign tenant disconnect is a no-op");
    facade
        .disconnect_channel_for_caller(caller(), "unknown-channel")
        .await
        .expect("unknown channel disconnect is a no-op");
    assert!(credential_cleanup.requests().is_empty());
    assert_eq!(identity_store.deletes(), Vec::new());
}

/// The generic facade projects the caller's durable credential-account
/// status per vendor through the injected reader, so the extensions wire
/// can render `expired` instead of the connected/disconnected collapse.
/// A foreign tenant gets no account states (fail-closed).
#[tokio::test]
async fn facade_projects_caller_account_status_per_vendor() {
    let identity_store = bound_identity_store("install-alpha");
    let reader = Arc::new(RecordingAccountStatusReader::new(Some(
        CredentialAccountStatus::RefreshFailed,
    )));
    let facade = GenericChannelConnectionFacade::new(
        tenant(),
        vec![ChannelConnectionEntry {
            extension_id: EXTENSION.to_string(),
            providers: vec![VENDOR.to_string()],
            scope_source: Arc::new(StaticScopeSource(Some(scope("install-alpha")))),
        }],
        None,
        identity_store.clone(),
        identity_store,
        None,
        Some(reader.clone()),
        None,
        workflow_state_service(),
        None,
    );

    let states = facade
        .caller_channel_account_states(caller())
        .await
        .expect("account states");
    let state = states
        .get(EXTENSION)
        .expect("vendor account state projected");
    assert_eq!(
        state.account_status,
        Some(CredentialAccountStatus::RefreshFailed),
        "the caller's real durable status must reach the wire, not the connection bool",
    );
    assert_eq!(state.active_flow_status, None);
    assert_eq!(
        reader.calls(),
        vec![(caller().user_id, VENDOR.to_string())],
        "the reader was consulted for the caller + vendor",
    );

    let foreign = ProductSurfaceCaller::new(
        TenantId::new("tenant:other").expect("tenant"),
        UserId::new("user:alice").expect("user"),
        None::<AgentId>,
        None,
    );
    assert!(
        facade
            .caller_channel_account_states(foreign)
            .await
            .expect("account states")
            .is_empty(),
        "a foreign tenant gets no account states",
    );
}

#[tokio::test]
async fn facade_projects_authoritative_missing_account_instead_of_bool_fallback() {
    let identity_store = bound_identity_store("install-alpha");
    let reader = Arc::new(RecordingAccountStatusReader::new(None));
    let facade = GenericChannelConnectionFacade::new(
        tenant(),
        vec![ChannelConnectionEntry {
            extension_id: EXTENSION.to_string(),
            providers: vec![VENDOR.to_string()],
            scope_source: Arc::new(StaticScopeSource(Some(scope("install-alpha")))),
        }],
        None,
        identity_store.clone(),
        identity_store,
        None,
        Some(reader),
        None,
        workflow_state_service(),
        None,
    );

    let states = facade
        .caller_channel_account_states(caller())
        .await
        .expect("account states");
    let state = states
        .get(EXTENSION)
        .expect("authoritative missing account marker");

    assert_eq!(state.account_status, None);
    assert_eq!(state.active_flow_status, None);
}

struct RecordingAccountStatusReader {
    status: Option<CredentialAccountStatus>,
    calls: Mutex<Vec<(UserId, String)>>,
}

impl RecordingAccountStatusReader {
    fn new(status: Option<CredentialAccountStatus>) -> Self {
        Self {
            status,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(UserId, String)> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ChannelAccountStatusReader for RecordingAccountStatusReader {
    async fn account_status_for_caller(
        &self,
        caller: &ProductSurfaceCaller,
        provider: &str,
    ) -> Result<Option<CredentialAccountStatus>, ProductSurfaceError> {
        self.calls
            .lock()
            .expect("lock")
            .push((caller.user_id.clone(), provider.to_string()));
        Ok(self.status)
    }
}

/// Discovered-extension disconnect drops both the caller's provisioned
/// direct target and the durable conversation-owner route before the
/// identity binding commit point. The same provider actor can therefore
/// reconnect to a different Reborn user without inheriting the old thread.
#[tokio::test]
async fn discovered_extension_disconnect_drops_the_callers_dm_target() {
    use ironclaw_extensions::{
        ExtensionInstallation, ExtensionInstallationId, ExtensionManifestRecord,
        ExtensionManifestRef, ManifestSource,
    };

    const DISCOVERED_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acmechat"
name = "AcmeChat"
version = "0.1.0"
description = "discovered disconnect fixture"
trust = "first_party_requested"

[admin_configuration]
group_id = "extension.acmechat"
display_name = "AcmeChat deployment configuration"
fields = [
  { handle = "acmechat_webhook_secret", label = "Webhook secret", secret = true, required = false },
  { handle = "acmechat_team_id", label = "Workspace ID", secret = false, required = false },
  { handle = "acmechat_oauth_client_id", label = "OAuth client ID", secret = false, required = false },
]

[runtime]
kind = "first_party"
service = "acmechat.extension/v1"

[[tools]]
id = "acmechat.read_messages"
description = "Read AcmeChat messages"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acmechat/read_messages.input.v1.json"

[[tools.credentials]]
handle = "acmechat_user_token"
vendor = "acmechat"
scopes = ["messages.read"]
audience = { scheme = "https", host = "api.acmechat.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel]
id = "messages"
display_name = "AcmeChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "acmechat_webhook_secret"
header = "X-AcmeChat-Secret"

[channel.presentation]
supports_markdown = false
supports_threads = false

[auth.acmechat]
method = "oauth2_code"
display_name = "AcmeChat account"
authorization_endpoint = "https://auth.acmechat.example/authorize"
token_endpoint = "https://auth.acmechat.example/token"
scopes = ["messages.read"]
client_credentials = { client_id_handle = "acmechat_oauth_client_id" }

[auth.acmechat.token_response]
access_token = "/access_token"

[auth.acmechat.identity]
account_id = "/authed_user/id"
team_id = "/team/id"
"#;

    let installation_store =
        Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
    let record = ExtensionManifestRecord::from_toml(
        DISCOVERED_FIXTURE_MANIFEST,
        ManifestSource::HostBundled,
        &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
        None,
        &product_extension_host_api_contract_registry().expect("contracts"),
    )
    .expect("fixture manifest parses");
    let extension_id = ExtensionId::new(EXTENSION).expect("extension id");
    installation_store
        .upsert_manifest_and_installation(
            record,
            ExtensionInstallation::new(
                ExtensionInstallationId::new("install-alpha".to_string()).expect("installation id"),
                extension_id.clone(),
                ExtensionManifestRef::new(extension_id.clone(), None),
                Vec::new(),
                chrono::Utc::now(),
                ironclaw_extensions::InstallationOwner::user(caller().user_id),
            )
            .expect("installation"),
        )
        .await
        .expect("persist install");
    let identity_store = bound_identity_store("install-alpha");
    let extension_filesystem = Arc::new(InMemoryBackend::new());
    let dm_store = Arc::new(FilesystemChannelDmTargetStore::new(
        Arc::clone(&extension_filesystem) as Arc<dyn ironclaw_filesystem::RootFilesystem>,
        tenant(),
        UserId::new("user:operator").expect("user"),
    ));
    let workflow_filesystem: Arc<dyn RootFilesystem> = extension_filesystem.clone();
    let workflow_state = Arc::new(ChannelWorkflowStateService::new(workflow_filesystem));
    let caller = caller();
    dm_store
        .upsert(
            EXTENSION,
            &caller.user_id,
            "U123".to_string(),
            crate::extension_host::channel_dm_targets::dm_target_payload(Some("T123"), "DM-9"),
        )
        .await
        .expect("seed DM target");
    let workflow_extension = ExtensionId::new(EXTENSION).expect("workflow extension");
    let workflow = workflow_state
        .build_for_extension(
            &workflow_extension,
            ResourceScope {
                tenant_id: tenant(),
                user_id: caller.user_id.clone(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
        )
        .await
        .expect("workflow state");
    let adapter_kind = ironclaw_conversations::AdapterKind::new(EXTENSION).expect("adapter kind");
    let conversation_installation =
        ironclaw_conversations::AdapterInstallationId::new("install-alpha")
            .expect("conversation installation");
    let external_actor =
        ironclaw_conversations::ExternalActorRef::new("user", "U123").expect("external actor");
    let external_conversation =
        ironclaw_conversations::ExternalConversationRef::new(Some("T123"), "DM-9", None, None)
            .expect("external conversation");
    workflow
        .conversations
        .pair_external_actor(
            tenant(),
            adapter_kind.clone(),
            conversation_installation.clone(),
            external_actor.clone(),
            caller.user_id.clone(),
        )
        .await
        .expect("pair old user");
    ironclaw_conversations::ConversationBindingService::resolve_or_create_binding(
        workflow.conversations.as_ref(),
        ironclaw_conversations::ResolveConversationRequest {
            tenant_id: tenant(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: conversation_installation.clone(),
            external_actor_ref: external_actor.clone(),
            external_conversation_ref: external_conversation.clone(),
            external_event_id: ironclaw_conversations::ExternalEventId::new("event-before")
                .expect("event id"),
            route_kind: ironclaw_conversations::ConversationRouteKind::Direct,
            requested_agent_id: None,
            requested_project_id: None,
        },
    )
    .await
    .expect("old user direct route");
    let other_installation = ironclaw_conversations::AdapterInstallationId::new("install-beta")
        .expect("other conversation installation");
    let other_actor =
        ironclaw_conversations::ExternalActorRef::new("user", "U456").expect("other actor");
    let other_conversation =
        ironclaw_conversations::ExternalConversationRef::new(Some("T456"), "DM-10", None, None)
            .expect("other conversation");
    workflow
        .conversations
        .pair_external_actor(
            tenant(),
            adapter_kind.clone(),
            other_installation.clone(),
            other_actor.clone(),
            caller.user_id.clone(),
        )
        .await
        .expect("pair same user under another installation");
    ironclaw_conversations::ConversationBindingService::resolve_or_create_binding(
        workflow.conversations.as_ref(),
        ironclaw_conversations::ResolveConversationRequest {
            tenant_id: tenant(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: other_installation.clone(),
            external_actor_ref: other_actor.clone(),
            external_conversation_ref: other_conversation.clone(),
            external_event_id: ironclaw_conversations::ExternalEventId::new("event-other-before")
                .expect("event id"),
            route_kind: ironclaw_conversations::ConversationRouteKind::Direct,
            requested_agent_id: None,
            requested_project_id: None,
        },
    )
    .await
    .expect("other installation route");

    let facade = GenericChannelConnectionFacade::new(
        tenant(),
        Vec::new(),
        Some(installation_store as Arc<dyn ExtensionInstallationStore>),
        identity_store.clone(),
        identity_store.clone(),
        None,
        None,
        Some(Arc::clone(&dm_store)),
        Arc::clone(&workflow_state),
        None,
    );

    // Discovered + bound: connected.
    assert_eq!(
        facade
            .caller_channel_connections(caller.clone())
            .await
            .expect("connection lookup"),
        HashMap::from([(EXTENSION.to_string(), true)])
    );

    facade
        .disconnect_channel_for_caller(caller.clone(), EXTENSION)
        .await
        .expect("disconnect succeeds");

    assert!(
        dm_store
            .load(EXTENSION, &caller.user_id)
            .await
            .expect("load")
            .is_none(),
        "disconnect must drop the caller's provisioned DM target"
    );
    assert_eq!(
        identity_store.deletes(),
        vec![(
            VENDOR.to_string(),
            caller.user_id.clone(),
            Some("install-alpha:".to_string())
        )],
        "bindings delete last, prefix-scoped to the installation"
    );

    let workflow_after_disconnect = workflow_state
        .build_for_extension(
            &workflow_extension,
            ResourceScope {
                tenant_id: tenant(),
                user_id: caller.user_id.clone(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
        )
        .await
        .expect("reload durable workflow state after disconnect");
    let preserved = ironclaw_conversations::ConversationBindingService::resolve_or_create_binding(
        workflow_after_disconnect.conversations.as_ref(),
        ironclaw_conversations::ResolveConversationRequest {
            tenant_id: tenant(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: other_installation,
            external_actor_ref: other_actor,
            external_conversation_ref: other_conversation,
            external_event_id: ironclaw_conversations::ExternalEventId::new("event-other-after")
                .expect("event id"),
            route_kind: ironclaw_conversations::ConversationRouteKind::Direct,
            requested_agent_id: None,
            requested_project_id: None,
        },
    )
    .await
    .expect("other installation route remains");
    assert_eq!(
        preserved.actor.user_id, caller.user_id,
        "disconnect cleanup must remain narrowed to its installation"
    );

    let new_user = UserId::new("user:bob").expect("user");
    workflow_after_disconnect
        .conversations
        .pair_external_actor(
            tenant(),
            adapter_kind.clone(),
            conversation_installation.clone(),
            external_actor.clone(),
            new_user.clone(),
        )
        .await
        .expect("pair new user");
    let rebound = ironclaw_conversations::ConversationBindingService::resolve_or_create_binding(
        workflow.conversations.as_ref(),
        ironclaw_conversations::ResolveConversationRequest {
            tenant_id: tenant(),
            adapter_kind,
            adapter_installation_id: conversation_installation,
            external_actor_ref: external_actor,
            external_conversation_ref: external_conversation,
            external_event_id: ironclaw_conversations::ExternalEventId::new("event-after")
                .expect("event id"),
            route_kind: ironclaw_conversations::ConversationRouteKind::Direct,
            requested_agent_id: None,
            requested_project_id: None,
        },
    )
    .await
    .expect("new user can claim a fresh direct route");
    assert_eq!(rebound.actor.user_id, new_user);
}

/// Proof-code channels have no auth vendor, but the connection facade
/// still has to discover them so its pairing registry can own status and
/// disconnect. This mirrors Telegram's manifest shape without naming a
/// provider in production code.
#[tokio::test]
async fn connection_discovery_includes_channel_without_auth_vendor() {
    use ironclaw_extensions::{
        ExtensionInstallation, ExtensionInstallationId, ExtensionManifestRecord,
        ExtensionManifestRef, ManifestSource,
    };

    const PAIRING_CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "pairchat"
name = "PairChat"
version = "0.1.0"
description = "proof-code channel discovery fixture"
trust = "first_party_requested"

[admin_configuration]
group_id = "extension.pairchat"
display_name = "PairChat deployment configuration"
fields = [
  { handle = "pairchat_bot_token", label = "Bot token", secret = true, required = false },
  { handle = "pairchat_webhook_secret", label = "Webhook secret", secret = true, required = false },
]

[runtime]
kind = "first_party"
service = "pairchat.extension/v1"

[channel]
id = "messages"
display_name = "PairChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "updates"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "pairchat_webhook_secret"
header = "X-PairChat-Secret"

[[channel.egress]]
scheme = "https"
host = "api.pairchat.example"
methods = ["post"]
credential_handle = "pairchat_bot_token"
injection = { type = "header", name = "authorization", prefix = "Bearer " }
"#;

    let installation_store =
        Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
    let record = ExtensionManifestRecord::from_toml(
        PAIRING_CHANNEL_MANIFEST,
        ManifestSource::HostBundled,
        &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
        None,
        &product_extension_host_api_contract_registry().expect("contracts"),
    )
    .expect("pairing channel manifest parses");
    let extension_id = ExtensionId::new("pairchat").expect("extension id");
    installation_store
        .upsert_manifest_and_installation(
            record,
            ExtensionInstallation::new(
                ExtensionInstallationId::new("pairchat-install").expect("installation id"),
                extension_id.clone(),
                ExtensionManifestRef::new(extension_id, None),
                Vec::new(),
                chrono::Utc::now(),
                ironclaw_extensions::InstallationOwner::user(caller().user_id),
            )
            .expect("installation"),
        )
        .await
        .expect("persist install");

    let identity_store = bound_identity_store("pairchat-install");
    let facade = GenericChannelConnectionFacade::new(
        tenant(),
        Vec::new(),
        Some(installation_store as Arc<dyn ExtensionInstallationStore>),
        identity_store.clone(),
        identity_store,
        None,
        None,
        None,
        workflow_state_service(),
        None,
    );

    let entries = facade
        .connection_entries()
        .await
        .expect("discover channels");
    let pairchat = entries
        .iter()
        .find(|entry| entry.extension_id == "pairchat")
        .expect("channel-only extension is discoverable");
    assert!(
        pairchat.providers.is_empty(),
        "proof-code pairing does not invent an OAuth vendor"
    );
}

struct StaticScopeSource(Option<ChannelConnectionScope>);

#[async_trait]
impl ChannelConnectionScopeSource for StaticScopeSource {
    async fn resolve_connection_scope(&self) -> Result<Option<ChannelConnectionScope>, String> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct RecordingCredentialCleanup {
    requests: Mutex<Vec<SecretCleanupRequest>>,
}

impl RecordingCredentialCleanup {
    fn requests(&self) -> Vec<SecretCleanupRequest> {
        self.requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ChannelCredentialCleanup for RecordingCredentialCleanup {
    async fn cleanup_credentials_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError> {
        self.requests.lock().expect("lock").push(request);
        Ok(SecretCleanupReport::default())
    }
}

struct FailingCredentialCleanup;

#[async_trait]
impl ChannelCredentialCleanup for FailingCredentialCleanup {
    async fn cleanup_credentials_for_lifecycle(
        &self,
        _request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError> {
        Err(ProductSurfaceError::internal_from(
            "credential cleanup unavailable",
        ))
    }
}

#[derive(Default)]
struct RecordingIdentityStore {
    bindings: Mutex<HashMap<String, UserId>>,
    deletes: Mutex<Vec<(String, UserId, Option<String>)>>,
}

impl RecordingIdentityStore {
    fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
        Self {
            bindings: Mutex::new(bindings.into_iter().collect()),
            deletes: Mutex::new(Vec::new()),
        }
    }

    fn deletes(&self) -> Vec<(String, UserId, Option<String>)> {
        self.deletes.lock().expect("lock").clone()
    }
}

#[async_trait]
impl RebornUserIdentityLookup for RecordingIdentityStore {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        if provider != VENDOR {
            return Ok(None);
        }
        Ok(self
            .bindings
            .lock()
            .expect("lock")
            .get(provider_user_id)
            .cloned())
    }

    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        self.user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
            .await
    }

    async fn user_has_provider_binding_with_provider_user_id_prefix(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        if provider != VENDOR {
            return Ok(false);
        }
        Ok(self
            .bindings
            .lock()
            .expect("lock")
            .iter()
            .any(|(provider_user_id, bound_user_id)| {
                bound_user_id == user_id
                    && provider_user_id_prefix
                        .map(|prefix| provider_user_id.starts_with(prefix))
                        .unwrap_or(true)
            }))
    }
}

#[async_trait]
impl RebornUserIdentityBindingDeleteStore for RecordingIdentityStore {
    async fn delete_user_identity_bindings_for_user(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<usize, RebornUserIdentityBindingError> {
        self.deletes.lock().expect("lock").push((
            provider.to_string(),
            user_id.clone(),
            provider_user_id_prefix.map(ToString::to_string),
        ));
        let mut bindings = self.bindings.lock().expect("lock");
        let before = bindings.len();
        bindings.retain(|provider_user_id, bound_user_id| {
            let prefix_matches = provider_user_id_prefix
                .map(|prefix| provider_user_id.starts_with(prefix))
                .unwrap_or(true);
            !(bound_user_id == user_id && prefix_matches)
        });
        Ok(before - bindings.len())
    }
}
