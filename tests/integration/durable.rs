//! Reborn integration test — cross-reopen capability durability (E-DURABLE seam).
//!
//! Installs an extension through a real turn, then reopens a FRESH, independent
//! `ExtensionInstallationStorePort` at the capability harness's on-disk storage root
//! and asserts the install survived — proving capability-produced state persists
//! to disk, not just to in-memory state. Parallels
//! `assert_reply_persists_after_reopen` for capability state.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::collections::BTreeSet;

use ironclaw_approvals::{CapabilityPermissionOverride, CapabilityPermissionOverrideKey};
use ironclaw_composition::open_standalone_secret_store;
use ironclaw_composition::test_support::{
    open_standalone_approval_settings_stores_for_test,
    open_standalone_extension_installation_store_for_test,
    open_standalone_skill_management_for_test, open_standalone_thread_service_for_test,
};
use ironclaw_config::RebornStoragePaths;
use ironclaw_extension_manager::extension_lifecycle_command::RebornExtensionLifecycleRuntime;
use ironclaw_extension_registry::{ExtensionInstallationId, InstallationOwner};
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, ExtensionId, SecretHandle, ThreadId, UserId},
    resource::ResourceScope,
};
use ironclaw_secrets::SecretMaterial;
use ironclaw_threads::{
    AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, MessageContent,
    SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use reborn_support::group::{RebornIntegrationGroup, assert_exact_installation_owner};
use reborn_support::reply::RebornScriptedReply;
use secrecy::ExposeSecret;
use serde_json::json;

#[test]
fn durable_extension_reopen_rejects_tenant_wide_owner_and_requires_one_member() {
    let expected_member = UserId::new("durable-owner").expect("valid expected user");
    let rejected_member = UserId::new("durable-other-user").expect("valid rejected user");

    let tenant_result = assert_exact_installation_owner(
        &InstallationOwner::Tenant,
        &expected_member,
        &rejected_member,
        "github",
    );
    assert!(
        tenant_result.is_err(),
        "legacy tenant-wide visibility must not satisfy a personal-install reopen assertion"
    );

    assert_exact_installation_owner(
        &InstallationOwner::users(BTreeSet::from([expected_member.clone()]))
            .expect("singleton owner set"),
        &expected_member,
        &rejected_member,
        "github",
    )
    .expect("the original user must remain the exact sole member");
}

#[test]
fn durable_host_state_survives_cold_reopen_with_exact_tenant_user_ownership() {
    run_async_test_with_stack(
        "durable_host_state_survives_cold_reopen_with_exact_tenant_user_ownership",
        durable_host_state_survives_cold_reopen_with_exact_tenant_user_ownership_async,
    );
}

async fn durable_host_state_survives_cold_reopen_with_exact_tenant_user_ownership_async() {
    const THREAD_MESSAGE: &str = "DURABLE_THREAD_MESSAGE_SENTINEL";
    const HOST_SECRET: &str = "DURABLE_HOST_SECRET_SENTINEL";
    const SYSTEM_SKILL: &str = "durable-system-skill";
    const USER_SKILL: &str = "durable-user-skill";
    const USER_SKILL_PROMPT: &str = "DURABLE_USER_SKILL_SENTINEL";
    const USER_SKILL_CONTENT: &str = "---\nname: durable-user-skill\ndescription: durable user skill\nactivation:\n  keywords: [\"durable-user-skill\"]\n---\n\nDURABLE_USER_SKILL_SENTINEL";

    let (
        installation_root,
        thread_scope,
        thread_id,
        secret_scope,
        secret_handle,
        setting_key,
        owner,
        rejected_owner,
    ) = {
        let group = RebornIntegrationGroup::extension_lifecycle_with_preboot_user_skills(&[(
            USER_SKILL,
            "durable user skill",
            USER_SKILL_PROMPT,
            false,
        )])
        .await
        .expect("extension lifecycle group builds with its preboot user skill");
        let harness = group
            .thread("conv-durable-cold-reopen")
            .script([
                RebornScriptedReply::tool_call(
                    "builtin.extension_install",
                    json!({"extension_id": "github"}),
                ),
                RebornScriptedReply::text("installed"),
            ])
            .build()
            .await
            .expect("extension lifecycle thread builds");
        harness
            .seed_capability_credential_account("github", "durable github ready path", &[])
            .await
            .expect("GitHub credential is ready for the durable install path");
        harness
            .submit_turn("install github")
            .await
            .expect("extension install turn completes");
        harness
            .assert_tool_result_contains("\"installed\":true")
            .await
            .expect("extension install reports success");

        let capability = group
            .capability_harness()
            .expect("extension lifecycle group owns the production capability harness");
        let owner = harness.binding.actor_user_id.clone();
        let rejected_owner =
            UserId::new("durable-cold-reopen-second-user").expect("valid second user id");
        let tenant = harness.binding.tenant_id.clone();
        let installation_root = capability.storage_root_for_test();

        capability
            .seed_system_skill_for_test(
                SYSTEM_SKILL,
                "durable system skill",
                "DURABLE_SYSTEM_SKILL_SENTINEL",
            )
            .expect("system skill fixture seeds in the canonical system namespace");
        let setting_capability =
            CapabilityId::new("builtin.write_file").expect("write-file capability id");
        capability
            .set_ask_each_time_override_for_test(&setting_capability, tenant.clone(), owner.clone())
            .await
            .expect("typed tenant/user setting writes through the host store");

        let thread_scope = ThreadScope {
            tenant_id: tenant.clone(),
            agent_id: AgentId::new("durable-cold-reopen-agent").expect("valid agent id"),
            project_id: None,
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let thread_id = ThreadId::new("durable-cold-reopen-thread").expect("valid thread id");
        let secret_scope = ResourceScope {
            tenant_id: tenant,
            user_id: owner.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: Default::default(),
        };
        let secret_handle = SecretHandle::new("durable-host-secret").expect("valid secret handle");

        let runtime = capability
            .reborn_services_for_test()
            .expect("host capability harness retains its production RebornRuntime");
        let live_user_skill = runtime
            .skill_management()
            .read_content_for_scope(secret_scope.clone(), USER_SKILL)
            .await
            .expect("preboot fixture imports into the live production user-skill service");
        assert_eq!(live_user_skill.content, USER_SKILL_CONTENT);
        assert_eq!(live_user_skill.source.as_str(), "user");
        let thread_service = runtime
            .standalone_thread_service_for_test()
            .expect("standalone thread service");
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: owner.as_str().to_string(),
                title: Some("durable cold-reopen thread".to_string()),
                metadata_json: None,
            })
            .await
            .expect("production thread service persists the typed thread owner");
        thread_service
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: "durable-cold-reopen-run".to_string(),
                content: MessageContent::text(THREAD_MESSAGE),
            })
            .await
            .expect("production thread service persists the representative message");

        // This sentinel is written and later resolved through host-side secret
        // stores only. This deterministic lifecycle invokes no sandbox and does
        // not place secret material in model, tool, process, or environment input.
        runtime
            .secret_store_for_test()
            .put(
                secret_scope.clone(),
                secret_handle.clone(),
                SecretMaterial::from(HOST_SECRET.to_string()),
                None,
            )
            .await
            .expect("host secret store encrypts the sentinel before cold reopen");

        let setting_key = CapabilityPermissionOverrideKey::new(&secret_scope, setting_capability);
        drop(thread_service);

        // End this scope before opening independent services below: it drops
        // the harness, its composed runtime, and all current store handles.
        (
            installation_root,
            thread_scope,
            thread_id,
            secret_scope,
            secret_handle,
            setting_key,
            owner,
            rejected_owner,
        )
    };
    let storage_paths = RebornStoragePaths::from_installation_root(&installation_root);

    let reopened_threads = open_standalone_thread_service_for_test(&installation_root)
        .await
        .expect("fresh production thread-service opener");
    let history = reopened_threads
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .expect("fresh thread service reads the seeded thread");
    assert_eq!(history.thread.thread_id, thread_id);
    assert_eq!(history.thread.scope, thread_scope);
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].content.as_deref(), Some(THREAD_MESSAGE));

    let reopened_secret_store = open_standalone_secret_store(storage_paths.state_root())
        .await
        .expect("fresh host secret-store opener");
    let rejected_secret_scope = ResourceScope {
        user_id: rejected_owner.clone(),
        ..secret_scope.clone()
    };
    assert!(
        reopened_secret_store
            .lease_once(&rejected_secret_scope, &secret_handle)
            .await
            .is_err(),
        "a second user must not resolve the original user's encrypted secret"
    );
    let lease = reopened_secret_store
        .lease_once(&secret_scope, &secret_handle)
        .await
        .expect("original typed tenant/user scope leases the host-side secret");
    let material = reopened_secret_store
        .consume(&secret_scope, lease.id)
        .await
        .expect("original typed tenant/user scope consumes the host-side secret");
    assert_eq!(material.expose_secret(), HOST_SECRET);

    let reopened_extensions =
        open_standalone_extension_installation_store_for_test(&installation_root)
            .await
            .expect("fresh extension installation-store opener");
    let extension_id = ExtensionId::new("github").expect("valid extension id");
    let installation_id =
        ExtensionInstallationId::new(extension_id.as_str()).expect("valid installation id");
    let installation = reopened_extensions
        .get_installation(&installation_id)
        .await
        .expect("fresh installation store reads github")
        .expect("github installation survives the cold reopen");
    assert_exact_installation_owner(
        installation.owner(),
        &owner,
        &rejected_owner,
        extension_id.as_str(),
    )
    .expect("reopened installation has exactly the original personal owner");

    let (reopened_overrides, _, _) =
        open_standalone_approval_settings_stores_for_test(storage_paths.installation_root())
            .await
            .expect("fresh approval-settings opener");
    let setting = reopened_overrides
        .get(&setting_key)
        .await
        .expect("fresh settings store reads the typed override")
        .expect("AskEachTime override survives the cold reopen");
    assert_eq!(setting.state, CapabilityPermissionOverride::AskEachTime);
    assert_eq!(setting.key, setting_key);

    let system_skill = std::fs::read_to_string(
        storage_paths
            .system_root()
            .join("skills")
            .join(SYSTEM_SKILL)
            .join("SKILL.md"),
    )
    .expect("system skill remains in canonical system storage after cold reopen");
    assert!(system_skill.contains("DURABLE_SYSTEM_SKILL_SENTINEL"));
    let reopened_skills =
        open_standalone_skill_management_for_test(&installation_root, owner.clone())
            .await
            .expect("fresh production skill-management opener");
    let user_skill = reopened_skills
        .read_content_for_scope(secret_scope.clone(), USER_SKILL)
        .await
        .expect("original typed tenant/user scope resolves the persisted user skill");
    assert_eq!(user_skill.content, USER_SKILL_CONTENT);
    assert_eq!(user_skill.source.as_str(), "user");
    assert!(
        reopened_skills
            .read_content_for_scope(rejected_secret_scope, USER_SKILL)
            .await
            .is_err(),
        "the second user must not receive the original user's skill leaf"
    );

    let system_prompt = std::fs::read_to_string(
        storage_paths
            .system_root()
            .join("prompts")
            .join("default-system.md"),
    )
    .expect("boot-seeded system prompt survives the cold reopen");
    assert!(
        !system_prompt.is_empty(),
        "the persisted system prompt must remain a readable file"
    );
}

#[test]
fn extension_install_survives_independent_reopen() {
    run_async_test_with_stack(
        "extension_install_survives_independent_reopen",
        extension_install_survives_independent_reopen_async,
    );
}

async fn extension_install_survives_independent_reopen_async() {
    let group = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("extension-lifecycle group builds");
    let harness = group
        .thread("conv-durable")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await
        .expect("thread builds");
    harness
        .seed_capability_credential_account("github", "durable github ready path", &[])
        .await
        .expect("GitHub credential is ready for the durable install path");

    harness
        .submit_turn("install github")
        .await
        .expect("turn completes");
    harness
        .assert_tool_result_contains("\"installed\":true")
        .await
        .expect("install reported success");

    harness
        .assert_extension_install_membership_persists_after_reopen(
            "github",
            &harness.binding.actor_user_id,
            &UserId::new("durable-second-user").expect("valid second user"),
        )
        .await
        .expect("installed extension membership survives an independent reopen");
}

fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
