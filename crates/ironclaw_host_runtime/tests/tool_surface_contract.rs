mod support;

use support::legacy_capability_fixture_to_v2_with_schema_suffix as legacy_capability_fixture_to_v2;

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::{
    CapabilityVisibility, ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource,
};
use ironclaw_filesystem::{
    DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, LocalFilesystem,
    RootFilesystem,
};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime,
    MAX_HOT_PROMPT_BYTES, MAX_HOT_SCHEMA_BYTES, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeFailureKind, SurfaceKind, VisibleCapabilityAccess, VisibleCapabilityRequest,
    VisibleCapabilitySurface, builtin_first_party_package, publish_hot_capability_catalog,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustError, TrustPolicy, TrustPolicyInput, TrustProvenance,
};
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn hot_capability_catalog_resolves_schema_and_prompt_refs() {
    let (_storage, fs, registry) = hot_catalog_fixture(
        Some(r#"{"type":"object","properties":{"message":{"type":"string"}}}"#),
        r#"{"type":"object","properties":{"ok":{"type":"boolean"}}}"#,
        "Use this tool to echo user text.",
    );

    let catalog = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap();

    let record = catalog.get(&capability_id("echo.say")).unwrap();
    assert_eq!(record.descriptor.id, capability_id("echo.say"));
    assert_eq!(
        record.descriptor.parameters_schema,
        json!({"type":"object","properties":{"message":{"type":"string"}}})
    );
    assert_eq!(
        record.output_schema,
        json!({"type":"object","properties":{"ok":{"type":"boolean"}}})
    );
    assert_eq!(
        record.prompt_doc.as_deref(),
        Some("Use this tool to echo user text.")
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_missing_schema_file() {
    let (_storage, fs, registry) =
        hot_catalog_fixture(None, r#"{"type":"object"}"#, "Prompt docs exist.");

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("missing input_schema_ref")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_invalid_json_schema_file() {
    let (_storage, fs, registry) = hot_catalog_fixture(
        Some("not-json"),
        r#"{"type":"object"}"#,
        "Prompt docs exist.",
    );

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("input_schema_ref") && reason.contains("valid JSON schema")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_invalid_json_schema_semantics() {
    let (_storage, fs, registry) = hot_catalog_fixture(
        Some(r#"{"type":"not-a-json-schema-type"}"#),
        r#"{"type":"object"}"#,
        "Prompt docs exist.",
    );

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("input_schema_ref") && reason.contains("valid JSON schema")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_invalid_output_schema_file() {
    let (_storage, fs, registry) = hot_catalog_fixture(
        Some(r#"{"type":"object"}"#),
        "not-json",
        "Prompt docs exist.",
    );

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("output_schema_ref") && reason.contains("valid JSON schema")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_missing_output_schema_file() {
    let (storage, fs, registry) = hot_catalog_fixture(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"object"}"#,
        "Prompt docs exist.",
    );
    std::fs::remove_file(storage.path().join("echo/schemas/echo/say.output.v1.json")).unwrap();

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("missing output_schema_ref")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_invalid_output_schema_semantics() {
    let (_storage, fs, registry) = hot_catalog_fixture(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"not-a-json-schema-type"}"#,
        "Prompt docs exist.",
    );

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("output_schema_ref") && reason.contains("valid JSON schema")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_missing_prompt_doc_file() {
    let (storage, fs, registry) = hot_catalog_fixture(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"object"}"#,
        "Prompt docs exist.",
    );
    std::fs::remove_file(storage.path().join("echo/prompts/echo/say.md")).unwrap();

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("missing prompt_doc_ref")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_allows_model_visible_capability_without_prompt_doc_ref() {
    let (_storage, fs, registry) = hot_catalog_fixture_with_manifest(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"object"}"#,
        b"Prompt docs exist.",
        manifest_without_prompt_doc_ref(),
    );

    let catalog = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap();

    let record = catalog.get(&capability_id("echo.say")).unwrap();
    assert!(record.prompt_doc.is_none());
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_invalid_utf8_prompt_doc() {
    let (_storage, fs, registry) = hot_catalog_fixture_with_prompt_bytes(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"object"}"#,
        &[0xff, 0xfe, 0xfd],
    );

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("prompt_doc_ref") && reason.contains("valid UTF-8")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_oversized_schema_file() {
    let oversized_schema = " ".repeat(MAX_HOT_SCHEMA_BYTES + 1);
    let (_storage, fs, registry) = hot_catalog_fixture(
        Some(&oversized_schema),
        r#"{"type":"object"}"#,
        "Prompt docs exist.",
    );

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("input_schema_ref") && reason.contains("exceeds")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_for_oversized_prompt_doc() {
    let oversized_prompt = vec![b'a'; MAX_HOT_PROMPT_BYTES + 1];
    let (_storage, fs, registry) = hot_catalog_fixture_with_prompt_bytes(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"object"}"#,
        &oversized_prompt,
    );

    let err = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("prompt_doc_ref") && reason.contains("exceeds")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn hot_capability_catalog_skips_non_model_capabilities() {
    let (_storage, fs, registry) = hot_catalog_fixture_with_manifest(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"object"}"#,
        b"Prompt docs exist.",
        manifest_with_visibility(CapabilityVisibility::Api),
    );

    let catalog = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap();

    assert!(catalog.capabilities.is_empty());
    assert!(catalog.get(&capability_id("echo.say")).is_none());
}

#[tokio::test]
async fn hot_capability_catalog_fails_closed_when_bounded_backend_returns_too_many_bytes() {
    let (_storage, _fs, registry) = hot_catalog_fixture(
        Some(r#"{"type":"object"}"#),
        r#"{"type":"object"}"#,
        "Prompt docs exist.",
    );

    let err = publish_hot_capability_catalog(&OversizedReadFilesystem, &registry)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ironclaw_host_runtime::HostRuntimeError::InvalidRequest { ref reason }
            if reason.contains("input_schema_ref") && reason.contains("exceeds")),
        "unexpected error: {err:?}"
    );
}

#[test]
fn hot_capability_manifest_rejects_traversal_schema_ref_at_parse_boundary() {
    let manifest = HOT_CAPABILITY_MANIFEST.replace(
        r#"input_schema_ref = "schemas/echo/say.input.v1.json""#,
        r#"input_schema_ref = "../secrets/schema.json""#,
    );

    let err = ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("..") || err.to_string().contains("dot path segments"),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn visible_surface_empty_registry_returns_deterministic_empty_version() {
    let runtime = runtime_with(ExtensionRegistry::new(), Arc::new(GrantAuthorizer));
    let context = context_with_grants([]);
    let request = VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap());

    let first = runtime.visible_capabilities(request.clone()).await.unwrap();
    let second = runtime.visible_capabilities(request).await.unwrap();

    assert!(first.capabilities.is_empty());
    assert_eq!(first.version, second.version);
    assert_ne!(first.version.as_str(), "surface-v1");
    assert!(first.version.as_str().starts_with("sha256:"));
}

#[tokio::test]
async fn visible_surface_default_policy_and_missing_provider_trust_fail_closed() {
    let authorizer = Arc::new(CountingGrantAuthorizer::default());
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        authorizer.clone(),
    );
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let default_policy_surface = runtime
        .visible_capabilities(
            VisibleCapabilityRequest::new(context.clone(), SurfaceKind::new("agent_loop").unwrap())
                .with_provider_trust(provider_trust_for(default_provider_trust())),
        )
        .await
        .unwrap();
    assert!(default_policy_surface.capabilities.is_empty());
    assert_eq!(authorizer.call_count(), 0);

    let missing_trust_surface = runtime
        .visible_capabilities(
            VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
                .with_policy(CapabilitySurfacePolicy::allow_all()),
        )
        .await
        .unwrap();
    assert!(missing_trust_surface.capabilities.is_empty());
    assert_eq!(authorizer.call_count(), 0);
}

#[tokio::test]
async fn visible_surface_uses_caller_provider_trust_not_host_trust_policy() {
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(PanicTrustPolicy));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_eq!(visible_ids(&surface), vec![capability_id("echo.say")]);
}

#[tokio::test]
async fn visible_surface_hides_host_internal_capabilities() {
    let manifest = r#"
schema_version = "reborn.extension_manifest.v2"
id = "github"
name = "GitHub"
version = "0.1.0"
description = "GitHub test"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/github.wasm"

[[capabilities]]
id = "github.search_issues"
description = "search"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/github/search.input.json"
output_schema_ref = "schemas/github/search.output.json"

[[capabilities]]
id = "github.get_issue"
description = "get"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/github/get.input.json"
output_schema_ref = "schemas/github/get.output.json"

[[capabilities]]
id = "github.comment_issue"
description = "comment"
effects = ["network", "external_write"]
default_permission = "ask"
visibility = "host_internal"
input_schema_ref = "schemas/github/comment.input.json"
output_schema_ref = "schemas/github/comment.output.json"
"#;
    let manifest = ExtensionManifest::parse(
        manifest,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/github").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let runtime = runtime_with(registry, Arc::new(GrantAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "github",
            "/system/extensions/github/manifest.toml",
            vec![EffectKind::Network, EffectKind::ExternalWrite],
        )]),
    ));
    let context = context_with_grants([
        (
            capability_id("github.search_issues"),
            vec![EffectKind::Network],
        ),
        (capability_id("github.get_issue"), vec![EffectKind::Network]),
        (
            capability_id("github.comment_issue"),
            vec![EffectKind::Network, EffectKind::ExternalWrite],
        ),
    ]);

    let surface = runtime
        .visible_capabilities(request_with_provider_trust(
            context,
            [(
                "github",
                vec![EffectKind::Network, EffectKind::ExternalWrite],
            )],
        ))
        .await
        .unwrap();

    assert_eq!(
        visible_ids(&surface),
        vec![
            capability_id("github.search_issues"),
            capability_id("github.get_issue"),
        ]
    );
}

#[tokio::test]
async fn visible_surface_resolves_builtin_first_party_input_schema_refs() {
    let package = builtin_first_party_package().unwrap();
    assert!(
        package
            .capabilities
            .iter()
            .all(|descriptor| descriptor.parameters_schema.get("$ref").is_some())
    );

    let mut registry = ExtensionRegistry::new();
    registry.insert(package.clone()).unwrap();
    let runtime = runtime_with(registry, Arc::new(GrantAuthorizer));
    let context = context_with_grant_entries(
        package
            .capabilities
            .iter()
            .map(|descriptor| (descriptor.id.clone(), descriptor.effects.clone())),
    );

    let surface = runtime
        .visible_capabilities(request_with_provider_trust(
            context,
            [("builtin", combined_descriptor_effects(&package))],
        ))
        .await
        .unwrap();

    assert_eq!(surface.capabilities.len(), package.capabilities.len());
    for capability in &surface.capabilities {
        jsonschema::validator_for(&capability.descriptor.parameters_schema).unwrap_or_else(
            |error| {
                panic!(
                    "{} should expose a valid JSON schema: {error}",
                    capability.descriptor.id
                )
            },
        );
        assert!(
            capability
                .descriptor
                .parameters_schema
                .get("$ref")
                .is_none(),
            "{} should expose a concrete input schema, got {:?}",
            capability.descriptor.id,
            capability.descriptor.parameters_schema
        );
    }
    assert_schema_has_property(&surface, "builtin.glob", "pattern");
    assert_schema_has_property(&surface, "builtin.grep", "pattern");
    assert_schema_has_property(&surface, "builtin.skill_install", "content");
    assert_schema_has_property(&surface, "builtin.skill_install", "name");
    assert_schema_has_property(&surface, "builtin.skill_install_url", "url");
    assert_schema_has_property(&surface, "builtin.skill_install_url", "name");

    let skill_install_schema = &surface
        .capabilities
        .iter()
        .find(|capability| capability.descriptor.id == capability_id("builtin.skill_install"))
        .expect("builtin.skill_install should be visible")
        .descriptor
        .parameters_schema;
    let skill_install_validator =
        jsonschema::validator_for(skill_install_schema).expect("skill_install schema is valid");

    skill_install_validator
        .validate(&json!({
            "content": "---\nname: pasted-skill\n---\n\nUse multiline Markdown.\n"
        }))
        .expect("skill_install should accept multiline SKILL.md content");
    assert!(
        skill_install_validator
            .validate(&json!({"name": "pasted-skill"}))
            .is_err(),
        "skill_install content remains required"
    );
    assert!(
        skill_install_validator
            .validate(&json!({
                "url": "https://example.test/SKILL.md",
                "content": "---\nname: pasted-skill\n---\n\nUse multiline Markdown.\n"
            }))
            .is_err(),
        "skill_install should reject URL input"
    );

    let skill_install_url_schema = &surface
        .capabilities
        .iter()
        .find(|capability| capability.descriptor.id == capability_id("builtin.skill_install_url"))
        .expect("builtin.skill_install_url should be visible")
        .descriptor
        .parameters_schema;
    let skill_install_url_validator = jsonschema::validator_for(skill_install_url_schema)
        .expect("skill_install_url schema is valid");
    skill_install_url_validator
        .validate(&json!({
            "url": "https://example.test/SKILL.md"
        }))
        .expect("skill_install_url should accept a SKILL.md URL");
    assert!(
        skill_install_url_validator
            .validate(&json!({
                "url": "https://example.test/SKILL.md",
                "content": "---\nname: pasted-skill\n---\n\nUse multiline Markdown.\n"
            }))
            .is_err(),
        "skill_install_url should reject ambiguous content plus url input"
    );
}

#[tokio::test]
async fn visible_surface_filters_by_grants_provider_trust_and_preserves_registry_order() {
    let registry = registry_from_manifests([
        (ECHO_MANIFEST, "/system/extensions/echo"),
        (FILES_MANIFEST, "/system/extensions/files"),
        (NET_MANIFEST, "/system/extensions/net"),
    ]);
    let runtime = runtime_with(registry, Arc::new(GrantAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([
            (
                "echo",
                "/system/extensions/echo/manifest.toml",
                vec![EffectKind::DispatchCapability],
            ),
            (
                "files",
                "/system/extensions/files/manifest.toml",
                vec![EffectKind::ReadFilesystem],
            ),
            (
                "net",
                "/system/extensions/net/manifest.toml",
                vec![EffectKind::Network],
            ),
        ]),
    ));
    let context = context_with_grants([
        (
            capability_id("files.read"),
            vec![EffectKind::ReadFilesystem],
        ),
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
    ]);

    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    let visible_ids: Vec<_> = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.clone())
        .collect();
    assert_eq!(
        visible_ids,
        vec![capability_id("echo.say"), capability_id("files.read")],
        "filtered surface must preserve registry order, not grant order"
    );
    assert_eq!(surface.capabilities.len(), 2);
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.access == VisibleCapabilityAccess::Available)
    );
}

