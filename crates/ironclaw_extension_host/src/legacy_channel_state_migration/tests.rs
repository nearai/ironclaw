use super::*;
use super::{oauth_channel::*, proof_code_channel::*};
use ironclaw_extensions::{
    AdminConfigurationField, ExtensionAdminConfigurationDescriptor, ExtensionInstallation,
    ExtensionInstallationId, ExtensionInstallationStore, ExtensionManifestRecord,
    ExtensionManifestRef, InstallationOwner, ManifestHash, ManifestSource, PackageRootBinding,
};
use ironclaw_filesystem::{
    CasExpectation, Entry, Fault, FaultInjecting, FilesystemOperation, InMemoryBackend,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    mount::{MountGrant, MountPermissions, MountView},
    path::MountAlias,
};
use ironclaw_secrets::{SecretStore, SecretStorePort};

fn slack_manifest_record() -> ExtensionManifestRecord {
    let raw = r#"
schema_version = "reborn.extension_manifest.v3"
id = "slack"
name = "Slack fixture"
version = "0.1.0"
description = "rc1 channel-state migration fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "slack.fixture/v1"
"#;
    let hash = ManifestHash::new(sha256_digest_token(raw.as_bytes())).expect("manifest hash");
    ExtensionManifestRecord::from_toml_with_root_binding(
        raw,
        ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
        Some(hash),
        &crate::product_extension_host_api_contract_registry().expect("contracts"),
        PackageRootBinding::Virtual,
    )
    .expect("Slack fixture manifest")
}

fn telegram_manifest_record() -> ExtensionManifestRecord {
    let raw = r#"
schema_version = "reborn.extension_manifest.v3"
id = "telegram"
name = "Telegram fixture"
version = "0.1.0"
description = "rc1 channel-state migration fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "telegram.fixture/v1"
"#;
    let hash = ManifestHash::new(sha256_digest_token(raw.as_bytes())).expect("manifest hash");
    ExtensionManifestRecord::from_toml_with_root_binding(
        raw,
        ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
        Some(hash),
        &crate::product_extension_host_api_contract_registry().expect("contracts"),
        PackageRootBinding::Virtual,
    )
    .expect("Telegram fixture manifest")
}

fn slack_admin_descriptor() -> ExtensionAdminConfigurationDescriptor {
    let fields = [
        ("slack_bot_token", true, true),
        ("slack_signing_secret", true, true),
        ("slack_team_id", false, true),
        ("slack_api_app_id", false, true),
        ("slack_installation_id", false, true),
        ("slack_bot_user_id", false, true),
        ("slack_shared_subject_user_id", false, false),
        ("slack_oauth_client_id", false, true),
        ("slack_oauth_client_secret", true, true),
        ("slack_allowed_channels", false, false),
        ("slack_subject_routes", false, false),
    ]
    .into_iter()
    .map(|(handle, secret, required)| AdminConfigurationField {
        handle: SecretHandle::new(handle).expect("fixture handle"),
        label: handle.to_string(),
        secret,
        required,
    })
    .collect();
    ExtensionAdminConfigurationDescriptor {
        group_id: AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
        display_name: "Slack fixture".to_string(),
        description: "rc1 migration fixture".to_string(),
        fields,
    }
}

fn telegram_admin_descriptor() -> ExtensionAdminConfigurationDescriptor {
    let fields = [
        ("telegram_bot_token", true, true),
        ("telegram_webhook_secret", true, true),
        ("telegram_webhook_url", false, true),
        ("bot_username", false, true),
    ]
    .into_iter()
    .map(|(handle, secret, required)| AdminConfigurationField {
        handle: SecretHandle::new(handle).expect("fixture handle"),
        label: handle.to_string(),
        secret,
        required,
    })
    .collect();
    ExtensionAdminConfigurationDescriptor {
        group_id: AdminConfigurationGroupId::new(TELEGRAM_GROUP).expect("group"),
        display_name: "Telegram fixture".to_string(),
        description: "rc1 migration fixture".to_string(),
        fields,
    }
}

fn fixed_admin_mount() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/extension-admin-configuration").expect("alias"),
        VirtualPath::new("/tenants/tenant-a/shared/admin-configuration").expect("target root"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("admin mount")
}

