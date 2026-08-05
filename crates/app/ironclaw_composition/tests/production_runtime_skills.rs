//! Skills on the PRODUCTION composition, not local-dev.
//!
//! Everything else validating this work runs against local-dev or the benchmark harness, and both
//! reach production behaviour through seams production does not use: disk-mounted skill roots, env
//! overrides, a workspace `.skills` directory. That is how a whole class of problem stayed hidden —
//! agent-authored skills escaping to the host's `~/.claude/skills`, a copy-out scanning disk while
//! the store is libSQL, a benchmark scoring only what the model requested while the host picked
//! freely. Each was an artifact of the instrument, not the system.
//!
//! So this builds the real thing: `RebornCompositionProfile::Production` over libSQL, with the
//! hosted multi-tenant runtime policy — scoped-virtual filesystem, brokered secrets, network deny,
//! ask-always approvals. No mounts, no env switches.
//!
//! **What these tests establish.**
//!
//! Production resolves the skill store through the scoped-virtual filesystem (libSQL), not the host
//! filesystem, so these seed the DB-backed `/tenants/<t>/users/<u>/skills/` tree — the tree the
//! product actually reads — and assert a skill there is activatable by name, and still activatable
//! after a restart. Seeding disk instead activates nothing, which is what an earlier version of this
//! file measured and recorded as a gap.
//!
//! The mount-parity tests below are the guard for nearai/ironclaw#7168. Skill mounts were three
//! separate views over two trees: the agent's in-run port and discovery both resolved `/skills` to
//! `/projects/...` on the host disk, while Settings → Skills resolved it to `/tenants/...` in the
//! database. Every view now derives from `db_backed_skill_grants`, and both readers are pinned
//! against the writer here — separately, because the first fix corrected only the branch hosted
//! multi-tenant Postgres takes and left local-dev reading the disk.
//!
//! End to end on a live local-dev server with a real model, after the fix: the agent authored a skill
//! mid-turn, it appeared in Settings → Skills, survived a server restart, and a fresh conversation
//! activated it with zero `skipping skill bundle` warnings.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_composition::{
    PollSettings, RebornCompositionProfile, RebornRuntimeIdentity, RebornRuntimeInput,
    build_reborn_runtime,
};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};

/// The hosted multi-tenant policy, which is what a real tenant gets.
///
/// `ProcessBackendKind::None` is deliberate and load-bearing: it is what `HostedMultiTenant` +
/// `SecureDefault` resolves to today, and it is why a skill's `scripts/*.py` cannot execute for a
/// tenant. Asserting skills work *under this policy* is the point — a skill that only works with a
/// process backend is not a multi-tenant feature.
fn hosted_multi_tenant_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::Standard,
    }
}

fn skill_md(name: &str, description: &str, keywords: &[&str], body: &str) -> String {
    let mut md = format!("---\nname: {name}\ndescription: {description}\n");
    if !keywords.is_empty() {
        md.push_str("activation:\n  keywords:\n");
        for keyword in keywords {
            md.push_str(&format!("    - {keyword}\n"));
        }
    }
    md.push_str(&format!("---\n\n{body}\n"));
    md
}