#[tokio::test]
async fn visible_surface_omits_missing_trust_and_insufficient_trust_ceiling() {
    let registry = registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]);
    let granted_context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let missing_policy_runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    );
    let missing_policy_surface = missing_policy_runtime
        .visible_capabilities(request_with_provider_trust(
            granted_context.clone(),
            Vec::new(),
        ))
        .await
        .unwrap();
    assert!(missing_policy_surface.capabilities.is_empty());

    let insufficient_policy_runtime = runtime_with(registry, Arc::new(GrantAuthorizer));
    let insufficient_surface = insufficient_policy_runtime
        .visible_capabilities(request_with_provider_trust(
            granted_context,
            vec![("echo", Vec::new())],
        ))
        .await
        .unwrap();
    assert!(insufficient_surface.capabilities.is_empty());
}

#[tokio::test]
async fn visible_surface_policy_filters_runtime_and_effects_before_authorization() {
    let registry = registry_from_manifests([
        (ECHO_MANIFEST, "/system/extensions/echo"),
        (SCRIPT_MANIFEST, "/system/extensions/scripts"),
        (NET_MANIFEST, "/system/extensions/net"),
    ]);
    let authorizer = Arc::new(PanicAuthorizer);
    let runtime =
        runtime_with(registry, authorizer).with_trust_policy(Arc::new(trust_policy_for([
            (
                "echo",
                "/system/extensions/echo/manifest.toml",
                vec![EffectKind::DispatchCapability],
            ),
            (
                "scripts",
                "/system/extensions/scripts/manifest.toml",
                vec![EffectKind::ExecuteCode],
            ),
            (
                "net",
                "/system/extensions/net/manifest.toml",
                vec![EffectKind::Network],
            ),
        ])));
    let mut request = visible_request(context_with_grants([
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
        (capability_id("scripts.run"), vec![EffectKind::ExecuteCode]),
        (capability_id("net.fetch"), vec![EffectKind::Network]),
    ]));
    request.policy = CapabilitySurfacePolicy {
        allowed_runtimes: vec![RuntimeKind::Wasm],
        allowed_effects: vec![EffectKind::DispatchCapability],
        include_requires_approval: true,
        max_capabilities: None,
    };

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert_eq!(surface.capabilities.len(), 1);
    assert_eq!(
        surface.capabilities[0].descriptor.id,
        capability_id("echo.say")
    );
}