#[test]
fn frozen_rc1_channel_wires_remain_readable_and_strict() {
    let slack_setup = r#"{
        "installation_id":"slack-old-install",
        "team_id":"T1",
        "api_app_id":"A1",
        "user_id":"U-BOT",
        "shared_subject_user_id":"operator",
        "bot_token_handle":"slack-bot-r1",
        "signing_secret_handle":"slack-signing-r1",
        "oauth_client_id":"oauth-client-1",
        "oauth_client_secret_handle":"slack-oauth-r1",
        "revision":1,
        "updated_at":"2026-07-01T00:00:00Z"
    }"#;
    let setup: Rc1SlackSetup = serde_json::from_str(slack_setup).expect("exact rc1 Slack wire");
    assert_eq!(setup.installation_id, "slack-old-install");
    assert_eq!(setup.oauth_client_id.as_deref(), Some("oauth-client-1"));
    assert_eq!(
        setup
            .oauth_client_secret_handle
            .as_ref()
            .map(SecretHandle::as_str),
        Some("slack-oauth-r1")
    );

    let telegram_setup = r#"{
        "bot_id":4242,
        "bot_username":"ironclaw_fixture_bot",
        "webhook_url":"https://example.invalid/telegram",
        "bot_token_handle":"telegram-bot-r2",
        "webhook_secret_handle":"telegram-webhook-r2",
        "revision":2,
        "updated_at":"2026-07-01T00:00:00Z"
    }"#;
    assert!(matches!(
        serde_json::from_str::<Rc1TelegramSetup>(telegram_setup).expect("exact rc1 Telegram wire"),
        Rc1TelegramSetup::Active(_)
    ));

    let malformed =
        slack_setup.replace("\"revision\":1", "\"revision\":1,\"unknown_p0_field\":true");
    assert!(
        serde_json::from_str::<Rc1SlackSetup>(&malformed).is_err(),
        "unknown released-state fields must fail closed"
    );
}

#[test]
fn identity_prefix_rewrite_is_exact_and_requires_the_target_installation() {
    assert_eq!(
        rewrite_installation_prefix(
            "slack-old-install:U123",
            Some("slack-old-install"),
            Some("slack-new-install"),
        )
        .unwrap(),
        "slack-new-install:U123"
    );
    assert!(matches!(
        rewrite_installation_prefix(
            "slack-old-install-extra:U123",
            Some("slack-old-install"),
            Some("slack-new-install"),
        ),
        Err(Rc1ChannelStateMigrationError::Malformed)
    ));
    assert!(matches!(
        rewrite_installation_prefix("slack-old-install:U123", Some("slack-old-install"), None,),
        Err(Rc1ChannelStateMigrationError::MissingInstallation)
    ));
}