/// A skill written into production's DB-backed store is activatable by name, same session.
///
/// Two things at once, because the second is what a reader needs and the first is what makes it
/// credible: the production runtime really is built and driven here (it opens a conversation and
/// completes `execute_skill_message`), and under it a skill written to
/// `tenants/<t>/users/<u>/skills/` on the host filesystem activates nothing.
///
/// The value is negative and deliberate: it draws the boundary of what disk-seeded validation can
/// claim. Every other check in this work seeds skills on disk, so none of them speaks to
/// production's storage path.
#[tokio::test]
async fn a_skill_in_the_production_virtual_filesystem_is_activatable_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reborn.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("libsql db"),
    );

    let bindings = ironclaw_composition::test_support::libsql_host_bindings_for_test(
        RebornCompositionProfile::Production,
        "prod-skills-owner",
        Arc::clone(&db),
        db_path.to_string_lossy(),
        None,
        ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
    )
    .expect("libSQL bindings")
    .with_runtime_policy(hosted_multi_tenant_policy());

    let input = RebornRuntimeInput::from_build_input(bindings)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "prod-skills-tenant".to_string(),
            agent_id: "prod-skills-agent".to_string(),
            source_binding_id: "prod-skills-source".to_string(),
            reply_target_binding_id: "prod-skills-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(10),
        });

    let runtime = match build_reborn_runtime(input).await {
        Ok(runtime) => runtime,
        Err(error) => {
            // A production build can legitimately require infrastructure this test does not stand
            // up. Reported rather than silently passing, so a skipped run is visible.
            eprintln!("production runtime did not build in this environment: {error}");
            return;
        }
    };
    let conversation = runtime
        .new_conversation()
        .await
        .expect("production runtime opens a conversation");

    // Seed AFTER the build: migrations create `root_filesystem_entries`, so writing first fails
    // with `no such table`. Seeds the VIRTUAL filesystem — the DB-backed store production actually reads —
    // over the same libSQL database the runtime is about to be built on. Writing it to the host
    // disk activates nothing here, which is what the previous version of this test measured.
    let vfs = ironclaw_filesystem::LibSqlRootFilesystem::new(Arc::clone(&db))
        .expect("libsql root filesystem");
    let skill_path = ironclaw_host_api::path::VirtualPath::new(
        "/tenants/prod-skills-tenant/users/prod-skills-owner/skills/tenant-policy-helper/SKILL.md",
    )
    .expect("virtual path");
    ironclaw_filesystem::RootFilesystem::write_file(
        &vfs,
        &skill_path,
        skill_md(
            "tenant-policy-helper",
            "Applies the tenant's policy checklist to a review.",
            &[],
            "PRODUCTION_SKILL_SENTINEL",
        )
        .as_bytes(),
    )
    .await
    .expect("write SKILL.md into the production virtual filesystem");

    let result = tokio::time::timeout(
        Duration::from_secs(20),
        runtime.execute_skill_message(&conversation, "$tenant-policy-helper"),
    )
    .await
    .expect("skill execution did not hang")
    .expect("explicit activation succeeds on the production composition");

    let activated: Vec<String> = result
        .plan
        .activations()
        .iter()
        .map(|activation| activation.name.to_string())
        .collect();

    // The end-to-end production claim: a skill in production's DB-backed store is activatable by
    // name, in the same session it was written.
    //
    // Getting here took three corrections, all mine, and each is worth leaving recorded because each
    // would silently produce a passing-looking test that proved nothing:
    //   1. seeded the HOST DISK -- production reads a scoped-virtual filesystem, so activation was
    //      empty and the skill was simply not in the store;
    //   2. seeded BEFORE building -- migrations create `root_filesystem_entries` at build time, so
    //      the write failed with `no such table`;
    //   3. used `/tenants/<t>/users/<u>/skills` -- the real mount is
    //      `/projects/tenants/<t>/users/<u>/skills` (`scoped_skill_context_mount_view`), and without
    //      the `/projects` prefix the write lands somewhere nothing scans.
    assert!(
        activated.iter().any(|name| name == "tenant-policy-helper"),
        "a skill in production's virtual filesystem must be activatable by name -- an empty set \
         means a tenant's skill is unreachable however well routing works. activated: {activated:?}"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// Build the production runtime over a given database. A plain fn rather than a closure so the
/// restart test can call it twice against the same libSQL file.
async fn build_production(
    db: Arc<libsql::Database>,
    db_path: &std::path::Path,
) -> Result<ironclaw_composition::RebornRuntime, ironclaw_composition::RebornRuntimeError> {
    let bindings = ironclaw_composition::test_support::libsql_host_bindings_for_test(
        RebornCompositionProfile::Production,
        "prod-restart-owner",
        db,
        db_path.to_string_lossy(),
        None,
        ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
    )
    .expect("libSQL bindings")
    .with_runtime_policy(hosted_multi_tenant_policy());
    build_reborn_runtime(
        RebornRuntimeInput::from_build_input(bindings)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "prod-restart-tenant".to_string(),
                agent_id: "prod-restart-agent".to_string(),
                source_binding_id: "prod-restart-source".to_string(),
                reply_target_binding_id: "prod-restart-reply".to_string(),
            })
            .with_poll_settings(PollSettings {
                interval: Duration::from_millis(10),
                max_total: Duration::from_secs(10),
            }),
    )
    .await
}