#[tokio::test]
async fn visible_surface_marks_askable_capabilities_without_granting_authority() {
    let registry = registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]);
    let runtime = runtime_with(registry, Arc::new(ApprovalAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability],
        )]),
    ));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let surface = runtime
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();

    assert_eq!(surface.capabilities.len(), 1);
    assert_eq!(
        surface.capabilities[0].access,
        VisibleCapabilityAccess::RequiresApproval
    );

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id("echo.say"),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Failed(_)),
        "surface visibility must not bypass approval stores or grant authority"
    );
}

#[tokio::test]
async fn hidden_capability_direct_invoke_still_fails_closed_through_authorization() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let runtime = DefaultHostRuntime::new(
        Arc::new(registry_from_manifests([(
            ECHO_MANIFEST,
            "/system/extensions/echo",
        )])),
        dispatcher.clone(),
        Arc::new(GrantAuthorizer),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_dev_runtime_policy(),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));
    let context = context_with_grants([]);

    let surface = runtime
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    assert!(surface.capabilities.is_empty());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id("echo.say"),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind.as_str(), "authorization");
        }
        other => panic!("expected authorization failure, got {other:?}"),
    }
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn visible_surface_version_changes_with_schema_and_policy_changes() {
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let runtime_a = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));
    let runtime_b = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST_WITH_SCHEMA, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));

    let surface_a = runtime_a
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    let surface_b = runtime_b
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    assert_ne!(surface_a.version, surface_b.version);

    let mut policy_request = visible_request(context);
    policy_request.policy.max_capabilities = Some(0);
    let narrowed = runtime_a
        .visible_capabilities(policy_request)
        .await
        .unwrap();
    assert_ne!(surface_a.version, narrowed.version);
}

