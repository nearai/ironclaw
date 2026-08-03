use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer, SigningKey};
use ironclaw_extensions::{ExtensionInstallationStorePort, InstallationOwner};
use ironclaw_filesystem::{Fault, FaultInjecting, FilesystemOperation, InMemoryBackend};
use ironclaw_host_api::{
    action::NetworkPolicy,
    http::{
        RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
        RuntimeHttpEgressResponse,
    },
    ids::{CapabilityId, ExtensionId, UserId},
    path::VirtualPath,
    resource::ResourceScope,
    runtime::RuntimeKind,
};
use ironclaw_skills::ManagedSkillSource;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use super::catalog::{classify_gate_and_digest, sha256_hex, verify_signed_manifest_with_keys};
use super::model::{
    IronHubArtifact, IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubInstallOptions,
    IronHubManifest, IronHubPhase, IronHubProvenance, IronHubSkillEntry,
};
use super::service::{
    IronHubService, clear_test_manifest_cache, configure_test_catalog, test_install_lock_exists,
    test_manifest_fetch_lock_exists,
};

const TOOL_RESULT_PREVIEW_BUDGET_BYTES: usize = 24 * 1024;

#[test]
fn signed_catalog_verification_accepts_only_the_selected_key() {
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let manifest = br#"{"version":"1"}"#;
    let signature = signing_key.sign(manifest);
    let envelope = serde_json::json!({
        "v": 1,
        "key_id": "test-key",
        "manifest_b64": URL_SAFE_NO_PAD.encode(manifest),
        "sig": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
    .to_string();
    let verify_key = hex::encode(signing_key.verifying_key().to_bytes());

    let verified =
        verify_signed_manifest_with_keys(envelope.as_bytes(), &[("test-key", &verify_key)])
            .expect("selected key verifies the envelope");
    assert_eq!(verified, manifest);
    assert!(
        verify_signed_manifest_with_keys(envelope.as_bytes(), &[("other-key", &verify_key)])
            .is_err()
    );
}

#[test]
fn unverified_entry_requires_non_model_operator_acknowledgement() {
    let manifest = IronHubManifest {
        version: "1".to_string(),
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        release_tag: "test".to_string(),
        repo: "nearai/ironhub".to_string(),
        tools: Vec::new(),
        skills: vec![IronHubSkillEntry {
            name: "community-skill".to_string(),
            trunk: String::new(),
            version: "0.1.0".to_string(),
            description: String::new(),
            provenance: IronHubProvenance::New,
            skill_md: IronHubArtifact {
                url: "https://hub.ironclaw.com/community-skill/SKILL.md".to_string(),
                size_bytes: 10,
                sha256: "a".repeat(64),
            },
        }],
    };

    let denied = classify_gate_and_digest(
        &manifest,
        "community-skill",
        Some(IronHubEntryKind::Skill),
        &IronHubInstallOptions::default(),
    )
    .expect_err("unverified content requires acknowledgement");
    assert!(denied.to_string().contains("UNVERIFIED community"));

    classify_gate_and_digest(
        &manifest,
        "community-skill",
        Some(IronHubEntryKind::Skill),
        &IronHubInstallOptions {
            acknowledge_unverified: true,
            ..IronHubInstallOptions::default()
        },
    )
    .expect("operator acknowledgement permits install");
}

/// A query that matches a SUBSET must report the catalog-wide total alongside the
/// matched count, so a filtered page cannot be read as the whole catalog.
///
/// Regression for the live incident behind #6821: asked what was installable, the
/// agent searched "tool", got back only the entries whose descriptions contain that
/// word, and reported 3 tools when the signed catalog held 18.
#[tokio::test]
async fn execute_search_reports_the_catalog_total_alongside_a_filtered_match_count() {
    let description = "an integration for records and reports".to_string();
    let (service, all_names) = catalog_test_service(
        "filtered-total",
        "ironhub-filtered-total-owner",
        18,
        42,
        &description,
    )
    .await;

    // "zz-final-skill" is the only entry whose name carries this token, so the
    // match is a strict, non-empty subset of the catalog.
    let response = service
        .execute(IronHubCommand::Search {
            query: "zz-final".to_string(),
        })
        .await
        .expect("filtered catalog search succeeds");

    assert!(
        response.returned_entries < all_names.len(),
        "fixture must produce a strict subset, got {} of {}",
        response.returned_entries,
        all_names.len()
    );
    assert_eq!(
        response.total_entries, response.returned_entries,
        "total_entries reports how many entries MATCHED"
    );
    assert_eq!(
        response.catalog_total,
        Some(all_names.len()),
        "a filtered result must still report the full catalog size, so the caller \
         cannot mistake the matched subset for the entire catalog"
    );
    assert!(!response.truncated);

    // The wire payload the model sees must carry it too, not just the Rust struct.
    let payload = serde_json::to_value(&response).expect("response serializes");
    assert_eq!(
        payload["catalog_total"],
        serde_json::json!(all_names.len()),
        "catalog_total must reach the model-visible payload"
    );
}

#[tokio::test]
async fn execute_search_and_list_return_the_complete_catalog_in_a_compact_payload() {
    let description = "long signed catalog description ".repeat(20);
    let (service, expected_names) = catalog_test_service(
        "compact-complete",
        "ironhub-compact-complete-owner",
        18,
        42,
        &description,
    )
    .await;
    let legacy_payload = serde_json::json!({
        "phase": "discovered",
        "entries": expected_names
            .iter()
            .map(|name| serde_json::json!({
                "kind": "tool",
                "name": name,
                "version": "0.1.0",
                "description": description,
                "provenance": "official",
                "artifact_digest": "a".repeat(64),
            }))
            .collect::<Vec<_>>(),
    });
    assert!(
        serde_json::to_vec(&legacy_payload)
            .expect("legacy payload serializes")
            .len()
            > TOOL_RESULT_PREVIEW_BUDGET_BYTES,
        "fixture must reproduce the pre-fix result-reference truncation"
    );

    for command in [
        IronHubCommand::Search {
            query: String::new(),
        },
        IronHubCommand::List { kind: None },
    ] {
        let response = service
            .execute(command)
            .await
            .expect("signed catalog query succeeds");
        let payload = serde_json::to_value(&response).expect("response serializes");
        let serialized = serde_json::to_vec(&payload).expect("response bytes serialize");
        let returned_names = response
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(response.total_entries, expected_names.len());
        assert_eq!(response.returned_entries, expected_names.len());
        assert!(!response.truncated);
        assert_eq!(
            returned_names,
            expected_names
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        assert!(
            returned_names.contains(&"zz-final-skill"),
            "the alphabetically last catalog entry must be present"
        );
        assert!(
            response.entries.iter().all(|entry| {
                entry.description.len() <= 120 && entry.description.ends_with('…')
            }),
            "search/list descriptions must be explicitly shortened to at most 120 bytes"
        );
        assert!(
            response
                .entries
                .iter()
                .all(|entry| entry.provenance == IronHubProvenance::Official),
            "compact catalog entries must retain provenance for trust gating"
        );
        assert!(
            payload["entries"]
                .as_array()
                .expect("entries are an array")
                .iter()
                .all(|entry| entry.get("artifact_digest").is_none()),
            "full artifact digests belong to ironhub_info, not catalog listings"
        );
        assert!(
            serialized.len() <= TOOL_RESULT_PREVIEW_BUDGET_BYTES,
            "complete catalog payload is {} bytes",
            serialized.len()
        );
    }

    let info = service
        .execute(IronHubCommand::Info {
            name: "zz-final-skill".to_string(),
            kind: Some(IronHubEntryKind::Skill),
        })
        .await
        .expect("full entry detail remains available");
    assert_eq!(info.entries[0].description, description);
    assert!(
        info.entries[0].artifact_digest.is_some(),
        "ironhub_info retains the signed artifact digest"
    );
}

#[tokio::test]
async fn execute_search_marks_an_oversized_catalog_as_incomplete_with_the_true_total() {
    let description = "oversized signed catalog description ".repeat(20);
    let (service, expected_names) = catalog_test_service(
        "compact-truncated",
        "ironhub-compact-truncated-owner",
        120,
        120,
        &description,
    )
    .await;

    let response = service
        .execute(IronHubCommand::Search {
            query: String::new(),
        })
        .await
        .expect("signed catalog search succeeds");
    let serialized = serde_json::to_vec(&response).expect("response serializes");
    let message = response
        .message
        .as_deref()
        .expect("incomplete response carries a model-visible warning");

    assert_eq!(response.total_entries, expected_names.len());
    assert_eq!(response.returned_entries, response.entries.len());
    assert!(response.returned_entries < response.total_entries);
    assert!(response.truncated);
    // The truncated path must carry catalog_total too, and it must be part of the
    // shape the byte budget was measured against — assigning it after the
    // size loop made the emitted payload larger than the budget that admitted it.
    assert_eq!(
        response.catalog_total,
        Some(expected_names.len()),
        "a truncated result must still report the full catalog size"
    );
    assert_eq!(
        serde_json::to_value(&response).expect("response serializes")["catalog_total"],
        serde_json::json!(expected_names.len()),
        "catalog_total must be present in the measured, emitted payload"
    );
    assert!(
        message.contains("INCOMPLETE") && message.contains(&expected_names.len().to_string()),
        "warning must state that the result is incomplete and report the true total: {message}"
    );
    assert!(
        serialized.len() <= TOOL_RESULT_PREVIEW_BUDGET_BYTES,
        "bounded incomplete response is {} bytes",
        serialized.len()
    );
}

#[tokio::test]
async fn verified_tool_and_skill_install_through_real_managers() {
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services("ironhub-owner", None, false)
            .await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner("ironhub-owner");
    let manifest_url = "https://hub.ironclaw.com/tests/native-install/manifest.json";
    let tool_url = "https://hub.ironclaw.com/tests/native-install/tool.wasm";
    let capabilities_url = "https://hub.ironclaw.com/tests/native-install/capabilities.json";
    let skill_url = "https://hub.ironclaw.com/tests/native-install/SKILL.md";
    let tool_bytes =
        include_bytes!("../../../extensions/packages/github/wasm/github_tool.wasm").to_vec();
    let capabilities_bytes = br#"{"capabilities":[]}"#.to_vec();
    let skill_bytes =
        b"---\nname: installed-skill\ndescription: Installed by IronHub\n---\n# Installed\n"
            .to_vec();
    let manifest = signed_manifest(
        mixed_manifest_json(MixedManifestFixture {
            tool_url,
            tool_size: tool_bytes.len(),
            tool_sha: &sha256_hex(&tool_bytes),
            capabilities_url,
            capabilities_size: capabilities_bytes.len(),
            capabilities_sha: &sha256_hex(&capabilities_bytes),
            skill_url,
            skill_size: skill_bytes.len(),
            skill_sha: &sha256_hex(&skill_bytes),
        }),
        &test_signing_key(),
    );
    let egress = Arc::new(RecordingEgress::new([
        (manifest_url, manifest),
        (tool_url, tool_bytes),
        (capabilities_url, capabilities_bytes),
        (skill_url, skill_bytes),
    ]));
    let service = configure_test_catalog(
        IronHubService::new_with_runtime_egress(
            Arc::clone(&services.skill_management),
            Arc::clone(&services.extension_management),
            egress.clone(),
            scope.clone(),
            CapabilityId::new(super::IRONHUB_INSTALL_CAPABILITY_ID).expect("capability id"),
        ),
        manifest_url,
        test_manifest_verify_keys(),
    );

    let tool = service
        .execute(IronHubCommand::Install {
            name: "installed-tool".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Tool),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect("verified tool installs");
    assert_eq!(tool.phase, IronHubPhase::Installed);
    let manifest_path =
        VirtualPath::new("/system/extensions/installed-tool/manifest.toml").expect("path");
    let materialized = services
        .filesystem
        .read_file(&manifest_path)
        .await
        .expect("tool manifest materialized");
    assert!(
        String::from_utf8(materialized)
            .expect("manifest utf8")
            .contains("reborn.extension_manifest.v3")
    );
    assert!(
        services
            .extension_management
            .installation_store_handle()
            .get_installation(
                &ironclaw_extensions::ExtensionInstallationId::new("installed-tool")
                    .expect("installation id")
            )
            .await
            .expect("installation read")
            .is_some(),
        "extension manager persisted the installation record"
    );
    assert!(
        services
            .extension_management
            .active_extensions_for_test()
            .snapshot()
            .get_extension(&ExtensionId::new("installed-tool").expect("extension id"))
            .is_some(),
        "extension manager activated and published the installed tool"
    );

    let skill = service
        .execute(IronHubCommand::Install {
            name: "installed-skill".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Skill),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect("verified skill installs");
    assert_eq!(skill.phase, IronHubPhase::Installed);
    let installed_skill = services
        .skill_management
        .read_content_for_scope(scope, "installed-skill")
        .await
        .expect("skill manager reads installed skill");
    assert!(installed_skill.content.contains("# Installed"));

    let requests = egress.requests();
    assert_eq!(requests.len(), 4);
    assert!(requests.iter().all(|request| {
        request.runtime == RuntimeKind::FirstParty
            && request.policy.deny_private_ip_ranges
            && request.capability_id.as_str() == super::IRONHUB_INSTALL_CAPABILITY_ID
    }));
}

#[tokio::test]
async fn forced_tool_replacement_failure_restores_previous_package() {
    let (services, _scope, error) = fail_forced_tool_replacement("tool-rollback", false).await;

    assert!(matches!(error, IronHubCommandError::Product(_)));
    let manifest_path =
        VirtualPath::new("/system/extensions/installed-tool/manifest.toml").expect("path");
    let restored_manifest = services
        .filesystem
        .read_file(&manifest_path)
        .await
        .expect("previous manifest restored");
    assert!(
        String::from_utf8(restored_manifest)
            .expect("manifest utf8")
            .contains("version = \"0.1.0\"")
    );
    let active = services
        .extension_management
        .active_extensions_for_test()
        .snapshot();
    assert!(
        active
            .get_extension(&ExtensionId::new("installed-tool").expect("extension id"))
            .is_some(),
        "previous tool is active after replacement compensation"
    );
}

#[tokio::test]
async fn forced_tool_replacement_failure_preserves_tenant_shared_scope() {
    let (services, _scope, error) =
        fail_forced_tool_replacement("tenant-scope-rollback", true).await;

    assert!(matches!(error, IronHubCommandError::Product(_)));
    let installation = services
        .extension_management
        .installation_store_handle()
        .get_installation(
            &ironclaw_extensions::ExtensionInstallationId::new("installed-tool")
                .expect("installation id"),
        )
        .await
        .expect("installation read")
        .expect("previous installation restored");
    assert_eq!(installation.owner(), &InstallationOwner::Tenant);
}

#[tokio::test]
async fn forced_skill_replacement_failure_restores_url_source() {
    let services = crate::lifecycle_test_support::build_lifecycle_test_services(
        "ironhub-skill-rollback-owner",
        None,
        false,
    )
    .await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(
        "ironhub-skill-rollback-owner",
    );
    let skill_filesystem = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let skill_management = ironclaw_skills::build_scoped_skill_management_port(
        UserId::new("ironhub-skill-rollback-owner").expect("owner id"),
        skill_filesystem.clone(),
    );
    let old_manifest_url = "https://hub.ironclaw.com/tests/skill-rollback/old-manifest.json";
    let old_skill_url = "https://hub.ironclaw.com/tests/skill-rollback/old-SKILL.md";
    let old_skill =
        b"---\nname: installed-skill\ndescription: Old IronHub skill\n---\n# Old\n".to_vec();
    let old_manifest = signed_manifest(
        skill_manifest_json(
            "installed-skill",
            "2026-01-03T00:00:00Z",
            "0.1.0",
            old_skill_url,
            old_skill.len(),
            &sha256_hex(&old_skill),
        ),
        &test_signing_key(),
    );
    let old_egress = Arc::new(RecordingEgress::new([
        (old_manifest_url, old_manifest),
        (old_skill_url, old_skill.clone()),
    ]));
    configured_service(
        Arc::clone(&skill_management),
        Arc::clone(&services.extension_management),
        old_egress,
        scope.clone(),
        old_manifest_url,
    )
    .execute(install_command(IronHubEntryKind::Skill, false))
    .await
    .expect("old skill installs through execute");

    skill_filesystem.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path(".ironclaw-install.json")
            .nth(1)
            .backend("injected replacement metadata failure"),
    );
    let new_manifest_url = "https://hub.ironclaw.com/tests/skill-rollback/new-manifest.json";
    let new_skill_url = "https://hub.ironclaw.com/tests/skill-rollback/new-SKILL.md";
    let new_skill =
        b"---\nname: installed-skill\ndescription: New IronHub skill\n---\n# New\n".to_vec();
    let new_manifest = signed_manifest(
        skill_manifest_json(
            "installed-skill",
            "2026-01-04T00:00:00Z",
            "0.2.0",
            new_skill_url,
            new_skill.len(),
            &sha256_hex(&new_skill),
        ),
        &test_signing_key(),
    );
    let new_egress = Arc::new(RecordingEgress::new([
        (new_manifest_url, new_manifest),
        (new_skill_url, new_skill),
    ]));
    let error = configured_service(
        Arc::clone(&skill_management),
        Arc::clone(&services.extension_management),
        new_egress,
        scope.clone(),
        new_manifest_url,
    )
    .execute(install_command(IronHubEntryKind::Skill, true))
    .await
    .expect_err("injected replacement failure reaches compensation");

    assert!(matches!(error, IronHubCommandError::Install { .. }));
    let restored = skill_management
        .read_content_for_scope(scope.clone(), "installed-skill")
        .await
        .expect("restored skill is readable");
    assert_eq!(restored.content.as_bytes(), old_skill);
    assert_eq!(restored.source, ManagedSkillSource::Installed);
    assert_eq!(restored.source_url.as_deref(), Some(old_skill_url));
    let listed = skill_management
        .list_for_scope(scope)
        .await
        .expect("restored skill is listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].source, ManagedSkillSource::Installed);
}

#[tokio::test]
async fn execute_rejects_artifact_size_and_sha256_mismatches() {
    let services = crate::lifecycle_test_support::build_lifecycle_test_services(
        "ironhub-artifact-owner",
        None,
        false,
    )
    .await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(
        "ironhub-artifact-owner",
    );
    let skill_bytes =
        b"---\nname: artifact-skill\ndescription: Artifact checks\n---\n# Skill\n".to_vec();

    let size_skill_name = "ironhub-lock-eviction-size-artifact-skill";
    let size_manifest_url = "https://hub.ironclaw.com/tests/lock-eviction-size/size-manifest.json";
    let size_skill_url = "https://hub.ironclaw.com/tests/lock-eviction-size/size-SKILL.md";
    let size_manifest = signed_manifest(
        skill_manifest_json(
            size_skill_name,
            "2026-01-05T00:00:00Z",
            "0.1.0",
            size_skill_url,
            skill_bytes.len() + 1,
            &sha256_hex(&skill_bytes),
        ),
        &test_signing_key(),
    );
    let size_error = configured_service(
        Arc::clone(&services.skill_management),
        Arc::clone(&services.extension_management),
        Arc::new(RecordingEgress::new([
            (size_manifest_url, size_manifest),
            (size_skill_url, skill_bytes.clone()),
        ])),
        scope.clone(),
        size_manifest_url,
    )
    .execute(install_named_command(
        size_skill_name,
        IronHubEntryKind::Skill,
        false,
    ))
    .await
    .expect_err("artifact size mismatch is rejected");
    assert!(matches!(
        size_error,
        IronHubCommandError::Install { reason } if reason.contains("size mismatch")
    ));
    assert!(!test_manifest_fetch_lock_exists(size_manifest_url));
    assert!(!test_install_lock_exists(
        "skill:ironhub-lock-eviction-size-artifact-skill"
    ));

    let sha_skill_name = "ironhub-lock-eviction-sha-artifact-skill";
    let sha_manifest_url = "https://hub.ironclaw.com/tests/lock-eviction-sha/sha-manifest.json";
    let sha_skill_url = "https://hub.ironclaw.com/tests/lock-eviction-sha/sha-SKILL.md";
    let sha_manifest = signed_manifest(
        skill_manifest_json(
            sha_skill_name,
            "2026-01-06T00:00:00Z",
            "0.1.0",
            sha_skill_url,
            skill_bytes.len(),
            &"0".repeat(64),
        ),
        &test_signing_key(),
    );
    let sha_error = configured_service(
        Arc::clone(&services.skill_management),
        Arc::clone(&services.extension_management),
        Arc::new(RecordingEgress::new([
            (sha_manifest_url, sha_manifest),
            (sha_skill_url, skill_bytes),
        ])),
        scope,
        sha_manifest_url,
    )
    .execute(install_named_command(
        sha_skill_name,
        IronHubEntryKind::Skill,
        false,
    ))
    .await
    .expect_err("artifact checksum mismatch is rejected");
    assert!(matches!(
        sha_error,
        IronHubCommandError::Install { reason } if reason.contains("checksum mismatch")
    ));
    assert!(!test_manifest_fetch_lock_exists(sha_manifest_url));
    assert!(!test_install_lock_exists(
        "skill:ironhub-lock-eviction-sha-artifact-skill"
    ));
}

#[tokio::test]
async fn execute_rejects_older_generated_at_after_cache_eviction() {
    let services = crate::lifecycle_test_support::build_lifecycle_test_services(
        "ironhub-replay-owner",
        None,
        false,
    )
    .await;
    let scope =
        crate::lifecycle_test_support::webui_gate_resource_scope_for_owner("ironhub-replay-owner");
    let manifest_url = "https://hub.ironclaw.com/tests/replay/manifest.json";
    let newer = signed_manifest(
        empty_manifest_json("2026-01-08T00:00:00Z"),
        &test_signing_key(),
    );
    let older = signed_manifest(
        empty_manifest_json("2026-01-07T00:00:00Z"),
        &test_signing_key(),
    );
    let service = configured_service(
        Arc::clone(&services.skill_management),
        Arc::clone(&services.extension_management),
        Arc::new(RecordingEgress::new([
            (manifest_url, newer),
            (manifest_url, older),
        ])),
        scope,
        manifest_url,
    );

    service
        .execute(IronHubCommand::List { kind: None })
        .await
        .expect("newer manifest is accepted");
    clear_test_manifest_cache(manifest_url);
    let error = service
        .execute(IronHubCommand::List { kind: None })
        .await
        .expect_err("older signed manifest is rejected");

    assert!(matches!(
        error,
        IronHubCommandError::Catalog { reason }
            if reason.contains("signed manifest replay rejected")
    ));
}

async fn fail_forced_tool_replacement(
    fixture: &str,
    tenant_shared: bool,
) -> (
    crate::lifecycle_test_support::ExtensionLifecycleTestServices,
    ResourceScope,
    IronHubCommandError,
) {
    let owner = format!("ironhub-{fixture}-owner");
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services(&owner, None, false).await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(&owner);
    let tool_bytes =
        include_bytes!("../../../extensions/packages/github/wasm/github_tool.wasm").to_vec();
    let capabilities_bytes = br#"{"capabilities":[]}"#.to_vec();
    let old_manifest_url = format!("https://hub.ironclaw.com/tests/{fixture}/old-manifest.json");
    let old_tool_url = format!("https://hub.ironclaw.com/tests/{fixture}/old-tool.wasm");
    let old_capabilities_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/old-capabilities.json");
    let old_manifest = signed_manifest(
        tool_manifest_json(ToolManifestFixture {
            generated_at: "2026-01-03T00:00:00Z",
            version: "0.1.0",
            tool_url: &old_tool_url,
            tool_size: tool_bytes.len(),
            tool_sha: &sha256_hex(&tool_bytes),
            capabilities_url: &old_capabilities_url,
            capabilities_size: capabilities_bytes.len(),
            capabilities_sha: &sha256_hex(&capabilities_bytes),
        }),
        &test_signing_key(),
    );
    configured_service(
        Arc::clone(&services.skill_management),
        Arc::clone(&services.extension_management),
        Arc::new(RecordingEgress::new([
            (old_manifest_url.as_str(), old_manifest),
            (old_tool_url.as_str(), tool_bytes.clone()),
            (old_capabilities_url.as_str(), capabilities_bytes.clone()),
        ])),
        scope.clone(),
        &old_manifest_url,
    )
    .execute(install_command(IronHubEntryKind::Tool, false))
    .await
    .expect("old tool installs through execute");

    if tenant_shared {
        let store = services.extension_management.installation_store_handle();
        let installation_id = ironclaw_extensions::ExtensionInstallationId::new("installed-tool")
            .expect("installation id");
        let installation = store
            .get_installation(&installation_id)
            .await
            .expect("installation read")
            .expect("old installation exists");
        store
            .upsert_installation(installation.with_owner(InstallationOwner::Tenant))
            .await
            .expect("tenant-shared compatibility owner persisted");
    }

    services.add_filesystem_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/system/extensions/installed-tool/manifest.toml")
            .nth(1)
            .backend("injected replacement materialization failure"),
    );
    let new_manifest_url = format!("https://hub.ironclaw.com/tests/{fixture}/new-manifest.json");
    let new_tool_url = format!("https://hub.ironclaw.com/tests/{fixture}/new-tool.wasm");
    let new_capabilities_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/new-capabilities.json");
    let new_manifest = signed_manifest(
        tool_manifest_json(ToolManifestFixture {
            generated_at: "2026-01-04T00:00:00Z",
            version: "0.2.0",
            tool_url: &new_tool_url,
            tool_size: tool_bytes.len(),
            tool_sha: &sha256_hex(&tool_bytes),
            capabilities_url: &new_capabilities_url,
            capabilities_size: capabilities_bytes.len(),
            capabilities_sha: &sha256_hex(&capabilities_bytes),
        }),
        &test_signing_key(),
    );
    let error = configured_service(
        Arc::clone(&services.skill_management),
        Arc::clone(&services.extension_management),
        Arc::new(RecordingEgress::new([
            (new_manifest_url.as_str(), new_manifest),
            (new_tool_url.as_str(), tool_bytes),
            (new_capabilities_url.as_str(), capabilities_bytes),
        ])),
        scope.clone(),
        &new_manifest_url,
    )
    .execute(install_command(IronHubEntryKind::Tool, true))
    .await
    .expect_err("injected replacement failure reaches compensation");

    (services, scope, error)
}

fn configured_service(
    skill_management: Arc<ironclaw_skills::ScopedSkillManagementPort>,
    extension_management: Arc<ironclaw_extension_host::ExtensionLifecycleManager>,
    egress: Arc<RecordingEgress>,
    scope: ResourceScope,
    manifest_url: &str,
) -> IronHubService {
    configure_test_catalog(
        IronHubService::new_with_runtime_egress(
            skill_management,
            extension_management,
            egress,
            scope,
            CapabilityId::new(super::IRONHUB_INSTALL_CAPABILITY_ID).expect("capability id"),
        ),
        manifest_url,
        test_manifest_verify_keys(),
    )
}

fn install_command(kind: IronHubEntryKind, force: bool) -> IronHubCommand {
    install_named_command(
        match kind {
            IronHubEntryKind::Tool => "installed-tool",
            IronHubEntryKind::Skill => "installed-skill",
        },
        kind,
        force,
    )
}

fn install_named_command(name: &str, kind: IronHubEntryKind, force: bool) -> IronHubCommand {
    IronHubCommand::Install {
        name: name.to_string(),
        options: IronHubInstallOptions {
            kind: Some(kind),
            force,
            ..IronHubInstallOptions::default()
        },
    }
}

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

fn test_manifest_verify_keys() -> &'static [(&'static str, &'static str)] {
    let verify_key = hex::encode(test_signing_key().verifying_key().to_bytes());
    let verify_key = Box::leak(verify_key.into_boxed_str());
    Box::leak(vec![("ironhub-test-key", verify_key as &str)].into_boxed_slice())
}

fn signed_manifest(manifest_json: String, signing_key: &SigningKey) -> Vec<u8> {
    let signature = signing_key.sign(manifest_json.as_bytes());
    serde_json::json!({
        "v": 1,
        "key_id": "ironhub-test-key",
        "manifest_b64": URL_SAFE_NO_PAD.encode(manifest_json.as_bytes()),
        "sig": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
    .to_string()
    .into_bytes()
}

async fn catalog_test_service(
    fixture: &str,
    owner: &str,
    tool_count: usize,
    skill_count: usize,
    description: &str,
) -> (IronHubService, Vec<String>) {
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services(owner, None, false).await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(owner);
    let manifest_url = format!("https://hub.ironclaw.com/tests/{fixture}/manifest.json");
    let (manifest_json, expected_names) =
        catalog_manifest_json(fixture, tool_count, skill_count, description);
    let manifest = signed_manifest(manifest_json, &test_signing_key());
    let service = configure_test_catalog(
        IronHubService::new_with_runtime_egress(
            services.skill_management,
            services.extension_management,
            Arc::new(RecordingEgress::new([(manifest_url.as_str(), manifest)])),
            scope,
            CapabilityId::new(super::IRONHUB_SEARCH_CAPABILITY_ID).expect("capability id"),
        ),
        manifest_url,
        test_manifest_verify_keys(),
    );
    (service, expected_names)
}

fn catalog_manifest_json(
    fixture: &str,
    tool_count: usize,
    skill_count: usize,
    description: &str,
) -> (String, Vec<String>) {
    let tools = (0..tool_count)
        .map(|index| {
            let name = format!("tool-{index:03}");
            serde_json::json!({
                "name": name,
                "crate_name": name,
                "version": "0.1.0",
                "description": description,
                "provenance": "official",
                "wasm": {
                    "url": format!("https://hub.ironclaw.com/tests/{fixture}/{name}.wasm"),
                    "size_bytes": 1,
                    "sha256": "a".repeat(64),
                },
                "capabilities": {
                    "url": format!("https://hub.ironclaw.com/tests/{fixture}/{name}.json"),
                    "size_bytes": 1,
                    "sha256": "b".repeat(64),
                },
            })
        })
        .collect::<Vec<_>>();
    let skills = (0..skill_count)
        .map(|index| {
            let name = if index + 1 == skill_count {
                "zz-final-skill".to_string()
            } else {
                format!("skill-{index:03}")
            };
            serde_json::json!({
                "name": name,
                "version": "0.1.0",
                "description": description,
                "provenance": "official",
                "skill_md": {
                    "url": format!("https://hub.ironclaw.com/tests/{fixture}/{name}.md"),
                    "size_bytes": 1,
                    "sha256": "c".repeat(64),
                },
            })
        })
        .collect::<Vec<_>>();
    let expected_names = tools
        .iter()
        .chain(&skills)
        .map(|entry| {
            entry["name"]
                .as_str()
                .expect("fixture entry name is a string")
                .to_string()
        })
        .collect();
    (
        serde_json::json!({
            "version": "1",
            "generated_at": "2026-07-28T00:00:00Z",
            "release_tag": "test",
            "repo": "nearai/ironhub",
            "tools": tools,
            "skills": skills,
        })
        .to_string(),
        expected_names,
    )
}

struct MixedManifestFixture<'a> {
    tool_url: &'a str,
    tool_size: usize,
    tool_sha: &'a str,
    capabilities_url: &'a str,
    capabilities_size: usize,
    capabilities_sha: &'a str,
    skill_url: &'a str,
    skill_size: usize,
    skill_sha: &'a str,
}

fn mixed_manifest_json(fixture: MixedManifestFixture<'_>) -> String {
    let MixedManifestFixture {
        tool_url,
        tool_size,
        tool_sha,
        capabilities_url,
        capabilities_size,
        capabilities_sha,
        skill_url,
        skill_size,
        skill_sha,
    } = fixture;
    serde_json::json!({
        "version": "1",
        "generated_at": "2026-01-02T00:00:00Z",
        "release_tag": "test",
        "repo": "nearai/ironhub",
        "tools": [{
            "name": "installed-tool",
            "crate_name": "installed-tool",
            "version": "0.1.0",
            "description": "test tool",
            "provenance": "official",
            "wasm": {
                "url": tool_url,
                "size_bytes": tool_size,
                "sha256": tool_sha
            },
            "capabilities": {
                "url": capabilities_url,
                "size_bytes": capabilities_size,
                "sha256": capabilities_sha
            }
        }],
        "skills": [{
            "name": "installed-skill",
            "version": "0.1.0",
            "description": "test skill",
            "provenance": "official",
            "skill_md": {
                "url": skill_url,
                "size_bytes": skill_size,
                "sha256": skill_sha
            }
        }]
    })
    .to_string()
}

struct ToolManifestFixture<'a> {
    generated_at: &'a str,
    version: &'a str,
    tool_url: &'a str,
    tool_size: usize,
    tool_sha: &'a str,
    capabilities_url: &'a str,
    capabilities_size: usize,
    capabilities_sha: &'a str,
}

fn tool_manifest_json(fixture: ToolManifestFixture<'_>) -> String {
    let ToolManifestFixture {
        generated_at,
        version,
        tool_url,
        tool_size,
        tool_sha,
        capabilities_url,
        capabilities_size,
        capabilities_sha,
    } = fixture;
    serde_json::json!({
        "version": "1",
        "generated_at": generated_at,
        "release_tag": "test",
        "repo": "nearai/ironhub",
        "tools": [{
            "name": "installed-tool",
            "crate_name": "installed-tool",
            "version": version,
            "description": "test tool",
            "provenance": "official",
            "wasm": {
                "url": tool_url,
                "size_bytes": tool_size,
                "sha256": tool_sha
            },
            "capabilities": {
                "url": capabilities_url,
                "size_bytes": capabilities_size,
                "sha256": capabilities_sha
            }
        }],
        "skills": []
    })
    .to_string()
}

fn skill_manifest_json(
    name: &str,
    generated_at: &str,
    version: &str,
    skill_url: &str,
    skill_size: usize,
    skill_sha: &str,
) -> String {
    serde_json::json!({
        "version": "1",
        "generated_at": generated_at,
        "release_tag": "test",
        "repo": "nearai/ironhub",
        "tools": [],
        "skills": [{
            "name": name,
            "version": version,
            "description": "test skill",
            "provenance": "official",
            "skill_md": {
                "url": skill_url,
                "size_bytes": skill_size,
                "sha256": skill_sha
            }
        }]
    })
    .to_string()
}

fn empty_manifest_json(generated_at: &str) -> String {
    serde_json::json!({
        "version": "1",
        "generated_at": generated_at,
        "release_tag": "test",
        "repo": "nearai/ironhub",
        "tools": [],
        "skills": []
    })
    .to_string()
}

#[derive(Clone)]
struct RecordedRequest {
    runtime: RuntimeKind,
    capability_id: CapabilityId,
    policy: NetworkPolicy,
}

struct RecordingEgress {
    responses: Mutex<HashMap<String, VecDeque<Vec<u8>>>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl RecordingEgress {
    fn new<const N: usize>(responses: [(&str, Vec<u8>); N]) -> Self {
        let mut queued = HashMap::<String, VecDeque<Vec<u8>>>::new();
        for (url, body) in responses {
            queued.entry(url.to_string()).or_default().push_back(body);
        }
        Self {
            responses: Mutex::new(queued),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests
            .lock()
            .expect("requests lock")
            .push(RecordedRequest {
                runtime: request.runtime,
                capability_id: request.capability_id.clone(),
                policy: request.network_policy.clone(),
            });
        let body = self
            .responses
            .lock()
            .expect("responses lock")
            .get_mut(&request.url)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| RuntimeHttpEgressError::Request {
                reason: format!("unexpected test URL {}", request.url),
                request_bytes: 0,
                response_bytes: 0,
            })?;
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body,
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        })
    }
}
