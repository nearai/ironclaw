use super::*;

/// One owner backs every lane that lands workspace *bytes*: the WebUI
/// attachment handle (`RebornRuntime::webui_workspace_filesystem`), the
/// production channel-inbound lander in `channel_host_source`, and the
/// C-ATTACH test seam. Pin that its two branches address different roots,
/// so no lane can be wired to a shared root while its sibling is scoped.
#[test]
fn read_write_workspace_handle_follows_the_deployment_policy() {
    use ironclaw_host_api::{
        ids::{InvocationId, TenantId, UserId},
        path::ScopedPath,
    };

    let scope = ResourceScope {
        tenant_id: TenantId::new("acme").expect("tenant id"),
        user_id: UserId::new("alice").expect("user id"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let path = ScopedPath::new("/workspace/landed.txt").expect("scoped path");
    let backend = Arc::new(CompositeRootFilesystem::default());

    let per_caller = read_write_workspace_filesystem(&backend, &WorkspaceMountPolicy::PerCaller)
        .expect("per-caller handle builds");
    let shared = read_write_workspace_filesystem(
        &backend,
        &WorkspaceMountPolicy::Shared(
            workspace_mount_view(MountPermissions::read_write(), &[]).expect("shared view builds"),
        ),
    )
    .expect("shared handle builds");

    assert_eq!(
        per_caller
            .resolve(&scope, &path)
            .expect("per-caller resolve")
            .as_str(),
        "/projects/workspace/users/c711caa52fd730885e365ba866cb387c38357e3a82dc675071d1bb9ac834fd22/landed.txt"
    );
    assert_eq!(
        shared
            .resolve(&scope, &path)
            .expect("shared resolve")
            .as_str(),
        "/projects/workspace/landed.txt"
    );
}

#[test]
fn shared_browser_workspace_uses_shared_root_while_memory_stays_caller_scoped() {
    use ironclaw_host_api::{
        ids::{InvocationId, TenantId, UserId},
        path::ScopedPath,
    };

    let alice_scope = ResourceScope {
        tenant_id: TenantId::new("acme").expect("tenant id"),
        user_id: UserId::new("alice").expect("user id"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let bob_scope = ResourceScope {
        tenant_id: TenantId::new("acme").expect("tenant id"),
        user_id: UserId::new("bob").expect("user id"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let policy = WorkspaceMountPolicy::Shared(
        workspace_mount_view(MountPermissions::read_write(), &[]).expect("shared workspace policy"),
    );
    let alice_browser =
        webui_browse_mount_view(&policy, &alice_scope).expect("alice browser mount view");
    let bob_browser = webui_browse_mount_view(&policy, &bob_scope).expect("bob browser mount view");
    let workspace_target = alice_browser
        .resolve(&ScopedPath::new("/workspace/landed.txt").expect("workspace path"))
        .expect("browser workspace resolve");
    let bob_workspace_target = bob_browser
        .resolve(&ScopedPath::new("/workspace/landed.txt").expect("workspace path"))
        .expect("bob browser workspace resolve");
    let memory_target = alice_browser
        .resolve(&ScopedPath::new("/memory/note.md").expect("memory path"))
        .expect("browser memory resolve");
    let bob_memory_target = bob_browser
        .resolve(&ScopedPath::new("/memory/note.md").expect("memory path"))
        .expect("bob browser memory resolve");

    assert_eq!(workspace_target.as_str(), "/projects/workspace/landed.txt");
    assert_eq!(workspace_target, bob_workspace_target);
    assert_eq!(
        memory_target.as_str(),
        "/memory/tenants/acme/users/alice/agents/_none/projects/_none/note.md"
    );
    assert_eq!(
        bob_memory_target.as_str(),
        "/memory/tenants/acme/users/bob/agents/_none/projects/_none/note.md"
    );
}

#[test]
fn per_caller_browser_workspace_stays_isolated_between_callers() {
    use ironclaw_host_api::{
        ids::{InvocationId, TenantId, UserId},
        path::ScopedPath,
    };

    let alice_scope = ResourceScope {
        tenant_id: TenantId::new("acme").expect("tenant id"),
        user_id: UserId::new("alice").expect("user id"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let bob_scope = ResourceScope {
        tenant_id: TenantId::new("acme").expect("tenant id"),
        user_id: UserId::new("bob").expect("user id"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let path = ScopedPath::new("/workspace/landed.txt").expect("workspace path");
    let policy = WorkspaceMountPolicy::PerCaller;

    let alice_target = webui_browse_mount_view(&policy, &alice_scope)
        .expect("alice browser mount view")
        .resolve(&path)
        .expect("alice browser workspace resolve");
    let bob_target = webui_browse_mount_view(&policy, &bob_scope)
        .expect("bob browser mount view")
        .resolve(&path)
        .expect("bob browser workspace resolve");

    assert_eq!(
        alice_target.as_str(),
        "/projects/workspace/users/c711caa52fd730885e365ba866cb387c38357e3a82dc675071d1bb9ac834fd22/landed.txt"
    );
    assert_ne!(alice_target, bob_target);
}

#[test]
fn ambient_workspace_mount_rejects_invalid_workspace_alias() {
    let err = ambient_workspace_mount_view(
        MountPermissions::read_write(),
        &[Path::new(r"C:\Users\alice\project")],
        &[],
    )
    .expect_err("invalid workspace alias should fail loudly");

    assert!(
        err.to_string().contains("backslashes are not allowed"),
        "unexpected error: {err}"
    );
}

#[test]
fn workspace_mount_rejects_host_home_alias_that_is_not_mount_shaped() {
    let err = workspace_mount_view(
        MountPermissions::read_write(),
        &[Path::new(r"C:\Users\alice")],
    )
    .expect_err("invalid raw alias should fail loudly");

    assert!(
        err.to_string().contains("backslashes are not allowed"),
        "unexpected error: {err}"
    );
}

#[test]
fn ambient_workspace_mount_deduplicates_workspace_alias_against_canonical_workspace() {
    let mounts = ambient_workspace_mount_view(
        MountPermissions::read_write(),
        &[Path::new(WORKSPACE_ALIAS)],
        &[],
    )
    .expect("mount view builds");

    assert_eq!(
        mounts
            .mounts
            .iter()
            .filter(|mount| mount.alias.as_str() == WORKSPACE_ALIAS)
            .count(),
        1
    );
}

#[test]
fn workspace_mount_deduplicates_normalized_host_home_aliases() {
    let mounts = workspace_mount_view(
        MountPermissions::read_write(),
        &[
            Path::new("/Users/alice"),
            Path::new("/Users/alice/"),
            Path::new("/Users/alice/."),
        ],
    )
    .expect("mount view builds");

    assert_eq!(
        mounts
            .mounts
            .iter()
            .filter(|mount| mount.alias.as_str() == "/Users/alice")
            .count(),
        1
    );
}

#[test]
fn ambient_workspace_mount_includes_raw_workspace_alias() {
    let mounts = ambient_workspace_mount_view(
        MountPermissions::read_write(),
        &[Path::new("/Users/alice/project")],
        &[Path::new("/Users/alice")],
    )
    .expect("mount view builds");

    let mount_for = |alias: &str| {
        mounts
            .mounts
            .iter()
            .find(|mount| mount.alias.as_str() == alias)
            .unwrap_or_else(|| panic!("missing mount alias {alias}"))
    };
    assert_eq!(
        mount_for("/Users/alice/project").target.as_str(),
        WORKSPACE_TARGET
    );
    assert_eq!(mount_for("/Users/alice").target.as_str(), HOST_TARGET);
}