#[tokio::test]
async fn visible_surface_version_changes_with_runtime_policy_changes() {
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let trust_policy = Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )]));
    let runtime_a = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::clone(&trust_policy));
    let runtime_b = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(trust_policy)
    .with_runtime_policy(secret_denied_runtime_policy());

    let surface_a = runtime_a
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    let surface_b = runtime_b
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_eq!(visible_ids(&surface_a), visible_ids(&surface_b));
    assert_ne!(
        surface_a.version, surface_b.version,
        "surface version must change when runtime policy changes even if visible descriptors do not"
    );
}

#[tokio::test]
async fn visible_surface_version_changes_with_returned_descriptor_metadata() {
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let runtime_a = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));
    let runtime_b = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST_WITH_DESCRIPTION, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));

    let surface_a = runtime_a
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    let surface_b = runtime_b
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_ne!(
        surface_a.capabilities[0].descriptor.description,
        surface_b.capabilities[0].descriptor.description
    );
    assert_ne!(
        surface_a.version, surface_b.version,
        "surface version must change when returned descriptor metadata changes"
    );
}

#[tokio::test]
async fn visible_surface_version_is_order_insensitive_for_equivalent_policy() {
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));

    let policy_a = CapabilitySurfacePolicy {
        allowed_runtimes: vec![RuntimeKind::Wasm, RuntimeKind::Script],
        allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        include_requires_approval: true,
        max_capabilities: None,
    };
    let policy_b = CapabilitySurfacePolicy {
        allowed_runtimes: vec![RuntimeKind::Script, RuntimeKind::Wasm],
        allowed_effects: vec![EffectKind::Network, EffectKind::DispatchCapability],
        include_requires_approval: true,
        max_capabilities: None,
    };

    let surface_a = runtime
        .visible_capabilities(visible_request(context.clone()).with_policy(policy_a))
        .await
        .unwrap();
    let surface_b = runtime
        .visible_capabilities(visible_request(context).with_policy(policy_b))
        .await
        .unwrap();

    assert_eq!(visible_ids(&surface_a), visible_ids(&surface_b));
    assert_eq!(
        surface_a.version, surface_b.version,
        "equivalent allow-list ordering must not churn the surface version"
    );
}