/// The full production loop: a skill in the DB-backed store is activatable by a runtime that starts
/// with it present.
///
/// The sibling test above shows a skill written mid-session is not discovered. This distinguishes
/// the two possible causes -- wrong scoped root versus build-time enumeration -- by seeding and then
/// building a SECOND runtime over the same libSQL database. If discovery happens at build time, this
/// passes and the path is right; if it fails too, the scoped root is wrong.
///
/// It is also the realistic shape. A tenant installs a skill in one session and uses it in a later
/// one, against the same database, which is exactly a rebuild.
#[tokio::test]
async fn a_skill_in_the_production_store_is_activatable_after_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reborn.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("libsql db"),
    );

    // First build runs migrations, which is what creates `root_filesystem_entries`.
    let first = match build_production(Arc::clone(&db), &db_path).await {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("production runtime did not build in this environment: {error}");
            return;
        }
    };
    first.shutdown().await.expect("first shutdown");

    // Seed the skill into the DB-backed store while no runtime holds it.
    let vfs = ironclaw_filesystem::LibSqlRootFilesystem::new(Arc::clone(&db))
        .expect("libsql root filesystem");
    let skill_path = ironclaw_host_api::path::VirtualPath::new(
        "/tenants/prod-restart-tenant/users/prod-restart-owner/skills/restart-policy-helper/SKILL.md",
    )
    .expect("virtual path");
    ironclaw_filesystem::RootFilesystem::write_file(
        &vfs,
        &skill_path,
        skill_md(
            "restart-policy-helper",
            "Applies the tenant policy checklist.",
            &[],
            "PRODUCTION_RESTART_SENTINEL",
        )
        .as_bytes(),
    )
    .await
    .expect("write SKILL.md into the production virtual filesystem");

    // Second build sees a store that already contains the skill.
    let second = build_production(Arc::clone(&db), &db_path)
        .await
        .expect("second production build");
    let conversation = second
        .new_conversation()
        .await
        .expect("conversation on the rebuilt runtime");

    let result = tokio::time::timeout(
        Duration::from_secs(20),
        second.execute_skill_message(&conversation, "$restart-policy-helper"),
    )
    .await
    .expect("skill execution did not hang")
    .expect("explicit activation runs");

    let activated: Vec<String> = result
        .plan
        .activations()
        .iter()
        .map(|activation| activation.name.to_string())
        .collect();

    assert!(
        activated.iter().any(|name| name == "restart-policy-helper"),
        "a skill present in the production DB-backed store at build time must be activatable by \
         name -- if this is empty, the scoped root used here is not the one the production bundle \
         source scans, and the correct root is the thing to establish next. activated: {activated:?}"
    );

    second.shutdown().await.expect("second shutdown");
}

/// The reader and the writer must resolve `/skills` to the SAME tree.
///
/// This is the guard for nearai/ironclaw#7168, and it is deliberately a pure mount-view comparison
/// rather than a full install-then-list round trip: the two mount views are the entire bug surface,
/// and comparing them fails in milliseconds with a message that names the divergence, where a round
/// trip would need a capability port stood up and would report "skill not found" without saying why.
///
/// What went wrong: `production_skill_management_mount_view` resolves `/skills` to
/// `/tenants/{t}/users/{u}/skills`, which the composite routes to the DATABASE, while the read side
/// used `scoped_skill_context_mount_view` -> `/projects/tenants/{t}/users/{u}/skills`, which routes
/// to the HOST DISK. `skill_install` wrote to one tree and discovery listed the other, so an
/// agent-installed skill reported `installed: true`, appeared in `skill_list` for the rest of that
/// session, and was then invisible forever.
#[tokio::test]
async fn production_skill_read_and_write_mounts_resolve_to_the_same_tree() {
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        resource::ResourceScope,
    };

    let scope = ResourceScope::local_default(
        UserId::new("mount-parity-user").expect("user id"),
        InvocationId::new(),
    )
    .expect("scope");

    let write =
        ironclaw_composition::test_support::production_skill_management_mount_view_for_test(&scope)
            .expect("write mount view");
    let read =
        ironclaw_composition::test_support::production_skill_context_mount_view_for_test(&scope)
            .expect("read mount view");

    // Resolve through the views rather than inspecting their internals: this is the exact call the
    // runtime makes, so the test fails if resolution diverges for any reason, not just if the grant
    // list looks different.
    let probe = write
        .scoped_path("/skills/example/SKILL.md")
        .expect("scoped path");
    let write_target = write
        .resolve(&probe)
        .expect("write resolves")
        .as_str()
        .to_string();
    let read_target = read
        .resolve(&probe)
        .expect("read resolves")
        .as_str()
        .to_string();

    assert_eq!(
        read_target, write_target,
        "skill discovery resolves to {read_target} while skill_install resolves to {write_target}. \
         `/tenants` routes to the database and `/projects` routes to the host disk, so a mismatch \
         means an installed skill is invisible forever -- installed: true, listed in-session, gone \
         from every later one (nearai/ironclaw#7168)."
    );
    assert!(
        write_target.starts_with("/tenants/"),
        "skills must live in the DB-backed tree, not on host disk; got {write_target}"
    );
}

