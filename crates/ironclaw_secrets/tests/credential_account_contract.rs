use std::collections::BTreeSet;
use std::sync::Arc;

use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::{
    AgentId, ExtensionId, HostPath, InvocationId, MissionId, ProjectId, ResourceScope,
    SecretHandle, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_secrets::{
    CredentialAccountId, CredentialAccountRecord, CredentialAccountRepository, CredentialSecretRef,
    CredentialSlotId, FilesystemCredentialAccountRepository, InMemoryCredentialAccountRepository,
};
use tempfile::tempdir;

#[tokio::test]
async fn credential_accounts_allow_multiple_accounts_for_same_extension_slot_without_material() {
    let repository = InMemoryCredentialAccountRepository::new();
    let scope = sample_scope(Some("agent-a"), Some("project-a"));
    let extension = ExtensionId::new("gmail").unwrap();
    let slot = CredentialSlotId::new("google_oauth").unwrap();

    repository
        .upsert(gmail_account(
            scope.clone(),
            extension.clone(),
            slot.clone(),
            "personal",
            "Personal Gmail",
            "me@gmail.com",
            "gmail.personal.refresh_token",
        ))
        .await
        .unwrap();
    repository
        .upsert(gmail_account(
            scope.clone(),
            extension.clone(),
            slot.clone(),
            "work",
            "Work Gmail",
            "me@company.com",
            "gmail.work.refresh_token",
        ))
        .await
        .unwrap();

    let accounts = repository
        .list_for_slot(&scope, &extension, &slot)
        .await
        .unwrap();
    let labels = accounts
        .iter()
        .map(|account| account.label.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(labels, BTreeSet::from(["Personal Gmail", "Work Gmail"]));
    assert!(
        repository
            .get(
                &scope,
                &extension,
                &slot,
                &CredentialAccountId::new("personal").unwrap(),
            )
            .await
            .unwrap()
            .is_some()
    );
    assert!(!format!("{accounts:?}").contains("raw-token-value"));
}

#[tokio::test]
async fn credential_accounts_are_isolated_by_tenant_user_agent_and_project() {
    let repository = InMemoryCredentialAccountRepository::new();
    let extension = ExtensionId::new("gmail").unwrap();
    let slot = CredentialSlotId::new("google_oauth").unwrap();
    let account_id = CredentialAccountId::new("work").unwrap();
    let agent_a = sample_scope(Some("agent-a"), Some("project-a"));
    let agent_b = sample_scope(Some("agent-b"), Some("project-a"));
    let project_b = sample_scope(Some("agent-a"), Some("project-b"));

    repository
        .upsert(gmail_account(
            agent_a.clone(),
            extension.clone(),
            slot.clone(),
            "work",
            "Agent A Work Gmail",
            "a@company.com",
            "gmail.agent_a.refresh_token",
        ))
        .await
        .unwrap();
    repository
        .upsert(gmail_account(
            agent_b.clone(),
            extension.clone(),
            slot.clone(),
            "work",
            "Agent B Work Gmail",
            "b@company.com",
            "gmail.agent_b.refresh_token",
        ))
        .await
        .unwrap();
    repository
        .upsert(gmail_account(
            project_b.clone(),
            extension.clone(),
            slot.clone(),
            "work",
            "Project B Work Gmail",
            "project-b@company.com",
            "gmail.project_b.refresh_token",
        ))
        .await
        .unwrap();

    let agent_a_record = repository
        .get(&agent_a, &extension, &slot, &account_id)
        .await
        .unwrap()
        .unwrap();
    let agent_b_record = repository
        .get(&agent_b, &extension, &slot, &account_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        agent_a_record.subject_hint.as_deref(),
        Some("a@company.com")
    );
    assert_eq!(
        agent_b_record.subject_hint.as_deref(),
        Some("b@company.com")
    );
    assert_eq!(
        repository
            .list_for_slot(&agent_a, &extension, &slot)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repository
            .list_for_slot(&agent_b, &extension, &slot)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repository
            .list_for_slot(&project_b, &extension, &slot)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn filesystem_credential_account_repository_persists_metadata_without_secret_material() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = FilesystemCredentialAccountRepository::new(root.clone());
    let scope = sample_scope(None, Some("project-a"));
    let extension = ExtensionId::new("gmail").unwrap();
    let slot = CredentialSlotId::new("google_oauth").unwrap();
    let account_id = CredentialAccountId::new("work").unwrap();
    let record = gmail_account(
        scope.clone(),
        extension.clone(),
        slot.clone(),
        "work",
        "Work Gmail",
        "me@company.com",
        "gmail.work.refresh_token",
    );

    assert_eq!(
        repository
            .record_path(&scope, &extension, &slot, &account_id)
            .unwrap()
            .as_str(),
        "/engine/tenants/tenant-a/users/user-a/agents/_none/projects/project-a/credential-accounts/gmail/google_oauth/work.json"
    );

    repository.upsert(record).await.unwrap();
    let raw = root
        .read_file(
            &repository
                .record_path(&scope, &extension, &slot, &account_id)
                .unwrap(),
        )
        .await
        .unwrap();
    let raw = String::from_utf8(raw).unwrap();
    assert!(raw.contains("Work Gmail"));
    assert!(raw.contains("gmail.work.refresh_token"));
    assert!(!raw.contains("raw-token-value"));
    assert!(!raw.contains("refresh_token_value"));

    let reader = FilesystemCredentialAccountRepository::new(root);
    let read_back = reader
        .get(&scope, &extension, &slot, &account_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_back.label, "Work Gmail");
    assert_eq!(read_back.subject_hint.as_deref(), Some("me@company.com"));
}

#[tokio::test]
async fn filesystem_credential_account_repository_lists_and_deletes_slot_accounts() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = FilesystemCredentialAccountRepository::new(root);
    let scope = sample_scope(Some("agent-a"), None);
    let extension = ExtensionId::new("gmail").unwrap();
    let gmail_slot = CredentialSlotId::new("google_oauth").unwrap();
    let drive_slot = CredentialSlotId::new("drive_oauth").unwrap();

    repository
        .upsert(gmail_account(
            scope.clone(),
            extension.clone(),
            gmail_slot.clone(),
            "personal",
            "Personal Gmail",
            "me@gmail.com",
            "gmail.personal.refresh_token",
        ))
        .await
        .unwrap();
    repository
        .upsert(gmail_account(
            scope.clone(),
            extension.clone(),
            gmail_slot.clone(),
            "work",
            "Work Gmail",
            "me@company.com",
            "gmail.work.refresh_token",
        ))
        .await
        .unwrap();
    repository
        .upsert(gmail_account(
            scope.clone(),
            extension.clone(),
            drive_slot.clone(),
            "drive",
            "Drive Account",
            "drive@company.com",
            "drive.refresh_token",
        ))
        .await
        .unwrap();

    assert_eq!(
        repository
            .list_for_slot(&scope, &extension, &gmail_slot)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        repository
            .delete(
                &scope,
                &extension,
                &gmail_slot,
                &CredentialAccountId::new("personal").unwrap(),
            )
            .await
            .unwrap()
    );
    let remaining = repository
        .list_for_slot(&scope, &extension, &gmail_slot)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].account_id,
        CredentialAccountId::new("work").unwrap()
    );
    assert_eq!(
        repository
            .list_for_slot(&scope, &extension, &drive_slot)
            .await
            .unwrap()
            .len(),
        1
    );
}

fn gmail_account(
    scope: ResourceScope,
    extension_id: ExtensionId,
    slot_id: CredentialSlotId,
    account_id: &str,
    label: &str,
    subject_hint: &str,
    refresh_token_handle: &str,
) -> CredentialAccountRecord {
    CredentialAccountRecord::new(
        scope,
        extension_id,
        slot_id,
        CredentialAccountId::new(account_id).unwrap(),
        label,
    )
    .with_subject_hint(subject_hint)
    .with_secret_ref(
        CredentialSecretRef::new(
            "refresh_token",
            SecretHandle::new(refresh_token_handle).unwrap(),
        )
        .unwrap(),
    )
}

fn local_engine_root(path: &std::path::Path) -> Arc<LocalFilesystem> {
    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(path.to_path_buf()),
    )
    .unwrap();
    Arc::new(root)
}

fn sample_scope(agent: Option<&str>, project: Option<&str>) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: agent.map(|agent| AgentId::new(agent).unwrap()),
        project_id: project.map(|project| ProjectId::new(project).unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}