#[tokio::test]
async fn visible_surface_version_is_order_insensitive_for_equivalent_capability_set() {
    let context = context_with_grants([
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
        (
            capability_id("files.read"),
            vec![EffectKind::ReadFilesystem],
        ),
    ]);
    let trust_policy = Arc::new(trust_policy_for([
        (
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability],
        ),
        (
            "files",
            "/system/extensions/files/manifest.toml",
            vec![EffectKind::ReadFilesystem],
        ),
    ]));
    let runtime_a = runtime_with(
        registry_from_manifests([
            (ECHO_MANIFEST, "/system/extensions/echo"),
            (FILES_MANIFEST, "/system/extensions/files"),
        ]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::clone(&trust_policy));
    let runtime_b = runtime_with(
        registry_from_manifests([
            (FILES_MANIFEST, "/system/extensions/files"),
            (ECHO_MANIFEST, "/system/extensions/echo"),
        ]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(trust_policy);

    let surface_a = runtime_a
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    let surface_b = runtime_b
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_ne!(visible_ids(&surface_a), visible_ids(&surface_b));
    assert_eq!(
        surface_a.version, surface_b.version,
        "equivalent capability sets must hash in canonical key order"
    );
}

#[tokio::test]
async fn visible_surface_max_capabilities_stops_authorization_after_limit() {
    let registry = registry_from_manifests([
        (ECHO_MANIFEST, "/system/extensions/echo"),
        (FILES_MANIFEST, "/system/extensions/files"),
        (NET_MANIFEST, "/system/extensions/net"),
    ]);
    let authorizer = Arc::new(CountingGrantAuthorizer::default());
    let runtime =
        runtime_with(registry, authorizer.clone()).with_trust_policy(Arc::new(trust_policy_for([
            (
                "echo",
                "/system/extensions/echo/manifest.toml",
                vec![EffectKind::DispatchCapability],
            ),
            (
                "files",
                "/system/extensions/files/manifest.toml",
                vec![EffectKind::ReadFilesystem],
            ),
            (
                "net",
                "/system/extensions/net/manifest.toml",
                vec![EffectKind::Network],
            ),
        ])));
    let context = context_with_grants([
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
        (
            capability_id("files.read"),
            vec![EffectKind::ReadFilesystem],
        ),
        (capability_id("net.fetch"), vec![EffectKind::Network]),
    ]);
    let request = visible_request(context).with_policy(CapabilitySurfacePolicy {
        max_capabilities: Some(1),
        ..CapabilitySurfacePolicy::allow_all()
    });

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert_eq!(surface.capabilities.len(), 1);
    assert_eq!(authorizer.call_count(), 1);
}

#[tokio::test]
async fn visible_surface_can_hide_approval_required_capabilities_by_policy() {
    let registry = registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]);
    let runtime = runtime_with(registry, Arc::new(ApprovalAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability],
        )]),
    ));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let request = visible_request(context).with_policy(CapabilitySurfacePolicy {
        include_requires_approval: false,
        ..CapabilitySurfacePolicy::allow_all()
    });

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert!(surface.capabilities.is_empty());
}

#[tokio::test]
async fn visible_surface_requires_every_descriptor_effect_to_be_policy_allowed() {
    let registry = registry_from_manifests([(ECHO_NETWORK_MANIFEST, "/system/extensions/echo")]);
    let runtime = runtime_with(registry, Arc::new(PanicAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability, EffectKind::Network],
        )]),
    ));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )]);
    let request = visible_request(context).with_policy(CapabilitySurfacePolicy {
        allowed_effects: vec![EffectKind::DispatchCapability],
        ..CapabilitySurfacePolicy::allow_all()
    });

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert!(surface.capabilities.is_empty());
}