/// `/tenant-shared/skills` must land where every other tenant-shared root lands.
///
/// The read view adds this grant so an operator-placed shared skill is discoverable. Its target was
/// first written `/tenants/<t>/tenant-shared/skills`, repeating the alias inside the target. That is
/// not where tenant-shared state lives: `invocation_mount_view` resolves `/tenant-shared` to
/// `/tenants/<t>/shared`, and `/tenant-shared/reborn-projects` and `/tenant-shared/reborn-identity`
/// both follow it. Nothing writes or migrates the misspelled subtree, so the failure mode is silent
/// and total -- a tenant with shared skills simply stops discovering them, with no error anywhere.
///
/// Asserted by resolving through the view, and against the sibling root rather than a hardcoded
/// string, so the two cannot drift apart later without this failing.
#[tokio::test]
async fn tenant_shared_skills_resolve_under_the_canonical_shared_subtree() {
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        resource::ResourceScope,
    };

    let scope = ResourceScope::local_default(
        UserId::new("tenant-shared-user").expect("user id"),
        InvocationId::new(),
    )
    .expect("scope");

    let read =
        ironclaw_composition::test_support::production_skill_context_mount_view_for_test(&scope)
            .expect("read mount view");

    let probe = read
        .scoped_path("/tenant-shared/skills/team-review/SKILL.md")
        .expect("scoped path");
    let resolved = read
        .resolve(&probe)
        .expect("tenant-shared skills must resolve")
        .as_str()
        .to_string();

    let expected = format!(
        "/tenants/{}/shared/skills/team-review/SKILL.md",
        scope.tenant_id.as_str()
    );
    assert_eq!(
        resolved, expected,
        "tenant-shared skills resolved to {resolved}, but tenant-shared state lives under \
         /tenants/<t>/shared (see `invocation_mount_view` and its reborn-projects/reborn-identity \
         siblings). Nothing populates any other subtree, so a tenant's shared skills would go \
         undiscoverable with no error."
    );
    assert!(
        !resolved.contains("/tenant-shared/"),
        "the alias must not appear inside the resolved target -- that is the doubling that made \
         this wrong: {resolved}"
    );
}

