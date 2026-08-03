use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, KeyInit, Mac};
use ironclaw_extensions::{ExtensionInstallationStorePort, InstallationOwner};
use ironclaw_filesystem::{
    Fault, FaultInjecting, FilesystemOperation, InMemoryBackend, RootFilesystem,
};
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
use ironclaw_product_contracts::ironhub::{
    IronhubInstallDeliveryRequest, IronhubLinkError, IronhubLinkService, IronhubRegisterRequest,
};
use ironclaw_product_contracts::lifecycle_service::LifecycleProductSurfaceContext;
use ironclaw_product_contracts::surface::ProductSurfaceCaller;
use ironclaw_skills::ManagedSkillSource;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use super::agent_link::{InstallDelivery, IronhubSharedKey, RegisterChallenge};
use super::catalog::{
    IronHubManifestSource, classify_gate_and_digest, sha256_hex, validate_manifest,
    validate_private_manifest, validate_private_manifest_origin, verify_signed_manifest_with_keys,
};
use super::link_service::{
    IronhubLinkStateStore, RebornIronhubLinkService, configure_test_manifest_verify_keys,
};
use super::model::{
    IronHubArtifact, IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubInstallOptions,
    IronHubManifest, IronHubPhase, IronHubProvenance, IronHubSkillEntry,
};
use super::service::{
    IronHubService, RebornIronHubRuntime, clear_test_manifest_cache, configure_test_catalog,
    execute_reborn_ironhub_command, execute_reborn_ironhub_service_command, validated_manifest_url,
};

const TOOL_RESULT_PREVIEW_BUDGET_BYTES: usize = 24 * 1024;

fn test_link_state() -> Arc<IronhubLinkStateStore> {
    Arc::new(IronhubLinkStateStore::new(Arc::new(InMemoryBackend::new())))
}

struct MissingEgressRuntime;

impl RebornIronHubRuntime for MissingEgressRuntime {
    fn ironhub_skill_management(&self) -> Arc<ironclaw_skills::ScopedSkillManagementPort> {
        unreachable!("missing egress must fail before skill management is requested")
    }

    fn ironhub_extension_management(
        &self,
    ) -> Arc<ironclaw_extension_host::ExtensionLifecycleManager> {
        unreachable!("missing egress must fail before extension management is requested")
    }

    fn ironhub_runtime_http_egress(&self) -> Option<Arc<dyn RuntimeHttpEgress>> {
        None
    }

    fn ironhub_surface_context(&self) -> LifecycleProductSurfaceContext {
        unreachable!("missing egress must fail before surface context is requested")
    }

    fn ironhub_link_state(&self) -> Arc<IronhubLinkStateStore> {
        unreachable!("missing egress must fail before link state is requested")
    }

    fn ironhub_manifest_url(&self) -> super::IronhubManifestUrl {
        unreachable!("missing egress must fail before manifest URL is requested")
    }
}

struct WiredRuntime {
    skill_management: Arc<ironclaw_skills::ScopedSkillManagementPort>,
    extension_management: Arc<ironclaw_extension_host::ExtensionLifecycleManager>,
    egress: Arc<dyn RuntimeHttpEgress>,
    context: LifecycleProductSurfaceContext,
    state: Arc<IronhubLinkStateStore>,
    manifest_url: super::IronhubManifestUrl,
}

impl RebornIronHubRuntime for WiredRuntime {
    fn ironhub_skill_management(&self) -> Arc<ironclaw_skills::ScopedSkillManagementPort> {
        Arc::clone(&self.skill_management)
    }

    fn ironhub_extension_management(
        &self,
    ) -> Arc<ironclaw_extension_host::ExtensionLifecycleManager> {
        Arc::clone(&self.extension_management)
    }

    fn ironhub_runtime_http_egress(&self) -> Option<Arc<dyn RuntimeHttpEgress>> {
        Some(Arc::clone(&self.egress))
    }

    fn ironhub_surface_context(&self) -> LifecycleProductSurfaceContext {
        self.context.clone()
    }

    fn ironhub_link_state(&self) -> Arc<IronhubLinkStateStore> {
        Arc::clone(&self.state)
    }

    fn ironhub_manifest_url(&self) -> super::IronhubManifestUrl {
        self.manifest_url.clone()
    }
}

#[tokio::test]
async fn reborn_runtime_wrapper_fails_before_requesting_other_services_without_egress() {
    let error =
        execute_reborn_ironhub_command(&MissingEgressRuntime, IronHubCommand::List { kind: None })
            .await
            .expect_err("runtime HTTP egress is required");

    assert!(matches!(
        error,
        IronHubCommandError::RuntimeHttpEgressUnavailable
    ));
}