#[tokio::test]
async fn visible_surface_hides_mcp_http_when_policy_denies_network_even_if_effect_underdeclared() {
    let runtime = runtime_with(
        registry_from_manifests([(MCP_UNDERDECLARED_NETWORK_MANIFEST, "/system/extensions/mcp")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "mcp",
        "/system/extensions/mcp/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])))
    .with_runtime_policy(network_denied_runtime_policy());

    let context = context_with_grants([(
        capability_id("mcp.search"),
        vec![EffectKind::DispatchCapability],
    )]);
    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert!(
        surface.capabilities.is_empty(),
        "NetworkMode::Deny must hide HTTP/SSE MCP even if the manifest omits the network effect"
    );
}

#[tokio::test]
async fn visible_surface_hides_script_when_policy_denies_processes_even_if_effect_underdeclared() {
    let runtime = runtime_with(
        registry_from_manifests([(
            SCRIPT_UNDERDECLARED_PROCESS_MANIFEST,
            "/system/extensions/scripts",
        )]),
        Arc::new(PanicAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "scripts",
        "/system/extensions/scripts/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])))
    .with_runtime_policy(network_denied_runtime_policy());

    let context = context_with_grants([(
        capability_id("scripts.run"),
        vec![EffectKind::DispatchCapability],
    )]);
    let surface = runtime
        .visible_capabilities(request_with_provider_trust(
            context,
            vec![("scripts", vec![EffectKind::DispatchCapability])],
        ))
        .await
        .unwrap();

    assert!(
        surface.capabilities.is_empty(),
        "ProcessBackendKind::None must hide script tools even if the manifest omits process effects"
    );
}

#[tokio::test]
async fn visible_surface_hides_secret_when_policy_denies_secrets() {
    let runtime = runtime_with(
        registry_from_manifests([(SECRET_MANIFEST, "/system/extensions/secret-tool")]),
        Arc::new(PanicAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "secret-tool",
        "/system/extensions/secret-tool/manifest.toml",
        vec![EffectKind::UseSecret],
    )])))
    .with_runtime_policy(secret_denied_runtime_policy());

    let surface = runtime
        .visible_capabilities(request_with_provider_trust(
            context_with_secret_grant(),
            vec![("secret-tool", vec![EffectKind::UseSecret])],
        ))
        .await
        .unwrap();

    assert!(
        surface.capabilities.is_empty(),
        "SecretMode::Deny must hide tools that require secret access"
    );
}

#[tokio::test]
async fn runtime_policy_denied_extension_invoke_does_not_dispatch() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let dispatcher_for_runtime: Arc<dyn CapabilityDispatcher> = dispatcher.clone();
    let runtime = runtime_with_dispatcher(
        registry_from_manifests([(ECHO_NETWORK_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
        dispatcher_for_runtime,
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )])))
    .with_runtime_policy(network_denied_runtime_policy());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context_with_grants([(
                capability_id("echo.say"),
                vec![EffectKind::DispatchCapability, EffectKind::Network],
            )]),
            capability_id("echo.say"),
            ResourceEstimate::default(),
            json!({"message": "blocked before dispatch"}),
            trust_decision_for(vec![EffectKind::DispatchCapability, EffectKind::Network]),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected runtime-policy failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert_eq!(failure.capability_id, capability_id("echo.say"));
    assert!(
        failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("NetworkMode::Deny")
    );
    assert!(
        !dispatcher.has_request(),
        "runtime-policy denial must happen before generic extension dispatch"
    );
}

#[tokio::test]
async fn runtime_policy_denied_secret_invoke_does_not_dispatch() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let dispatcher_for_runtime: Arc<dyn CapabilityDispatcher> = dispatcher.clone();
    let runtime = runtime_with_dispatcher(
        registry_from_manifests([(SECRET_MANIFEST, "/system/extensions/secret-tool")]),
        Arc::new(GrantAuthorizer),
        dispatcher_for_runtime,
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "secret-tool",
        "/system/extensions/secret-tool/manifest.toml",
        vec![EffectKind::UseSecret],
    )])))
    .with_runtime_policy(secret_denied_runtime_policy());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context_with_secret_grant(),
            capability_id("secret-tool.read"),
            ResourceEstimate::default(),
            json!({}),
            trust_decision_for(vec![EffectKind::UseSecret]),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected runtime-policy failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert_eq!(failure.capability_id, capability_id("secret-tool.read"));
    assert!(
        failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("SecretMode::Deny")
    );
    assert!(
        !dispatcher.has_request(),
        "runtime-policy secret denial must happen before generic extension dispatch"
    );
}

#[tokio::test]
async fn runtime_policy_denied_mcp_http_invoke_does_not_dispatch_when_effect_underdeclared() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let dispatcher_for_runtime: Arc<dyn CapabilityDispatcher> = dispatcher.clone();
    let runtime = runtime_with_dispatcher(
        registry_from_manifests([(MCP_UNDERDECLARED_NETWORK_MANIFEST, "/system/extensions/mcp")]),
        Arc::new(GrantAuthorizer),
        dispatcher_for_runtime,
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "mcp",
        "/system/extensions/mcp/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])))
    .with_runtime_policy(network_denied_runtime_policy());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context_with_grants([(
                capability_id("mcp.search"),
                vec![EffectKind::DispatchCapability],
            )]),
            capability_id("mcp.search"),
            ResourceEstimate::default(),
            json!({"query": "blocked before mcp dispatch"}),
            trust_decision_for(vec![EffectKind::DispatchCapability]),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected runtime-policy failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert_eq!(failure.capability_id, capability_id("mcp.search"));
    assert!(
        failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("NetworkMode::Deny")
    );
    assert!(
        !dispatcher.has_request(),
        "runtime-policy denial must happen before MCP dispatch"
    );
}

#[tokio::test]
async fn visible_surface_rejects_invalid_execution_context() {
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    );
    let mut context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    context.resource_scope.invocation_id = InvocationId::new();

    let error = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ironclaw_host_runtime::HostRuntimeError::InvalidRequest { .. }
    ));
}

#[tokio::test]
async fn visible_surface_debug_does_not_expose_authority_internals() {
    let registry = registry_from_manifests([(SECRET_MANIFEST, "/system/extensions/secret-tool")]);
    let runtime = runtime_with(registry, Arc::new(GrantAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "secret-tool",
            "/system/extensions/secret-tool/manifest.toml",
            vec![EffectKind::UseSecret],
        )]),
    ));
    let context = context_with_secret_grant();

    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();
    let debug = format!("{surface:?}");

    assert_eq!(surface.capabilities.len(), 1);
    assert!(!debug.contains("sentinel_secret"));
    assert!(!debug.contains("/private/sentinel"));
    assert!(!debug.contains("approval_store"));
    assert!(!debug.contains("lease"));
}