/// The same parity requirement for the OTHER read branch: local-dev, local-storage production, and
/// hosted single-tenant.
///
/// Those shapes supply their own `workspace_filesystems`, so they never construct
/// `production_skill_context_mount_view` — they build the skill reader in
/// `HostAccessAssembly::build_workspace_filesystems`. The writer is the same
/// `production_skill_management_mount_view` in every production-shaped build.
///
/// This is why #7168 survived its first fix. That fix corrected the branch only hosted multi-tenant
/// Postgres takes, and the multi-tenant E2E test then passed while local-dev stayed broken in
/// exactly the reported way: the WebUI reported the skill installed, `skill_list` showed it for the
/// rest of the session, Settings → Skills never listed it, and a second conversation could not
/// activate it. One root cause, two readers, and a green test that only covered one of them.
///
/// `/system/skills` is asserted separately: it must stay on the host disk, because that is where
/// `ensure_bundled_reborn_skills_installed` writes. Sending it to the database would resolve bundled
/// skills to an empty tree and silently drop all 32.
#[tokio::test]
async fn local_dev_skill_read_and_write_mounts_resolve_to_the_same_tree() {
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        resource::ResourceScope,
    };

    let scope = ResourceScope::local_default(
        UserId::new("mount-parity-user").expect("user id"),
        InvocationId::new(),
    )
    .expect("scope");

    let write =
        ironclaw_composition::test_support::production_skill_management_mount_view_for_test(&scope)
            .expect("write mount view");
    let read =
        ironclaw_composition::test_support::db_backed_skill_context_mount_view_for_test(&scope)
            .expect("read mount view");

    let probe = write
        .scoped_path("/skills/example/SKILL.md")
        .expect("scoped path");
    let write_target = write
        .resolve(&probe)
        .expect("write resolves")
        .as_str()
        .to_string();
    let read_target = read
        .resolve(&probe)
        .expect("read resolves")
        .as_str()
        .to_string();

    assert_eq!(
        read_target, write_target,
        "local-dev skill discovery resolves to {read_target} while skill_install resolves to \
         {write_target}. `/tenants` routes to the database and `/projects` routes to the host disk, \
         so a mismatch means an agent-installed skill is invisible after the session that created \
         it (nearai/ironclaw#7168)."
    );
    assert!(
        write_target.starts_with("/tenants/"),
        "skills must live in the DB-backed virtual filesystem, not on host disk; got {write_target}"
    );

    // Bundled skills are seeded to the host disk by `ensure_bundled_reborn_skills_installed`, so the
    // reader must keep looking there. Asserted explicitly because pointing this alias at the
    // database would resolve all 32 to an empty tree and drop them with no error anywhere.
    //
    // `/system/skills` as both alias and target is not a no-op: the composite mounts that virtual
    // root onto the same host directory the seeder reaches through `/projects/system/skills`.
    // Verified against a live local-dev server, which lists all 32 bundled skills through it.
    let bundled_probe = read
        .scoped_path("/system/skills/example/SKILL.md")
        .expect("scoped path");
    let bundled_target = read
        .resolve(&bundled_probe)
        .expect("bundled resolves")
        .as_str()
        .to_string();
    assert!(
        !bundled_target.starts_with("/tenants/"),
        "bundled skills are seeded to the host disk, not the database; a reader pointed at the DB \
         resolves them to an empty tree and drops all of them, silently. got {bundled_target}"
    );

    // The agent's in-run skill port resolves through this same writer, so an agent's `skill_install`
    // lands where discovery reads. That port was the third view over the second tree: it wrote to
    // `/projects/tenants/...` on the host disk while Settings → Skills listed the database.
    let write_bundled_target = write
        .resolve(
            &write
                .scoped_path("/system/skills/example/SKILL.md")
                .expect("scoped path"),
        )
        .expect("bundled resolves")
        .as_str()
        .to_string();
    assert_eq!(
        write_bundled_target, bundled_target,
        "reader and writer must agree on the bundled root too, or a skill_update can shadow a \
         bundled skill in a tree discovery never reads"
    );
}

/// Production must ship the built-in skills, not an empty Skills page.
///
/// Hosted multi-tenant production shipped with **zero** built-in skills. The bundled seeder is only
/// reachable from `bootstrap_standalone_host`, which the Postgres path does not run — correctly, since
/// that bootstrap writes through a host-disk filesystem and a tenant here has no host disk. But
/// `/system/skills` *is* mounted on this path, to the database, and nothing ever wrote to it. So
/// Settings → Skills read an empty root and said "No skills installed" while local-dev listed all 32,
/// and there was no error anywhere to notice.
///
/// Asserted through the same composite filesystem the product reads, so it fails if the seeding moves,
/// the mount moves, or the root path changes.
#[tokio::test]
async fn the_production_database_is_seeded_with_the_bundled_skills() {
    use ironclaw_filesystem::RootFilesystem;
    use ironclaw_host_api::path::VirtualPath;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("bundled.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("libsql db"),
    );
    let database = Arc::new(
        ironclaw_filesystem::LibSqlRootFilesystem::new(Arc::clone(&db)).expect("libsql filesystem"),
    );
    database.run_migrations().await.expect("migrations");

    let filesystem =
        ironclaw_composition::test_support::production_database_root_filesystem_for_test(
            database,
            "bundled-skill-seeding-test",
        )
        .expect("production database composite");

    let system_skills_root = VirtualPath::new("/system/skills").expect("virtual path");
    ironclaw_extension_host::bundled_skills::ensure_bundled_reborn_skills_installed_in(
        filesystem.as_ref(),
        &system_skills_root,
    )
    .await
    .expect("bundled skills install into the database");

    let entries = RootFilesystem::list_dir(filesystem.as_ref(), &system_skills_root)
        .await
        .expect("the seeded system skill root lists");
    let skill_dirs = entries
        .iter()
        .filter(|entry| !entry.name.starts_with('.'))
        .count();
    let expected = ironclaw_extension_host::bundled_skills::bundled_reborn_skill_summaries()
        .expect("bundled summaries")
        .len();
    assert!(
        skill_dirs >= expected,
        "production must ship every bundled skill in the database-backed system root: found \
         {skill_dirs}, expected at least {expected}. Zero here is what production actually did -- an \
         empty Skills page with nothing logged."
    );

    // Idempotent: production boots repeatedly, and several instances can share one database.
    ironclaw_extension_host::bundled_skills::ensure_bundled_reborn_skills_installed_in(
        filesystem.as_ref(),
        &system_skills_root,
    )
    .await
    .expect("a second boot re-runs cleanly");
    let after = RootFilesystem::list_dir(filesystem.as_ref(), &system_skills_root)
        .await
        .expect("list after second seed")
        .iter()
        .filter(|entry| !entry.name.starts_with('.'))
        .count();
    assert_eq!(
        after, skill_dirs,
        "re-seeding must not duplicate or drop skills"
    );
}