#[tokio::test]
async fn service_wrapper_routes_catalog_requests_through_the_mediated_egress() {
    let owner = "service-wrapper-owner";
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services(owner, None, false).await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(owner);
    let manifest_url_value = "https://hub.ironclaw.com/tests/service-wrapper/manifest.json";
    clear_test_manifest_cache(manifest_url_value);
    let manifest_url = validated_manifest_url(manifest_url_value).expect("valid manifest URL");
    let egress = Arc::new(RecordingEgress::new([]));
    let state = Arc::new(IronhubLinkStateStore::new(Arc::new(InMemoryBackend::new())));

    let error = execute_reborn_ironhub_service_command(
        services.skill_management,
        services.extension_management,
        egress.clone(),
        state,
        manifest_url,
        scope.clone(),
        IronHubCommand::List { kind: None },
    )
    .await
    .expect_err("the empty recording egress rejects the catalog request");

    assert!(matches!(error, IronHubCommandError::Catalog { .. }));
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(
        requests[0].capability_id.as_str(),
        super::IRONHUB_SEARCH_CAPABILITY_ID
    );
}

#[tokio::test]
async fn reborn_runtime_wrapper_wires_authenticated_scope_and_mediated_egress() {
    let owner = "runtime-wrapper-owner";
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services(owner, None, false).await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(owner);
    let manifest_url =
        validated_manifest_url("https://hub.ironclaw.com/tests/runtime-wrapper/manifest.json")
            .expect("manifest URL");
    clear_test_manifest_cache(manifest_url.as_str());
    let egress = Arc::new(RecordingEgress::new([]));
    let runtime = WiredRuntime {
        skill_management: services.skill_management,
        extension_management: services.extension_management,
        egress: egress.clone(),
        context: LifecycleProductSurfaceContext {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
        },
        state: Arc::new(IronhubLinkStateStore::new(Arc::new(InMemoryBackend::new()))),
        manifest_url,
    };

    for command in [
        IronHubCommand::List { kind: None },
        IronHubCommand::Info {
            name: "fixture".to_string(),
            kind: None,
        },
        IronHubCommand::Install {
            name: "fixture".to_string(),
            options: IronHubInstallOptions::default(),
        },
    ] {
        let error = execute_reborn_ironhub_command(&runtime, command)
            .await
            .expect_err("empty egress rejects catalog fetch");
        assert!(matches!(error, IronHubCommandError::Catalog { .. }));
    }
    let requests = egress.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].capability_id.as_str(),
        super::IRONHUB_SEARCH_CAPABILITY_ID
    );
    assert_eq!(
        requests[1].capability_id.as_str(),
        super::IRONHUB_INFO_CAPABILITY_ID
    );
    assert_eq!(
        requests[2].capability_id.as_str(),
        super::IRONHUB_INSTALL_CAPABILITY_ID
    );
    for request in requests {
        assert_eq!(request.scope.tenant_id, scope.tenant_id);
        assert_eq!(request.scope.user_id, scope.user_id);
        assert_eq!(request.scope.agent_id, scope.agent_id);
        assert_eq!(request.scope.project_id, scope.project_id);
    }
}