fn visible_request(context: ExecutionContext) -> VisibleCapabilityRequest {
    request_with_provider_trust(context, default_provider_trust())
}

fn request_with_provider_trust(
    context: ExecutionContext,
    provider_trust: impl IntoIterator<Item = (&'static str, Vec<EffectKind>)>,
) -> VisibleCapabilityRequest {
    VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(provider_trust_for(provider_trust))
}

fn default_provider_trust() -> Vec<(&'static str, Vec<EffectKind>)> {
    vec![
        ("echo", vec![EffectKind::DispatchCapability]),
        ("files", vec![EffectKind::ReadFilesystem]),
        ("net", vec![EffectKind::Network]),
        ("scripts", vec![EffectKind::ExecuteCode]),
        ("secret-tool", vec![EffectKind::UseSecret]),
    ]
}

fn provider_trust_for(
    entries: impl IntoIterator<Item = (&'static str, Vec<EffectKind>)>,
) -> BTreeMap<ExtensionId, TrustDecision> {
    entries
        .into_iter()
        .map(|(provider, effects)| {
            (
                ExtensionId::new(provider).unwrap(),
                trust_decision_for(effects),
            )
        })
        .collect()
}

fn trust_decision_for(allowed_effects: Vec<EffectKind>) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn combined_descriptor_effects(package: &ExtensionPackage) -> Vec<EffectKind> {
    let mut effects = Vec::new();
    for descriptor in &package.capabilities {
        for effect in &descriptor.effects {
            if !effects.contains(effect) {
                effects.push(*effect);
            }
        }
    }
    effects
}

fn assert_schema_has_property(
    surface: &VisibleCapabilitySurface,
    capability: &str,
    property: &str,
) {
    let schema = &surface
        .capabilities
        .iter()
        .find(|entry| entry.descriptor.id == capability_id(capability))
        .unwrap_or_else(|| panic!("{capability} should be visible"))
        .descriptor
        .parameters_schema;
    assert!(
        schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|properties| properties.contains_key(property)),
        "{capability} schema should expose property {property}, got {schema:?}"
    );
}

fn visible_ids(surface: &VisibleCapabilitySurface) -> Vec<CapabilityId> {
    surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.clone())
        .collect()
}

fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn network_denied_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn secret_denied_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::Deny,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn hot_catalog_fixture(
    input_schema: Option<&str>,
    output_schema: &str,
    prompt_doc: &str,
) -> (tempfile::TempDir, LocalFilesystem, ExtensionRegistry) {
    hot_catalog_fixture_with_prompt_bytes(input_schema, output_schema, prompt_doc.as_bytes())
}

fn hot_catalog_fixture_with_prompt_bytes(
    input_schema: Option<&str>,
    output_schema: &str,
    prompt_doc: &[u8],
) -> (tempfile::TempDir, LocalFilesystem, ExtensionRegistry) {
    let manifest = ExtensionManifest::parse(
        HOT_CAPABILITY_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap();
    hot_catalog_fixture_with_manifest(input_schema, output_schema, prompt_doc, manifest)
}

fn hot_catalog_fixture_with_manifest(
    input_schema: Option<&str>,
    output_schema: &str,
    prompt_doc: &[u8],
    manifest: ExtensionManifest,
) -> (tempfile::TempDir, LocalFilesystem, ExtensionRegistry) {
    let storage = tempdir().unwrap();
    let extension_root = storage.path().join("echo");
    std::fs::create_dir_all(extension_root.join("schemas/echo")).unwrap();
    std::fs::create_dir_all(extension_root.join("prompts/echo")).unwrap();
    if let Some(input_schema) = input_schema {
        std::fs::write(
            extension_root.join("schemas/echo/say.input.v1.json"),
            input_schema,
        )
        .unwrap();
    }
    std::fs::write(
        extension_root.join("schemas/echo/say.output.v1.json"),
        output_schema,
    )
    .unwrap();
    std::fs::write(extension_root.join("prompts/echo/say.md"), prompt_doc).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();

    (storage, fs, registry)
}

fn manifest_without_prompt_doc_ref() -> ExtensionManifest {
    let mut manifest = ExtensionManifest::parse(
        HOT_CAPABILITY_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap();
    manifest.capabilities[0].prompt_doc_ref = None;
    manifest
}

fn manifest_with_visibility(visibility: CapabilityVisibility) -> ExtensionManifest {
    let mut manifest = ExtensionManifest::parse(
        HOT_CAPABILITY_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap();
    manifest.capabilities[0].visibility = visibility;
    manifest.capabilities[0].prompt_doc_ref = None;
    manifest
}

struct OversizedReadFilesystem;

#[async_trait]
impl RootFilesystem for OversizedReadFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::ListDir,
        })
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        Ok(FileStat {
            path: path.clone(),
            file_type: FileType::File,
            len: 1,
            modified: None,
            sensitive: false,
        })
    }

    async fn read_file_bounded(
        &self,
        _path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        Ok(Some(vec![b'a'; max_bytes + 1]))
    }
}

fn runtime_with(
    registry: ExtensionRegistry,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
) -> DefaultHostRuntime {
    runtime_with_dispatcher(
        registry,
        authorizer,
        Arc::new(RecordingDispatcher::default()),
    )
}

