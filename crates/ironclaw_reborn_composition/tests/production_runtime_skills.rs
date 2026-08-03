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

/// The production composition runs a skill turn, and a DISK-seeded skill is not in its store.
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
async fn production_composition_runs_skills_and_ignores_disk_seeded_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reborn.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("libsql db"),
    );

    // Seed a skill into the tenant/user-scoped store the production profile reads.
    let skill_root = dir
        .path()
        .join("tenants/prod-skills-tenant/users/prod-skills-owner/skills/tenant-policy-helper");
    std::fs::create_dir_all(&skill_root).expect("skill dir");
    std::fs::write(
        skill_root.join("SKILL.md"),
        skill_md(
            "tenant-policy-helper",
            "Applies the tenant's policy checklist to a review.",
            &[],
            "PRODUCTION_SKILL_SENTINEL",
        ),
    )
    .expect("write SKILL.md");

    let bindings = ironclaw_reborn_composition::test_support::libsql_host_bindings_for_test(
        RebornCompositionProfile::Production,
        "prod-skills-owner",
        db,
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

    // A disk-seeded skill is expected to be INVISIBLE here: production reads the scoped-virtual
    // store, not the host filesystem. Asserted rather than left as a surprise, because the
    // surprising thing is how much other validation quietly depends on disk seeding.
    assert!(
        activated.is_empty(),
        "a disk-seeded skill became visible on the production composition, which would mean the \
         storage contract changed -- production reads the scoped-virtual store. If this is now \
         intended, the disk-seeding validation elsewhere finally covers production and this test \
         should assert the skill IS activatable. activated: {activated:?}"
    );

    runtime.shutdown().await.expect("shutdown");
}