#[tokio::test]
async fn caller_imports_rc1_slack_setup_and_secrets_idempotently() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem: Arc<dyn RootFilesystem> = backend.clone();
    let secret_store = Arc::new(SecretStore::ephemeral_over(Arc::clone(&backend)));
    let secret_store_port: Arc<dyn SecretStorePort> = secret_store.clone();
    let admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
        ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
    );
    let admin_configuration = Arc::new(
        AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
            crate::FilesystemAdminConfigurationStore::new(admin_filesystem),
            Arc::clone(&secret_store_port),
            [slack_admin_descriptor()],
        )
        .expect("admin configuration"),
    );
    let installation_store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        ExtensionInstallationStore::default_state_path().expect("state path"),
        ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
        crate::product_extension_host_api_contract_registry().expect("contracts"),
    )
    .await
    .expect("installation store");
    let manifest = slack_manifest_record();
    let extension_id = ironclaw_host_api::ids::ExtensionId::new("slack").expect("extension");
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new("slack-target").expect("installation"),
        extension_id.clone(),
        ExtensionManifestRef::new(extension_id, manifest.manifest_hash().cloned()),
        Vec::new(),
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .expect("installation");
    installation_store
        .upsert_manifest_and_installation(manifest, installation)
        .await
        .expect("seed target installation");
    let installation_store: Arc<dyn ExtensionInstallationStorePort> = Arc::new(installation_store);
    let slack_scope = ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("operator-a").expect("operator"),
        agent_id: Some(AgentId::new("agent-a").expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let admin_scope = ResourceScope {
        tenant_id: slack_scope.tenant_id.clone(),
        user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    for (handle, material) in [
        ("rc1-bot-token", "xoxb-rc1"),
        ("rc1-signing", "signing-rc1"),
        ("rc1-oauth", "oauth-rc1"),
    ] {
        secret_store
            .put(
                slack_scope.clone(),
                SecretHandle::new(handle).expect("handle"),
                SecretMaterial::from(material.to_string()),
                None,
            )
            .await
            .expect("seed source secret");
    }
    let setup = serde_json::json!({
        "installation_id": "slack-rc1-install",
        "team_id": "T-RC1",
        "api_app_id": "A-RC1",
        "user_id": "U-BOT-RC1",
        "shared_subject_user_id": "operator-a",
        "bot_token_handle": "rc1-bot-token",
        "signing_secret_handle": "rc1-signing",
        "oauth_client_id": "client-rc1",
        "oauth_client_secret_handle": "rc1-oauth",
        "revision": 7,
        "updated_at": "2026-07-01T00:00:00Z"
    });
    filesystem
        .put(
            &VirtualPath::new("/tenants/tenant-a/shared/slack-setup/installation.json")
                .expect("setup path"),
            Entry::bytes(serde_json::to_vec(&setup).expect("setup wire")),
            CasExpectation::Absent,
        )
        .await
        .expect("seed setup");
    for (path, value) in [
        (
            "/tenants/tenant-a/shared/slack-personal-binding/identities/U-PERSON.json",
            serde_json::json!({
                "provider": "slack",
                "provider_user_id": "slack-rc1-install:U-PERSON",
                "user_id": "operator-a",
                "state": "active",
                "created_at": "2026-07-01T00:00:00Z",
                "updated_at": "2026-07-01T00:00:00Z"
            }),
        ),
        (
            "/tenants/tenant-a/shared/slack-channel-routes/C-ALLOWED.json",
            serde_json::json!({
                "tenant_id": "tenant-a",
                "installation_id": "slack-rc1-install",
                "team_id": "T-RC1",
                "channel_id": "C-ALLOWED",
                "subject_user_id": "user:slack-channel:C-ALLOWED",
                "updated_at": "2026-07-01T00:00:00Z"
            }),
        ),
        (
            "/tenants/tenant-a/shared/slack-channel-routes/C-EXPLICIT.json",
            serde_json::json!({
                "tenant_id": "tenant-a",
                "installation_id": "slack-rc1-install",
                "team_id": "T-RC1",
                "channel_id": "C-EXPLICIT",
                "subject_user_id": "operator-a",
                "updated_at": "2026-07-01T00:00:00Z"
            }),
        ),
        (
            "/tenants/tenant-a/shared/slack-personal-binding/dm-targets/operator-a.json",
            serde_json::json!({
                "tenant_id": "tenant-a",
                "installation_id": "slack-rc1-install",
                "team_id": "T-RC1",
                "user_id": "operator-a",
                "slack_user_id": "U-PERSON",
                "dm_channel_id": "D-RC1",
                "created_at": "2026-07-01T00:00:00Z",
                "updated_at": "2026-07-01T00:00:00Z"
            }),
        ),
    ] {
        filesystem
            .put(
                &VirtualPath::new(path).expect("legacy state path"),
                Entry::bytes(serde_json::to_vec(&value).expect("legacy state wire")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed legacy state");
    }

    let inputs = Rc1ChannelStateMigrationInputs {
        filesystem: Arc::clone(&filesystem),
        installation_store,
        secret_store: secret_store_port,
        admin_configuration: Arc::clone(&admin_configuration),
        oauth_channel_secret_scope: slack_scope,
        proof_code_channel_secret_scope: admin_scope.clone(),
        admin_scope: admin_scope.clone(),
        identity_store: Arc::new(FilesystemChannelIdentityStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        )),
        dm_targets: Arc::new(FilesystemChannelDmTargetStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        )),
    };
    let first = migrate_rc1_channel_state(&inputs)
        .await
        .expect("migrate exact setup");
    assert_eq!(first.configuration_values, 9);
    assert_eq!(first.identities, 1);
    assert_eq!(first.route_values, 2);
    assert_eq!(first.dm_targets, 1);
    let state = admin_configuration
        .get(
            &admin_scope,
            &AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
        )
        .await
        .expect("read imported setup");
    assert!(state.complete);
    assert_eq!(
        state
            .fields
            .iter()
            .find(|field| field.handle.as_str() == "slack_team_id")
            .and_then(|field| field.value.as_deref()),
        Some("T-RC1")
    );
    let target_secrets = secret_store
        .metadata_for_scope(&admin_scope.tenant_shared_managed_scope())
        .await
        .expect("list migrated secrets");
    assert_eq!(target_secrets.len(), 3);
    let binding = inputs
        .identity_store
        .resolve_user_identity("slack", "slack-target:U-PERSON")
        .await
        .expect("identity lookup")
        .expect("identity migrated");
    assert_eq!(binding.as_str(), "operator-a");
    let dm = inputs
        .dm_targets
        .load(SLACK, &UserId::new("operator-a").expect("operator"))
        .await
        .expect("DM lookup")
        .expect("DM target migrated");
    assert_eq!(dm.external_actor_id, "U-PERSON");
    assert_eq!(dm.target["conversation_id"], "D-RC1");

    // Reconstruct every 1.1 reader over the durable backend. This models
    // the next process start rather than proving only that the migration
    // service's in-memory objects can still see their own writes.
    let restarted_admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
        ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
    );
    let restarted_admin =
        AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
            crate::FilesystemAdminConfigurationStore::new(restarted_admin_filesystem),
            Arc::clone(&inputs.secret_store),
            [slack_admin_descriptor()],
        )
        .expect("reopen admin configuration");
    let restarted_state = restarted_admin
        .get(
            &admin_scope,
            &AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
        )
        .await
        .expect("read setup after restart");
    assert!(restarted_state.complete);
    for (handle, expected) in [
        ("slack_bot_token", "xoxb-rc1"),
        ("slack_signing_secret", "signing-rc1"),
        ("slack_oauth_client_secret", "oauth-rc1"),
    ] {
        let material = restarted_admin
            .secret_material(
                &admin_scope,
                &AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
                &SecretHandle::new(handle).expect("handle"),
            )
            .await
            .expect("consume migrated secret after restart")
            .expect("migrated secret exists");
        assert_eq!(material.expose_secret(), expected);
    }
    let restarted_identities = FilesystemChannelIdentityStore::new(
        Arc::clone(&filesystem),
        admin_scope.tenant_id.clone(),
        admin_scope.user_id.clone(),
    );
    assert_eq!(
        restarted_identities
            .resolve_user_identity("slack", "slack-target:U-PERSON")
            .await
            .expect("identity lookup after restart")
            .expect("identity retained after restart")
            .as_str(),
        "operator-a"
    );
    let restarted_dm_targets = FilesystemChannelDmTargetStore::new(
        Arc::clone(&filesystem),
        admin_scope.tenant_id.clone(),
        admin_scope.user_id.clone(),
    );
    let restarted_dm = restarted_dm_targets
        .load(SLACK, &UserId::new("operator-a").expect("operator"))
        .await
        .expect("DM lookup after restart")
        .expect("DM target retained after restart");
    assert_eq!(restarted_dm.external_actor_id, "U-PERSON");
    assert_eq!(restarted_dm.target["conversation_id"], "D-RC1");

    let second = migrate_rc1_channel_state(&inputs)
        .await
        .expect("second pass revalidates");
    assert_eq!(second.configuration_values, 0);
    assert_eq!(second.identities, 0);
    assert_eq!(second.route_values, 0);
    assert_eq!(second.dm_targets, 0);
}