fn runtime_with_dispatcher(
    registry: ExtensionRegistry,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    dispatcher: Arc<dyn CapabilityDispatcher>,
) -> DefaultHostRuntime {
    DefaultHostRuntime::new(
        Arc::new(registry),
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_dev_runtime_policy(),
    )
}

fn registry_from_manifests<const N: usize>(manifests: [(&str, &str); N]) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    for (manifest, root) in manifests {
        let manifest = parse_manifest(manifest);
        let package =
            ExtensionPackage::from_manifest(manifest, VirtualPath::new(root).unwrap()).unwrap();
        registry.insert(package).unwrap();
    }
    registry
}

fn parse_manifest(manifest: &str) -> ExtensionManifest {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap()
}

fn trust_policy_for<const N: usize>(
    entries: [(&str, &str, Vec<EffectKind>); N],
) -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(
        entries
            .into_iter()
            .map(|(package_id, path, effects)| {
                AdminEntry::for_local_manifest(
                    PackageId::new(package_id).unwrap(),
                    path.to_string(),
                    None,
                    HostTrustAssignment::user_trusted(),
                    effects,
                    None,
                )
            })
            .collect::<Vec<_>>(),
    ))])
    .unwrap()
}

fn context_with_grants<const N: usize>(
    grants: [(CapabilityId, Vec<EffectKind>); N],
) -> ExecutionContext {
    context_with_grant_entries(grants)
}

fn context_with_grant_entries(
    grants: impl IntoIterator<Item = (CapabilityId, Vec<EffectKind>)>,
) -> ExecutionContext {
    let grants = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|(capability, allowed_effects)| CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability,
                grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects,
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            })
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn context_with_secret_grant() -> ExecutionContext {
    let mut context = context_with_grants([]);
    context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id("secret-tool.read"),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::UseSecret],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: vec![SecretHandle::new("sentinel_secret").unwrap()],
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    context
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
    trust_decision_for(vec![EffectKind::DispatchCapability])
}

struct PanicTrustPolicy;

impl TrustPolicy for PanicTrustPolicy {
    fn evaluate(&self, _input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
        panic!("visible surface must use caller-supplied provider_trust, not host trust policy")
    }
}

#[derive(Default)]
struct RecordingDispatcher {
    request: Mutex<Option<CapabilityDispatchRequest>>,
}

impl RecordingDispatcher {
    fn has_request(&self) -> bool {
        self.request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

#[async_trait]
impl CapabilityDispatcher for RecordingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self
            .request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.clone());
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: ExtensionId::new("echo").unwrap(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

#[derive(Default)]
struct CountingGrantAuthorizer {
    calls: AtomicUsize,
}

impl CountingGrantAuthorizer {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for CountingGrantAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        GrantAuthorizer::new()
            .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
            .await
    }
}

struct ApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: descriptor.id.clone(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

struct PanicAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for PanicAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        if descriptor.id != capability_id("echo.say") {
            panic!("policy filters must skip authorizer for disallowed descriptors")
        }
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}

const HOT_CAPABILITY_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
prompt_doc_ref = "prompts/echo/say.md"
"#;

const ECHO_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;

const ECHO_NETWORK_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input over network"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = {}
"#;

const MCP_UNDERDECLARED_NETWORK_MANIFEST: &str = r#"
id = "mcp"
name = "MCP Search"
version = "0.1.0"
description = "MCP integration extension"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/rpc"

[[capabilities]]
id = "mcp.search"
description = "Search through MCP"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;

const ECHO_MANIFEST_WITH_SCHEMA: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object", properties = { message = { type = "string" } } }
"#;

const ECHO_MANIFEST_WITH_DESCRIPTION: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes transformed input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;

const FILES_MANIFEST: &str = r#"
id = "files"
name = "Files"
version = "0.1.0"
description = "File reader"
trust = "third_party"

[runtime]
kind = "wasm"
module = "files.wasm"

[[capabilities]]
id = "files.read"
description = "Reads files"
effects = ["read_filesystem"]
default_permission = "allow"
parameters_schema = {}
"#;

const NET_MANIFEST: &str = r#"
id = "net"
name = "Network"
version = "0.1.0"
description = "Network fetcher"
trust = "third_party"

[runtime]
kind = "wasm"
module = "net.wasm"

[[capabilities]]
id = "net.fetch"
description = "Fetches URLs"
effects = ["network"]
default_permission = "allow"
parameters_schema = {}
"#;

const SECRET_MANIFEST: &str = r#"
id = "secret-tool"
name = "Secret Tool"
version = "0.1.0"
description = "Uses one secret"
trust = "third_party"

[runtime]
kind = "wasm"
module = "secret.wasm"

[[capabilities]]
id = "secret-tool.read"
description = "Uses a scoped secret"
effects = ["use_secret"]
default_permission = "allow"
parameters_schema = {}
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "scripts"
name = "Scripts"
version = "0.1.0"
description = "Script runner"
trust = "third_party"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "scripts.run"
description = "Runs a script"
effects = ["execute_code"]
default_permission = "allow"
parameters_schema = {}
"#;

const SCRIPT_UNDERDECLARED_PROCESS_MANIFEST: &str = r#"
id = "scripts"
name = "Scripts"
version = "0.1.0"
description = "Script runner"
trust = "third_party"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "scripts.run"
description = "Runs a script"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;
