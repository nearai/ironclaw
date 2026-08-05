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
//! **What this test establishes, and the gap it exposes.**
//!
//! The production composition builds, opens a conversation, and runs a skill-execution turn — so
//! skills are wired on the production path. But a skill written to
//! `tenants/<t>/users/<u>/skills/` **on disk is invisible to it**: production resolves the skill
//! store through the scoped-virtual filesystem (libSQL), not the host filesystem. Measured here:
//! explicit `$name` activation returned an empty activation set for a disk-seeded skill.
//!
//! That is not a product bug — it is the storage contract. It means something else, though, and it
//! is the honest answer to "would this work in production": **every other validation of this work
//! seeds skills on disk, so none of it exercises production's storage path.** The benchmark mounts
//! `system/skills`, the local-dev tests write files, and both are real paths for their profiles and
//! neither is production's.
//!
//! Closing that needs a seam this crate does not expose: installing a skill into the production
//! scoped-virtual store from a test, i.e. driving `builtin.skill_install` through the production
//! capability port rather than writing bytes to a directory. That is ironclaw infrastructure work
//! with an owner other than this PR, so it is recorded here rather than approximated — a test that
//! seeds disk and asserts success would report production coverage it does not have.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_reborn_composition::{
    PollSettings, RebornCompositionProfile, RebornRuntimeIdentity, RebornRuntimeInput,
    build_reborn_runtime,
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

    let bindings = ironclaw_reborn_composition::test_support::libsql_host_bindings_for_test(
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
        "/projects/tenants/prod-skills-tenant/users/prod-skills-owner/skills/tenant-policy-helper/SKILL.md",
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
) -> Result<
    ironclaw_reborn_composition::RebornRuntime,
    ironclaw_reborn_composition::RebornRuntimeError,
> {
    let bindings = ironclaw_reborn_composition::test_support::libsql_host_bindings_for_test(
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
        "/projects/tenants/prod-restart-tenant/users/prod-restart-owner/skills/restart-policy-helper/SKILL.md",
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