#[tokio::test]
async fn caller_imports_rc1_telegram_setup_and_reopens_every_usable_state() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem: Arc<dyn RootFilesystem> = backend.clone();
    let secret_store = Arc::new(SecretStore::ephemeral_over(Arc::clone(&backend)));
    let secret_store_port: Arc<dyn SecretStorePort> = secret_store.clone();
    let admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
        ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
    );
    let admin_configuration = Arc::new(
        AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
            crate::FilesystemAdminConfigurationStore::new(admin_filesystem),
            Arc::clone(&secret_store_port),
            [telegram_admin_descriptor()],
        )
        .expect("admin configuration"),
    );
    let installation_store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        ExtensionInstallationStore::default_state_path().expect("state path"),
        ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
        crate::product_extension_host_api_contract_registry().expect("contracts"),
    )
    .await
    .expect("installation store");
    let manifest = telegram_manifest_record();
    let extension_id = ironclaw_host_api::ids::ExtensionId::new(TELEGRAM).expect("extension");
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new("telegram-target").expect("installation"),
        extension_id.clone(),
        ExtensionManifestRef::new(extension_id, manifest.manifest_hash().cloned()),
        Vec::new(),
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .expect("installation");
    installation_store
        .upsert_manifest_and_installation(manifest, installation)
        .await
        .expect("seed target installation");
    let installation_store: Arc<dyn ExtensionInstallationStorePort> = Arc::new(installation_store);
    let telegram_scope = ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("operator-a").expect("operator"),
        agent_id: Some(AgentId::new("agent-a").expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let admin_scope = ResourceScope {
        tenant_id: telegram_scope.tenant_id.clone(),
        user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    for (handle, material) in [
        ("rc1-telegram-bot", "telegram-token-rc1"),
        ("rc1-telegram-webhook", "telegram-webhook-rc1"),
    ] {
        secret_store
            .put(
                telegram_scope.clone(),
                SecretHandle::new(handle).expect("handle"),
                SecretMaterial::from(material.to_string()),
                None,
            )
            .await
            .expect("seed source secret");
    }
    let legacy_rows = [
        (
            "/tenants/tenant-a/shared/telegram-setup/installation.json",
            serde_json::json!({
                "bot_id": 4242,
                "bot_username": "ironclaw_fixture_bot",
                "webhook_url": "https://example.invalid/telegram",
                "bot_token_handle": "rc1-telegram-bot",
                "webhook_secret_handle": "rc1-telegram-webhook",
                "revision": 2,
                "updated_at": "2026-07-01T00:00:00Z"
            }),
        ),
        (
            "/tenants/tenant-a/shared/telegram-binding/identities/9001.json",
            serde_json::json!({
                "provider_user_id": "tg-bot-4242:9001",
                "user_id": "operator-a",
                "epoch": "epoch-rc1",
                "active": true
            }),
        ),
        (
            "/tenants/tenant-a/shared/telegram-dm-targets/operator-a.json",
            serde_json::json!({
                "user_id": "operator-a",
                "chat_id": 12345
            }),
        ),
        (
            "/tenants/tenant-a/shared/telegram-dm-targets/stale-operator.json",
            serde_json::json!({
                "user_id": "stale-operator",
                "chat_id": 54321
            }),
        ),
        (
            "/tenants/tenant-a/shared/telegram-pairing/codes/ABCDEFGH.json",
            serde_json::json!({
                "code": "ABCDEFGH",
                "tenant_id": "tenant-a",
                "user_id": "operator-a",
                "installation_id": "tg-bot-4242",
                "created_at": "2026-07-01T00:00:00Z",
                "expires_at": "2026-07-01T00:15:00Z",
                "consumed_at": null
            }),
        ),
        (
            "/tenants/tenant-a/shared/telegram-pairing/users/operator-a.json",
            serde_json::json!({
                "code": "ABCDEFGH",
                "active": true
            }),
        ),
        (
            "/tenants/tenant-a/shared/telegram-pairing/pending-completions/operator-a.json",
            serde_json::json!({
                "installation_id": "tg-bot-4242",
                "user_id": "operator-a",
                "chat_id": 12345,
                "completed": false
            }),
        ),
    ];
    for (path, value) in legacy_rows {
        filesystem
            .put(
                &VirtualPath::new(path).expect("legacy state path"),
                Entry::bytes(serde_json::to_vec(&value).expect("legacy state wire")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed legacy state");
    }

    let inputs = Rc1ChannelStateMigrationInputs {
        filesystem: Arc::clone(&filesystem),
        installation_store,
        secret_store: secret_store_port,
        admin_configuration,
        oauth_channel_secret_scope: admin_scope.clone(),
        proof_code_channel_secret_scope: telegram_scope,
        admin_scope: admin_scope.clone(),
        identity_store: Arc::new(FilesystemChannelIdentityStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        )),
        dm_targets: Arc::new(FilesystemChannelDmTargetStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        )),
    };
    let first = migrate_rc1_channel_state(&inputs)
        .await
        .expect("migrate exact Telegram setup");
    assert_eq!(first.configuration_values, 4);
    assert_eq!(first.identities, 1);
    assert_eq!(first.dm_targets, 1);
    assert_eq!(first.unbound_dm_targets_skipped, 1);
    assert_eq!(first.proof_code_pairing_challenges_expired, 1);
    assert_eq!(first.proof_code_pending_completions_expired, 1);

    let restarted_admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
        ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
    );
    let restarted_admin =
        AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
            crate::FilesystemAdminConfigurationStore::new(restarted_admin_filesystem),
            Arc::clone(&inputs.secret_store),
            [telegram_admin_descriptor()],
        )
        .expect("reopen admin configuration");
    let group = AdminConfigurationGroupId::new(TELEGRAM_GROUP).expect("group");
    let state = restarted_admin
        .get(&admin_scope, &group)
        .await
        .expect("read Telegram setup after restart");
    assert!(state.complete);
    for (handle, expected) in [
        ("telegram_bot_token", "telegram-token-rc1"),
        ("telegram_webhook_secret", "telegram-webhook-rc1"),
    ] {
        let material = restarted_admin
            .secret_material(
                &admin_scope,
                &group,
                &SecretHandle::new(handle).expect("handle"),
            )
            .await
            .expect("consume migrated secret after restart")
            .expect("migrated secret exists");
        assert_eq!(material.expose_secret(), expected);
    }
    let restarted_identities = FilesystemChannelIdentityStore::new(
        Arc::clone(&filesystem),
        admin_scope.tenant_id.clone(),
        admin_scope.user_id.clone(),
    );
    assert_eq!(
        restarted_identities
            .resolve_user_identity(TELEGRAM, "telegram-target:9001")
            .await
            .expect("identity lookup after restart")
            .expect("identity retained after restart")
            .as_str(),
        "operator-a"
    );
    let restarted_dm_targets = FilesystemChannelDmTargetStore::new(
        Arc::clone(&filesystem),
        admin_scope.tenant_id.clone(),
        admin_scope.user_id.clone(),
    );
    let dm = restarted_dm_targets
        .load(TELEGRAM, &UserId::new("operator-a").expect("operator"))
        .await
        .expect("DM lookup after restart")
        .expect("DM target retained after restart");
    assert_eq!(dm.external_actor_id, "9001");
    assert_eq!(dm.target["conversation_id"], "12345");

    let second = migrate_rc1_channel_state(&inputs)
        .await
        .expect("second pass revalidates");
    assert_eq!(second.configuration_values, 0);
    assert_eq!(second.identities, 0);
    assert_eq!(second.dm_targets, 0);
    assert_eq!(second.proof_code_pairing_rows_unchanged, 3);
}