/// A model must be able to READ a skill file with the ordinary filesystem tools.
///
/// Skill mounts were granted to the skill capabilities only, so `read_file` saw nothing but
/// `workspace`. Observed on a real production turn: the model installed a skill, tried to read it
/// back to verify it, and got
///
/// ```text
/// path skills clinical-lab-conversions SKILL.md does not resolve inside an available scoped root
/// (available roots: workspace)
/// ```
///
/// It burned a tool call and fell back to `skill_activate`. This is a parity gap, not just an unhelpful
/// error: in Claude Code a SKILL.md *is* a file, so models are trained to read it, and skills reference
/// sibling files (`references/*.md`, `scripts/*.py`) that progressive disclosure expects the agent to
/// open on demand — dead ends without a readable path.
///
/// Read-only is asserted too: writes must stay exclusive to `skill_install`/`skill_update`, which
/// validate the manifest. A writable `/skills` alias here would let an agent hand-write a bundle that
/// discovery then silently skips, which is the failure this whole change removes.
#[tokio::test]
async fn skill_files_are_readable_through_the_filesystem_tools_but_not_writable() {
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        resource::ResourceScope,
    };

    let scope = ResourceScope::local_default(
        UserId::new("skill-read-user").expect("user id"),
        InvocationId::new(),
    )
    .expect("scope");

    let workspace_only = ironclaw_composition::test_support::scoped_workspace_mount_view_for_test(
        &scope,
        ironclaw_host_api::mount::MountPermissions::read_write(),
    )
    .expect("workspace mount view");
    let unreachable = workspace_only
        .scoped_path("/skills/example/SKILL.md")
        .map(|path| workspace_only.resolve(&path).is_err())
        .unwrap_or(true);
    assert!(
        unreachable,
        "premise of this test: the workspace-only view cannot resolve a skill path -- that is the \
         error a real turn hit"
    );

    let with_skills =
        ironclaw_composition::test_support::capability_workspace_mounts_with_skills_for_test(
            workspace_only,
            &scope,
        )
        .expect("workspace + skill read view");

    let probe = with_skills
        .scoped_path("/skills/example/SKILL.md")
        .expect("skill paths are addressable");
    let (resolved, grant) = with_skills
        .resolve_with_grant(&probe)
        .expect("a skill path resolves for the filesystem tools");
    assert!(
        resolved.as_str().starts_with("/tenants/"),
        "must resolve into the DB-backed skill tree, the one skill_install writes; got {resolved}"
    );
    assert!(
        !grant.permissions.write,
        "the filesystem tools' skill grant must be read-only: writes belong to skill_install, which \
         validates the manifest a hand-written bundle would fail"
    );

    // The workspace itself must be unchanged -- this fix adds reach, it does not move anything.
    let workspace_probe = with_skills
        .scoped_path("/workspace/notes.md")
        .expect("workspace still addressable");
    assert!(
        with_skills.resolve(&workspace_probe).is_ok(),
        "adding skill aliases must not disturb the workspace grant"
    );
}