#[tokio::test]
async fn install_rejects_catalog_tools_that_predate_published_extension_manifests() {
    let owner = "stale-tool-owner";
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services(owner, None, false).await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(owner);
    let manifest_url = "https://hub.ironclaw.com/tests/stale-tool/manifest.json";
    clear_test_manifest_cache(manifest_url);
    let artifact = IronHubArtifact {
        url: "https://hub.ironclaw.com/tests/stale-tool/artifact".to_string(),
        size_bytes: 1,
        sha256: "a".repeat(64),
    };
    let manifest = IronHubManifest {
        version: "1".to_string(),
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        release_tag: "v1".to_string(),
        repo: "nearai/stale-tool".to_string(),
        tools: vec![super::model::IronHubToolEntry {
            name: "stale-tool".to_string(),
            version: "1.0.0".to_string(),
            description: "stale catalog entry".to_string(),
            provenance: IronHubProvenance::Official,
            wasm: artifact.clone(),
            capabilities: artifact,
            manifest: None,
            schemas: std::collections::BTreeMap::new(),
        }],
        skills: Vec::new(),
    };
    let envelope = signed_manifest(
        serde_json::to_string(&manifest).expect("manifest JSON"),
        &test_signing_key(),
    );
    let service = configure_test_catalog(
        IronHubService::new_with_runtime_egress(
            services.skill_management,
            services.extension_management,
            Arc::new(RecordingEgress::new([(manifest_url, envelope)])),
            scope,
            CapabilityId::new(super::IRONHUB_INSTALL_CAPABILITY_ID).expect("capability id"),
            test_link_state(),
        ),
        manifest_url,
        test_manifest_verify_keys(),
    );

    let error = service
        .execute(IronHubCommand::Install {
            name: "stale-tool".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Tool),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect_err("tool without a published extension manifest must fail closed");
    assert!(
        error
            .to_string()
            .contains("publishes no extension manifest")
    );
}

#[tokio::test]
async fn default_manifest_verifier_rejects_unsigned_catalog_bytes() {
    let owner = "production-key-owner";
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services(owner, None, false).await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(owner);
    let manifest_url = "https://hub.ironclaw.com/tests/production-key/manifest.json";
    clear_test_manifest_cache(manifest_url);
    let service = IronHubService::new_with_runtime_egress(
        services.skill_management,
        services.extension_management,
        Arc::new(RecordingEgress::new([(manifest_url, b"unsigned".to_vec())])),
        scope,
        CapabilityId::new(super::IRONHUB_SEARCH_CAPABILITY_ID).expect("capability id"),
        test_link_state(),
    )
    .with_manifest_url(manifest_url.to_string());

    let error = service
        .execute(IronHubCommand::List { kind: None })
        .await
        .expect_err("unsigned catalog must fail production-key verification");
    assert!(
        error
            .to_string()
            .contains("signed manifest verification failed")
    );
}

#[tokio::test]
async fn private_manifest_fetch_rejects_oversize_malformed_and_invalid_timestamp_payloads() {
    let owner = "private-validation-owner";
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services(owner, None, false).await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(owner);
    let configured_url = super::model::DEFAULT_IRONHUB_MANIFEST_URL;
    let private_url = "https://hub.ironclaw.com/api/private/validation-manifest";
    let artifact_url = "https://hub.ironclaw.com/api/private/fixture/SKILL.md";
    let artifact = b"---\nname: fixture\ndescription: fixture\n---\n# Fixture\n";
    let invalid_timestamp = skill_manifest("fixture", artifact_url, artifact, "not-rfc3339");
    let cases = [
        (
            signed_manifest("not JSON".to_string(), &test_signing_key()),
            "private manifest parse failed",
        ),
        (
            signed_manifest(
                serde_json::to_string(&invalid_timestamp).expect("manifest JSON"),
                &test_signing_key(),
            ),
            "generated_at is not RFC3339",
        ),
        (
            signed_manifest(
                format!(r#"{{"padding":"{}"}}"#, "x".repeat(1024 * 1024)),
                &test_signing_key(),
            ),
            "private manifest exceeds size cap",
        ),
    ];

    for (envelope, expected) in cases {
        let service = configure_test_catalog(
            IronHubService::new_with_runtime_egress(
                Arc::clone(&services.skill_management),
                Arc::clone(&services.extension_management),
                Arc::new(RecordingEgress::new([(private_url, envelope)])),
                scope.clone(),
                CapabilityId::new(super::IRONHUB_INSTALL_CAPABILITY_ID).expect("capability id"),
                test_link_state(),
            ),
            configured_url,
            test_manifest_verify_keys(),
        );
        let error = service
            .execute(IronHubCommand::Install {
                name: "fixture".to_string(),
                options: IronHubInstallOptions {
                    kind: Some(IronHubEntryKind::Skill),
                    private_manifest_url: Some(private_url.to_string()),
                    ..IronHubInstallOptions::default()
                },
            })
            .await
            .expect_err("invalid private manifest must fail closed");
        assert!(
            error.to_string().contains(expected),
            "expected {expected}, got {error}"
        );
    }
}

#[test]
fn manifest_url_validation_rejects_non_https_and_preserves_valid_urls() {
    assert!(validated_manifest_url("http://hub.ironclaw.com/manifest.json").is_err());

    let value = "https://hub.ironclaw.com/manifest.json";
    let validated = validated_manifest_url(value).expect("HTTPS manifest URL");
    assert_eq!(validated.as_str(), value);
}

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
fn signed_catalog_verification_rejects_bad_signature() {
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let other_key = SigningKey::from_bytes(&[8_u8; 32]);
    let manifest = br#"{"version":"1"}"#;
    let envelope = serde_json::json!({
        "v": 1,
        "key_id": "test-key",
        "manifest_b64": URL_SAFE_NO_PAD.encode(manifest),
        "sig": URL_SAFE_NO_PAD.encode(other_key.sign(manifest).to_bytes()),
    })
    .to_string();
    let verify_key = hex::encode(signing_key.verifying_key().to_bytes());

    let error = verify_signed_manifest_with_keys(envelope.as_bytes(), &[("test-key", &verify_key)])
        .expect_err("signature from another key must fail closed");
    assert_eq!(error, "manifest signature verification failed");
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
        IronHubManifestSource::Public,
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
        IronHubManifestSource::Public,
    )
    .expect("operator acknowledgement permits install");
}

#[test]
fn private_provenance_requires_a_validated_private_source() {
    let mut manifest = skill_manifest(
        "private-skill",
        "https://hub.ironclaw.com/private/SKILL.md",
        b"private",
        "2026-01-02T00:00:00Z",
    );
    manifest.skills[0].provenance = IronHubProvenance::Private;
    let options = IronHubInstallOptions::default();

    let error = classify_gate_and_digest(
        &manifest,
        "private-skill",
        Some(IronHubEntryKind::Skill),
        &options,
        IronHubManifestSource::Public,
    )
    .expect_err("private provenance from a public source must fail closed");
    assert!(error.to_string().contains("claims private provenance"));

    let (_, provenance, _) = classify_gate_and_digest(
        &manifest,
        "private-skill",
        Some(IronHubEntryKind::Skill),
        &options,
        IronHubManifestSource::Private,
    )
    .expect("validated private source establishes private provenance");
    assert_eq!(provenance, IronHubProvenance::Private);
}

#[test]
fn private_manifest_origin_is_pinned_to_configured_catalog() {
    let configured = "https://catalog.example/api/catalog/manifest.json";
    let origin = validate_private_manifest_origin(
        configured,
        "https://catalog.example/api/private/manifest?access=rotating",
    )
    .expect("same origin is accepted");
    assert!(
        validate_private_manifest_origin(
            configured,
            "https://evil.example/api/private/manifest?access=rotating"
        )
        .is_err()
    );
    assert!(
        validate_private_manifest_origin(
            configured,
            "https://catalog.example:444/api/private/manifest"
        )
        .is_err()
    );
    assert!(
        validate_private_manifest_origin(configured, "http://catalog.example/api/private/manifest")
            .is_err()
    );
    for url in [
        "https://user@catalog.example/api/private/manifest",
        "https://user:password@catalog.example/api/private/manifest",
        "https://:password@catalog.example/api/private/manifest",
    ] {
        assert!(
            validate_private_manifest_origin(configured, url).is_err(),
            "private manifest URLs with userinfo must be rejected"
        );
    }

    let cross_origin_artifact = skill_manifest(
        "private-skill",
        "https://evil.example/private/SKILL.md",
        b"private",
        "2026-01-02T00:00:00Z",
    );
    assert!(
        validate_private_manifest(&cross_origin_artifact, &origin).is_err(),
        "a private manifest cannot redirect artifact download off the pinned origin"
    );
}

#[test]
fn catalog_validation_covers_published_tool_assets_and_origin_boundaries() {
    let artifact = |url: &str| IronHubArtifact {
        url: url.to_string(),
        size_bytes: 2,
        sha256: "a".repeat(64),
    };
    let tool = super::model::IronHubToolEntry {
        name: "fixture".to_string(),
        version: "1.0.0".to_string(),
        description: "fixture".to_string(),
        provenance: IronHubProvenance::Official,
        wasm: artifact("https://hub.ironclaw.com/fixture.wasm"),
        capabilities: artifact("https://hub.ironclaw.com/capabilities.json"),
        manifest: Some(artifact("https://hub.ironclaw.com/manifest.toml")),
        schemas: std::collections::BTreeMap::from([(
            "schemas/input.json".to_string(),
            artifact("https://hub.ironclaw.com/input.json"),
        )]),
    };
    let manifest = IronHubManifest {
        version: "1".to_string(),
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        release_tag: "v1".to_string(),
        repo: "nearai/fixture".to_string(),
        tools: vec![tool.clone()],
        skills: Vec::new(),
    };
    validate_manifest(&manifest).expect("complete published tool manifest");

    let mut too_many = manifest.clone();
    too_many.tools[0].schemas = (0..=super::model::MAX_TOOL_SCHEMA_ARTIFACTS)
        .map(|index| {
            (
                format!("schemas/{index}.json"),
                artifact("https://hub.ironclaw.com/schema.json"),
            )
        })
        .collect();
    assert!(validate_manifest(&too_many).is_err());

    let mut invalid_path = manifest;
    invalid_path.tools[0].schemas = std::collections::BTreeMap::from([(
        "../outside.json".to_string(),
        artifact("https://hub.ironclaw.com/schema.json"),
    )]);
    assert!(validate_manifest(&invalid_path).is_err());

    let port_origin = validate_private_manifest_origin(
        "https://catalog.example:8443/catalog",
        "https://catalog.example:8443/private",
    )
    .expect("same non-default port");
    assert_eq!(
        port_origin.redacted_source_url(),
        "https://catalog.example:8443/"
    );

    for configured in [
        "not a URL",
        "http://catalog.example/catalog",
        "https://user@catalog.example/catalog",
        "https://localhost/catalog",
    ] {
        assert!(
            validate_private_manifest_origin(configured, "https://catalog.example/private")
                .is_err(),
            "configured catalog origin must reject {configured}"
        );
    }
}

#[test]
fn private_manifest_access_token_is_redacted_from_debug_output() {
    let secret = "high-value-access-token";
    let options = IronHubInstallOptions {
        private_manifest_url: Some(format!(
            "https://catalog.example/private/manifest?token={secret}"
        )),
        ..IronHubInstallOptions::default()
    };

    let debug = format!("{options:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains(secret));
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
    let tool_manifest_url = "https://hub.ironclaw.com/tests/native-install/manifest.toml";
    let input_schema_url = "https://hub.ironclaw.com/tests/native-install/invoke.input.v1.json";
    let output_schema_url = "https://hub.ironclaw.com/tests/native-install/raw_output.v1.json";
    let skill_url = "https://hub.ironclaw.com/tests/native-install/SKILL.md";
    let tool_bytes = include_bytes!(
        "../../../ironclaw_first_party_extensions/assets/github/wasm/github_tool.wasm"
    )
    .to_vec();
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
            tool_manifest_url,
            input_schema_url,
            output_schema_url,
        }),
        &test_signing_key(),
    );
    let egress = Arc::new(RecordingEgress::new([
        (manifest_url, manifest),
        (tool_url, tool_bytes),
        (capabilities_url, capabilities_bytes),
        (tool_manifest_url, published_tool_manifest("0.1.0")),
        (input_schema_url, published_input_schema()),
        (output_schema_url, published_output_schema()),
        (skill_url, skill_bytes),
    ]));
    let service = configure_test_catalog(
        IronHubService::new_with_runtime_egress(
            Arc::clone(&services.skill_management),
            Arc::clone(&services.extension_management),
            egress.clone(),
            scope.clone(),
            CapabilityId::new(super::IRONHUB_INSTALL_CAPABILITY_ID).expect("capability id"),
            test_link_state(),
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
    let schema_path = VirtualPath::new(
        "/system/extensions/installed-tool/schemas/installed-tool/invoke.input.v1.json",
    )
    .expect("schema path");
    assert_eq!(
        services
            .filesystem
            .read_file(&schema_path)
            .await
            .expect("verified schema materialized"),
        published_input_schema(),
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
    // Catalog, then the tool's manifest, wasm, capabilities, and two schemas,
    // then the skill.
    assert_eq!(requests.len(), 7);
    assert!(requests.iter().all(|request| {
        request.runtime == RuntimeKind::FirstParty
            && request.policy.deny_private_ip_ranges
            && request.capability_id.as_str() == super::IRONHUB_INSTALL_CAPABILITY_ID
    }));
}

#[tokio::test]
async fn private_manifest_install_retries_after_artifact_failure() {
    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services("private-owner", None, false)
            .await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner("private-owner");
    let configured_catalog_url = "https://hub.ironclaw.com/api/catalog/manifest.json";
    let private_url_a = "https://hub.ironclaw.com/api/private/manifest?access=token-a";
    let private_url_b = "https://hub.ironclaw.com/api/private/manifest?access=token-b";
    let artifact_url = "https://hub.ironclaw.com/api/private/private-skill/SKILL.md";
    let artifact = b"---\nname: private-skill\ndescription: private\n---\n# Private\n".to_vec();
    let manifest = skill_manifest(
        "private-skill",
        artifact_url,
        &artifact,
        "2026-01-02T00:00:00Z",
    );
    let envelope = signed_manifest(
        serde_json::to_string(&manifest).expect("manifest JSON"),
        &test_signing_key(),
    );
    let egress = Arc::new(RecordingEgress::new([
        (private_url_a, envelope.clone()),
        (private_url_b, envelope),
        (artifact_url, vec![b'X'; artifact.len()]),
        (artifact_url, artifact),
    ]));
    let state = Arc::new(IronhubLinkStateStore::new(Arc::clone(&services.filesystem)));
    let service = configure_test_catalog(
        IronHubService::new_with_runtime_egress(
            Arc::clone(&services.skill_management),
            Arc::clone(&services.extension_management),
            egress,
            scope,
            CapabilityId::new(super::IRONHUB_INSTALL_CAPABILITY_ID).expect("capability id"),
            state,
        ),
        configured_catalog_url,
        test_manifest_verify_keys(),
    );

    let first_error = service
        .execute(IronHubCommand::Install {
            name: "private-skill".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Skill),
                private_manifest_url: Some(private_url_a.to_string()),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect_err("first artifact download fails digest verification");
    assert!(first_error.to_string().contains("checksum mismatch"));

    let installed = service
        .execute(IronHubCommand::Install {
            name: "private-skill".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Skill),
                private_manifest_url: Some(private_url_b.to_string()),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect("the identical signed manifest remains retryable");
    assert_eq!(installed.entries[0].provenance, IronHubProvenance::Private);
}

#[tokio::test]
async fn deep_link_install_accepts_hub_digest_and_uses_authenticated_caller_scope() {
    const LINK_KEY: &str = "ihub_sk_CallerScopeTestKey0000000000000000000000000";

    let services =
        crate::lifecycle_test_support::build_lifecycle_test_services("runtime-owner", None, false)
            .await;
    let owner_scope =
        crate::lifecycle_test_support::webui_gate_resource_scope_for_owner("runtime-owner");
    let caller_user = UserId::new("authenticated-caller").expect("caller user");
    let caller = ProductSurfaceCaller::new(
        owner_scope.tenant_id.clone(),
        caller_user.clone(),
        owner_scope.agent_id.clone(),
        owner_scope.project_id.clone(),
    );
    let expected_scope = ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
    };
    let private_url = "https://hub.ironclaw.com/api/private/manifest?access=caller-test";
    let artifact_url = "https://hub.ironclaw.com/api/private/caller-skill/SKILL.md";
    let artifact =
        b"---\nname: caller-skill\ndescription: caller scoped\n---\n# Caller scoped\n".to_vec();
    let mut manifest = skill_manifest(
        "caller-skill",
        artifact_url,
        &artifact,
        "2026-01-03T00:00:00Z",
    );
    manifest.skills[0].provenance = IronHubProvenance::Official;
    let envelope = signed_manifest(
        serde_json::to_string(&manifest).expect("manifest JSON"),
        &test_signing_key(),
    );
    let egress = Arc::new(RecordingEgress::new([
        (private_url, envelope),
        (artifact_url, artifact),
    ]));
    let state = Arc::new(IronhubLinkStateStore::new(Arc::clone(&services.filesystem)));
    let service = configure_test_manifest_verify_keys(
        RebornIronhubLinkService::new(
            Arc::clone(&services.skill_management),
            Arc::clone(&services.extension_management),
            egress.clone(),
            state,
            IronhubSharedKey::new(LINK_KEY).expect("link key"),
        )
        .expect("link service")
        .with_manifest_url(super::IronhubManifestUrl::default()),
        test_manifest_verify_keys(),
    );
    let artifact_digest = format!(
        "sha256:{}",
        sha256_hex(manifest.skills[0].skill_md.sha256.as_bytes())
    );
    let timestamp = u64::try_from(chrono::Utc::now().timestamp()).expect("positive timestamp");
    let mut register_request = IronhubRegisterRequest {
        uid: "signed-hub-user-not-the-caller".to_string(),
        aid: "signed-hub-agent".to_string(),
        ts: timestamp,
        nonce: "caller-scope-register-nonce".to_string(),
        sig: String::new(),
    };
    let register_challenge = RegisterChallenge {
        uid: &register_request.uid,
        aid: &register_request.aid,
        ts: register_request.ts,
        nonce: &register_request.nonce,
    };
    let mut register_mac =
        Hmac::<sha2::Sha256>::new_from_slice(LINK_KEY.as_bytes()).expect("HMAC key");
    register_mac.update(register_challenge.payload().as_bytes());
    register_request.sig = hex::encode(register_mac.finalize().into_bytes());
    service
        .register(register_request)
        .await
        .expect("valid registration succeeds through the product service");

    let mut request = IronhubInstallDeliveryRequest {
        slug: "caller-skill".to_string(),
        version: "1.0.0".to_string(),
        uid: "signed-hub-user-not-the-caller".to_string(),
        aid: "signed-hub-agent".to_string(),
        ts: timestamp,
        nonce: "caller-scope-single-use-nonce".to_string(),
        artifact_digest,
        sig: String::new(),
        private_manifest_url: Some(private_url.to_string()),
    };
    let delivery = InstallDelivery {
        slug: &request.slug,
        version: &request.version,
        uid: &request.uid,
        aid: &request.aid,
        ts: request.ts,
        nonce: &request.nonce,
        artifact_digest: &request.artifact_digest,
        private_manifest_url: request.private_manifest_url.as_deref(),
    };
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(LINK_KEY.as_bytes()).expect("HMAC key");
    mac.update(delivery.payload().as_bytes());
    request.sig = hex::encode(mac.finalize().into_bytes());

    let replay_caller = caller.clone();
    let replay_request = request.clone();
    let result = service
        .deliver_install(caller, request)
        .await
        .expect("authenticated caller install succeeds");
    assert!(result.installed);
    let installed = services
        .skill_management
        .read_content_for_scope(expected_scope.clone(), "caller-skill")
        .await
        .expect("skill is installed for authenticated caller");
    assert!(installed.content.contains("# Caller scoped"));
    assert!(
        services
            .skill_management
            .read_content_for_scope(owner_scope, "caller-skill")
            .await
            .is_err(),
        "runtime owner must not receive the authenticated caller's install"
    );
    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|record| {
        record.scope.tenant_id == expected_scope.tenant_id
            && record.scope.user_id == caller_user
            && record.scope.agent_id == expected_scope.agent_id
            && record.scope.project_id == expected_scope.project_id
    }));
    assert!(matches!(
        service.deliver_install(replay_caller, replay_request).await,
        Err(IronhubLinkError::Replay)
    ));
    assert_eq!(
        egress.requests().len(),
        2,
        "a replayed nonce must be rejected before another manifest or artifact fetch"
    );
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
async fn forced_skill_replacement_failure_restores_installed_skill_without_exposing_source_url() {
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

    let metadata_path = skill_filesystem
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .find(|path| path.as_str().ends_with("/.ironclaw-install.json"))
        .expect("old install metadata path");
    let skill_dir = metadata_path
        .as_str()
        .strip_suffix("/.ironclaw-install.json")
        .expect("metadata has skill directory");
    let companion_path =
        VirtualPath::new(format!("{skill_dir}/references.txt")).expect("companion path");
    let companion_bytes = b"old bundled reference\n";
    skill_filesystem
        .write_file(&companion_path, companion_bytes)
        .await
        .expect("seed old companion file");
    let malformed_metadata = br#"{"source":"installed_url","source_url":"unterminated"#;
    skill_filesystem
        .write_file(&metadata_path, malformed_metadata)
        .await
        .expect("seed malformed install metadata");

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
    let metadata_writes = skill_filesystem
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .filter(|path| path.as_str().contains(".ironclaw-install.json"))
        .count();
    assert_eq!(
        metadata_writes, 4,
        "old install, malformed fixture, failed replacement, and compensation must write metadata"
    );
    assert_eq!(
        skill_filesystem
            .read_file(&metadata_path)
            .await
            .expect("restored metadata"),
        malformed_metadata,
        "compensation must restore malformed metadata byte-for-byte"
    );
    assert_eq!(
        skill_filesystem
            .read_file(&companion_path)
            .await
            .expect("restored companion file"),
        companion_bytes,
        "compensation must restore every bundled file"
    );
    let restored = skill_management
        .read_content_for_scope(scope.clone(), "installed-skill")
        .await
        .expect("restored skill is readable");
    assert_eq!(restored.content.as_bytes(), old_skill);
    assert_eq!(restored.source, ManagedSkillSource::Installed);
    assert_eq!(restored.source_url, None);
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
}

#[tokio::test]
async fn execute_rejects_non_utf8_install_artifacts() {
    let services = crate::lifecycle_test_support::build_lifecycle_test_services(
        "ironhub-non-utf8-owner",
        None,
        false,
    )
    .await;
    let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(
        "ironhub-non-utf8-owner",
    );

    let skill_manifest_url = "https://hub.ironclaw.com/tests/non-utf8-skill/manifest.json";
    let skill_url = "https://hub.ironclaw.com/tests/non-utf8-skill/SKILL.md";
    let invalid_skill = vec![0xff, 0xfe];
    let skill_manifest = signed_manifest(
        skill_manifest_json(
            "installed-skill",
            "2026-01-07T00:00:00Z",
            "0.1.0",
            skill_url,
            invalid_skill.len(),
            &sha256_hex(&invalid_skill),
        ),
        &test_signing_key(),
    );
    let skill_error = configured_service(
        Arc::clone(&services.skill_management),
        Arc::clone(&services.extension_management),
        Arc::new(RecordingEgress::new([
            (skill_manifest_url, skill_manifest),
            (skill_url, invalid_skill),
        ])),
        scope.clone(),
        skill_manifest_url,
    )
    .execute(install_command(IronHubEntryKind::Skill, false))
    .await
    .expect_err("non-UTF-8 skill markdown is rejected");
    assert!(matches!(
        skill_error,
        IronHubCommandError::Install { reason } if reason.contains("not UTF-8")
    ));

    let catalog_url = "https://hub.ironclaw.com/tests/non-utf8-tool/manifest.json";
    let tool_url = "https://hub.ironclaw.com/tests/non-utf8-tool/tool.wasm";
    let capabilities_url = "https://hub.ironclaw.com/tests/non-utf8-tool/capabilities.json";
    let tool_manifest_url = "https://hub.ironclaw.com/tests/non-utf8-tool/manifest.toml";
    let input_schema_url = "https://hub.ironclaw.com/tests/non-utf8-tool/invoke.input.v1.json";
    let output_schema_url = "https://hub.ironclaw.com/tests/non-utf8-tool/raw_output.v1.json";
    let tool_bytes = include_bytes!(
        "../../../ironclaw_first_party_extensions/assets/github/wasm/github_tool.wasm"
    )
    .to_vec();
    let capabilities_bytes = br#"{"capabilities":[]}"#.to_vec();
    let invalid_tool_manifest = vec![0xff, 0xfe];
    let mut catalog: serde_json::Value =
        serde_json::from_str(&tool_manifest_json(ToolManifestFixture {
            generated_at: "2026-01-08T00:00:00Z",
            version: "0.1.0",
            tool_url,
            tool_size: tool_bytes.len(),
            tool_sha: &sha256_hex(&tool_bytes),
            capabilities_url,
            capabilities_size: capabilities_bytes.len(),
            capabilities_sha: &sha256_hex(&capabilities_bytes),
            tool_manifest_url,
            input_schema_url,
            output_schema_url,
        }))
        .expect("tool catalog fixture");
    catalog["tools"][0]["manifest"] = serde_json::json!({
        "url": tool_manifest_url,
        "size_bytes": invalid_tool_manifest.len(),
        "sha256": sha256_hex(&invalid_tool_manifest),
    });
    let catalog = signed_manifest(catalog.to_string(), &test_signing_key());
    let tool_error = configured_service(
        Arc::clone(&services.skill_management),
        Arc::clone(&services.extension_management),
        Arc::new(RecordingEgress::new([
            (catalog_url, catalog),
            (tool_url, tool_bytes),
            (capabilities_url, capabilities_bytes),
            (tool_manifest_url, invalid_tool_manifest),
            (input_schema_url, published_input_schema()),
            (output_schema_url, published_output_schema()),
        ])),
        scope,
        catalog_url,
    )
    .execute(install_command(IronHubEntryKind::Tool, false))
    .await
    .expect_err("non-UTF-8 tool manifest is rejected");
    assert!(matches!(
        tool_error,
        IronHubCommandError::Catalog { reason } if reason.contains("not UTF-8")
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
    let tool_bytes = include_bytes!(
        "../../../ironclaw_first_party_extensions/assets/github/wasm/github_tool.wasm"
    )
    .to_vec();
    let capabilities_bytes = br#"{"capabilities":[]}"#.to_vec();
    let old_manifest_url = format!("https://hub.ironclaw.com/tests/{fixture}/old-manifest.json");
    let old_tool_url = format!("https://hub.ironclaw.com/tests/{fixture}/old-tool.wasm");
    let old_capabilities_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/old-capabilities.json");
    let old_tool_manifest_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/old-manifest.toml");
    let old_input_schema_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/old-input-schema.json");
    let old_output_schema_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/old-output-schema.json");
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
            tool_manifest_url: &old_tool_manifest_url,
            input_schema_url: &old_input_schema_url,
            output_schema_url: &old_output_schema_url,
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
            (
                old_tool_manifest_url.as_str(),
                published_tool_manifest("0.1.0"),
            ),
            (old_input_schema_url.as_str(), published_input_schema()),
            (old_output_schema_url.as_str(), published_output_schema()),
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
    let new_tool_manifest_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/new-manifest.toml");
    let new_input_schema_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/new-input-schema.json");
    let new_output_schema_url =
        format!("https://hub.ironclaw.com/tests/{fixture}/new-output-schema.json");
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
            tool_manifest_url: &new_tool_manifest_url,
            input_schema_url: &new_input_schema_url,
            output_schema_url: &new_output_schema_url,
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
            (
                new_tool_manifest_url.as_str(),
                published_tool_manifest("0.2.0"),
            ),
            (new_input_schema_url.as_str(), published_input_schema()),
            (new_output_schema_url.as_str(), published_output_schema()),
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
            test_link_state(),
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

pub(super) fn test_manifest_verify_keys() -> &'static [(&'static str, &'static str)] {
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

fn skill_manifest(
    name: &str,
    artifact_url: &str,
    artifact: &[u8],
    generated_at: &str,
) -> IronHubManifest {
    IronHubManifest {
        version: "1".to_string(),
        generated_at: generated_at.to_string(),
        release_tag: "test".to_string(),
        repo: "test-org/private-repo".to_string(),
        tools: Vec::new(),
        skills: vec![IronHubSkillEntry {
            name: name.to_string(),
            trunk: String::new(),
            version: "1.0.0".to_string(),
            description: "private skill".to_string(),
            provenance: IronHubProvenance::Private,
            skill_md: IronHubArtifact {
                url: artifact_url.to_string(),
                size_bytes: u64::try_from(artifact.len()).expect("test artifact length"),
                sha256: sha256_hex(artifact),
            },
        }],
    }
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
            test_link_state(),
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
    tool_manifest_url: &'a str,
    input_schema_url: &'a str,
    output_schema_url: &'a str,
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
        tool_manifest_url,
        input_schema_url,
        output_schema_url,
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
            },
            "manifest": published_tool_manifest_artifact(tool_manifest_url, "0.1.0"),
            "schemas": published_tool_schema_artifacts(input_schema_url, output_schema_url)
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
    tool_manifest_url: &'a str,
    input_schema_url: &'a str,
    output_schema_url: &'a str,
}

/// The extension manifest IronHub publishes for the fixture tool, shaped like
/// what `scripts/generate-extension-manifest.py` emits in that repository.
pub(crate) fn published_tool_manifest(version: &str) -> Vec<u8> {
    format!(
        r#"schema_version = "reborn.extension_manifest.v3"
id = "installed-tool"
name = "installed-tool"
version = "{version}"
description = "test tool"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/installed-tool.wasm"

[[tools]]
origin_gate_matrix = {{ loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }}
id = "installed-tool.invoke"
description = "test tool"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/installed-tool/invoke.input.v1.json"
output_schema_ref = "schemas/installed-tool/raw_output.v1.json"
"#
    )
    .into_bytes()
}

fn published_tool_manifest_artifact(url: &str, version: &str) -> serde_json::Value {
    let bytes = published_tool_manifest(version);
    serde_json::json!({
        "url": url,
        "size_bytes": bytes.len(),
        "sha256": sha256_hex(&bytes),
    })
}

fn published_input_schema() -> Vec<u8> {
    br#"{"type":"object","required":["action"]}"#.to_vec()
}

fn published_output_schema() -> Vec<u8> {
    br#"{"description":"Raw JSON output"}"#.to_vec()
}

fn published_tool_schema_artifacts(input_url: &str, output_url: &str) -> serde_json::Value {
    let input = published_input_schema();
    let output = published_output_schema();
    serde_json::json!({
        "schemas/installed-tool/invoke.input.v1.json": {
            "url": input_url,
            "size_bytes": input.len(),
            "sha256": sha256_hex(&input),
        },
        "schemas/installed-tool/raw_output.v1.json": {
            "url": output_url,
            "size_bytes": output.len(),
            "sha256": sha256_hex(&output),
        }
    })
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
        tool_manifest_url,
        input_schema_url,
        output_schema_url,
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
            },
            "manifest": published_tool_manifest_artifact(tool_manifest_url, version),
            "schemas": published_tool_schema_artifacts(input_schema_url, output_schema_url)
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
    scope: ResourceScope,
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
                scope: request.scope.clone(),
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