#[tokio::test]
async fn scope_discovery_is_a_noop_on_an_empty_backend() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());

    let scopes = discover_rc1_channel_migration_scopes(filesystem)
        .await
        .expect("empty installation has no rc1 channel state");

    assert!(scopes.is_empty());
}

#[tokio::test]
async fn scope_discovery_finds_every_tenant_and_exact_secret_owner() {
    let backend = InMemoryBackend::new();
    for (tenant, user, agent) in [
        ("tenant-a", "operator-a", "agent-a"),
        ("tenant-b", "operator-b", "agent-b"),
    ] {
        let setup_path = VirtualPath::new(format!(
            "/tenants/{tenant}/shared/slack-setup/installation.json"
        ))
        .expect("setup path");
        let setup = serde_json::json!({
            "installation_id": format!("slack-{tenant}"),
            "team_id": format!("team-{tenant}"),
            "api_app_id": format!("app-{tenant}"),
            "user_id": format!("bot-{tenant}"),
            "shared_subject_user_id": user,
            "bot_token_handle": format!("bot-token-{tenant}"),
            "signing_secret_handle": format!("signing-{tenant}"),
            "revision": 1,
            "updated_at": "2026-07-01T00:00:00Z"
        });
        backend
            .put(
                &setup_path,
                Entry::bytes(serde_json::to_vec(&setup).expect("setup wire")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed setup");
        for handle in [format!("bot-token-{tenant}"), format!("signing-{tenant}")] {
            let secret_path = VirtualPath::new(format!(
                "/tenants/{tenant}/users/{user}/secrets/agents/{agent}/secrets/{handle}.json"
            ))
            .expect("secret path");
            backend
                .put(
                    &secret_path,
                    Entry::bytes(vec![1, 2, 3]),
                    CasExpectation::Absent,
                )
                .await
                .expect("seed secret authority");
        }
    }

    let filesystem: Arc<dyn RootFilesystem> = Arc::new(
        FaultInjecting::new(backend).with_fault(
            Fault::on(FilesystemOperation::Query)
                .path("/users")
                .backend("scope discovery must not query the user-data subtree"),
        ),
    );

    let scopes = discover_rc1_channel_migration_scopes(filesystem)
        .await
        .expect("discover all rc1 tenants");
    assert_eq!(scopes.len(), 2);
    assert_eq!(scopes[0].admin_scope.tenant_id.as_str(), "tenant-a");
    assert_eq!(
        scopes[0].oauth_channel_secret_scope.user_id.as_str(),
        "operator-a"
    );
    assert_eq!(
        scopes[0]
            .oauth_channel_secret_scope
            .agent_id
            .as_ref()
            .map(AgentId::as_str),
        Some("agent-a")
    );
    assert_eq!(scopes[1].admin_scope.tenant_id.as_str(), "tenant-b");
    assert_eq!(
        scopes[1].oauth_channel_secret_scope.user_id.as_str(),
        "operator-b"
    );
}

#[tokio::test]
async fn slack_connection_disposition_pages_and_second_run_is_unchanged() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let admin_scope = ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("operator-a").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let shared = "/tenants/tenant-a/shared";
    let total = Page::MAX_LIMIT as usize + 1;
    for index in 0..total {
        let user = format!("user-{index:04}");
        let path = VirtualPath::new(format!(
            "{shared}/slack-personal-binding/connections/slack-install/{user}.json"
        ))
        .expect("connection path");
        let state = if index + 1 == total {
            "connecting"
        } else {
            "active"
        };
        let connection = serde_json::json!({
            "tenant_id": "tenant-a",
            "user_id": user,
            "installation_id": "slack-install",
            "epoch": ironclaw_auth::AuthFlowId::new(),
            "state": state,
            "expires_at": "2026-07-02T00:00:00Z",
            "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z"
        });
        filesystem
            .put(
                &path,
                Entry::bytes(serde_json::to_vec(&connection).expect("connection wire")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed connection");
    }

    let first = inspect_slack_connection_disposition(&filesystem, &admin_scope, shared)
        .await
        .expect("inspect every page");
    assert!(!first.already_complete);
    assert_eq!(first.marker.source_rows, total);
    assert_eq!(first.marker.active_superseded, total - 1);
    assert_eq!(first.marker.stale_expired, 1);
    let marker_path =
        format!("{shared}/channel-extensions/slack/migrations/rc1-connections-v1.complete.json");
    commit_disposition_marker(&filesystem, &marker_path, &first.marker, false)
        .await
        .expect("commit versioned disposition");

    let second = inspect_slack_connection_disposition(&filesystem, &admin_scope, shared)
        .await
        .expect("reverify retained source");
    assert!(second.already_complete);
    assert_eq!(second.marker.source_rows, total);
}

#[tokio::test]
async fn interrupted_rc1_slack_disconnect_fails_closed() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let admin_scope = ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("operator-a").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let shared = "/tenants/tenant-a/shared";
    let path = VirtualPath::new(format!(
        "{shared}/slack-personal-binding/connections/slack-install/user-a.json"
    ))
    .expect("connection path");
    let connection = serde_json::json!({
        "tenant_id": "tenant-a",
        "user_id": "user-a",
        "installation_id": "slack-install",
        "epoch": ironclaw_auth::AuthFlowId::new(),
        "state": "disconnecting",
        "disconnect_cleanup": {"kind": "all_owned"},
        "expires_at": "2026-07-02T00:00:00Z",
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-01T00:00:00Z"
    });
    filesystem
        .put(
            &path,
            Entry::bytes(serde_json::to_vec(&connection).expect("connection wire")),
            CasExpectation::Absent,
        )
        .await
        .expect("seed connection");

    assert!(matches!(
        inspect_slack_connection_disposition(&filesystem, &admin_scope, shared).await,
        Err(Rc1ChannelStateMigrationError::InterruptedSetup)
    ));
}
