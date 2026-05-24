use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_trust::TrustPolicyInput;
use tempfile::tempdir;

#[test]
fn v2_manifest_builds_capability_descriptors_from_schema_refs() {
    let manifest = parse_manifest(WASM_MANIFEST);
    assert_eq!(manifest.id.as_str(), "echo");
    assert_eq!(manifest.requested_trust, RequestedTrustClass::Untrusted);
    assert_eq!(manifest.descriptor_trust_default, TrustClass::Sandbox);
    assert!(matches!(
        manifest.runtime,
        ExtensionRuntime::Wasm { ref module } if module.as_str() == "wasm/echo.wasm"
    ));

    let package = package_from_manifest(manifest, "echo");
    assert_eq!(package.capabilities.len(), 1);

    let descriptor = &package.capabilities[0];
    assert_eq!(descriptor.id.as_str(), "echo.say");
    assert_eq!(descriptor.provider.as_str(), "echo");
    assert_eq!(descriptor.runtime, RuntimeKind::Wasm);
    assert_eq!(descriptor.trust_ceiling, TrustClass::Sandbox);
    assert_eq!(descriptor.default_permission, PermissionMode::Allow);
    assert_eq!(descriptor.effects, vec![EffectKind::DispatchCapability]);
    assert_eq!(
        descriptor.parameters_schema,
        serde_json::json!({"$ref": "schemas/echo/say.input.v1.json"})
    );
}

#[test]
fn package_builds_trust_policy_input_from_v2_requested_trust() {
    let manifest =
        parse_manifest(&WASM_MANIFEST.replace("trust = \"untrusted\"", "trust = \"third_party\""));
    let package = package_from_manifest(manifest, "echo");

    let input: TrustPolicyInput = package
        .trust_policy_input(
            PackageSource::LocalManifest {
                path: "/system/extensions/echo/manifest.toml".to_string(),
            },
            Some("sha256:abc".to_string()),
            Some("alice@example.com".to_string()),
        )
        .unwrap();

    assert_eq!(input.identity.package_id.as_str(), "echo");
    assert!(matches!(
        input.identity.source,
        PackageSource::LocalManifest { ref path } if path == "/system/extensions/echo/manifest.toml"
    ));
    assert_eq!(input.identity.digest.as_deref(), Some("sha256:abc"));
    assert_eq!(input.identity.signer.as_deref(), Some("alice@example.com"));
    assert_eq!(input.requested_trust, RequestedTrustClass::ThirdParty);
    assert_eq!(
        input
            .requested_authority
            .iter()
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>(),
        vec!["echo.say"]
    );
}

#[test]
fn package_trust_policy_input_rejects_mutated_public_descriptors() {
    let mut package = package_from_manifest(parse_manifest(WASM_MANIFEST), "echo");
    package.capabilities[0].id = CapabilityId::new("echo.mutated").unwrap();

    let err = package
        .trust_policy_input(PackageSource::Bundled, None, None)
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::InvalidManifest { reason } if reason.contains("capability descriptors")
    ));
}

#[test]
fn registry_rejects_installed_package_with_mutated_parameters_schema() {
    let mut package = package_from_manifest(parse_manifest(WASM_MANIFEST), "echo");
    package.capabilities[0].parameters_schema = serde_json::json!({"type": "object"});

    let err = ExtensionRegistry::new().insert(package).unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::InvalidManifest { reason } if reason.contains("capability descriptors")
    ));
}

#[test]
fn registry_allows_host_bundled_package_with_resolved_parameters_schema() {
    let manifest = ExtensionManifest::parse(
        WASM_MANIFEST,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
    )
    .unwrap();
    let mut package = package_from_manifest(manifest, "echo");
    package.capabilities[0].parameters_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "message": { "type": "string" }
        },
        "required": ["message"],
        "additionalProperties": false
    });

    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();

    let descriptor = registry
        .get_capability(&CapabilityId::new("echo.say").unwrap())
        .unwrap();
    assert_eq!(
        descriptor.parameters_schema,
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    );
}

#[test]
fn script_and_mcp_runtime_metadata_stays_declarative() {
    let script = parse_manifest(SCRIPT_MANIFEST);
    assert_eq!(script.runtime_kind(), RuntimeKind::Script);
    assert!(matches!(
        script.runtime,
        ExtensionRuntime::Script {
            ref runner,
            image: Some(ref image),
            ref command,
            ref args,
        } if runner == "docker" && image == "python:3.12-slim" && command == "pytest" && args == &["tests/".to_string()]
    ));
    assert_eq!(
        package_from_manifest(script, "project-tools").capabilities[0].runtime,
        RuntimeKind::Script
    );

    let mcp = parse_manifest(MCP_MANIFEST);
    assert_eq!(mcp.runtime_kind(), RuntimeKind::Mcp);
    assert_eq!(mcp.requested_trust, RequestedTrustClass::ThirdParty);
    assert_eq!(mcp.descriptor_trust_default, TrustClass::UserTrusted);
    assert!(matches!(
        mcp.runtime,
        ExtensionRuntime::Mcp {
            ref transport,
            ref command,
            ref args,
            url: None,
        } if transport == "stdio" && command.as_deref() == Some("github-mcp-server") && args == &["--stdio".to_string()]
    ));
}

#[tokio::test]
async fn discovery_reads_host_bundled_legacy_manifests_from_filesystem_virtual_root() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();
    std::fs::write(storage.path().join("echo/manifest.toml"), WASM_MANIFEST).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let registry = ExtensionDiscovery::discover_with_manifest_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        &HostApiContractRegistry::new(),
    )
    .await
    .unwrap();

    assert!(
        registry
            .get_extension(&ExtensionId::new("echo").unwrap())
            .is_some()
    );
    assert!(
        registry
            .get_capability(&CapabilityId::new("echo.say").unwrap())
            .is_some()
    );
}

#[tokio::test]
async fn discovery_rejects_installed_local_privileged_manifest() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();
    std::fs::write(
        storage.path().join("echo/manifest.toml"),
        WASM_MANIFEST.replace("trust = \"untrusted\"", "trust = \"first_party_requested\""),
    )
    .unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let err = ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(ManifestV2Error::TrustForbiddenForSource {
            manifest_source: ManifestSource::InstalledLocal,
            requested: RequestedTrustClass::FirstPartyRequested,
        })
    ));
}

#[test]
fn production_parser_rejects_installed_legacy_top_level_capabilities() {
    let err = ExtensionManifest::parse_with_optional_host_api_contracts(
        WASM_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &HostApiContractRegistry::new(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(
            ManifestV2Error::LegacyTopLevelCapabilitiesForInstalledSource {
                manifest_source: ManifestSource::InstalledLocal,
            }
        )
    ));
}

#[test]
fn production_parser_allows_host_bundled_legacy_top_level_capabilities() {
    let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
        WASM_MANIFEST,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        &HostApiContractRegistry::new(),
    )
    .unwrap();

    assert_eq!(manifest.id.as_str(), "echo");
    assert_eq!(manifest.capabilities.len(), 1);
}

#[tokio::test]
async fn discovery_rejects_installed_legacy_top_level_capabilities() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();
    std::fs::write(storage.path().join("echo/manifest.toml"), WASM_MANIFEST).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let err = ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(
            ManifestV2Error::LegacyTopLevelCapabilitiesForInstalledSource {
                manifest_source: ManifestSource::InstalledLocal,
            }
        )
    ));
}

#[tokio::test]
async fn discovery_validates_host_api_manifest_with_supplied_contracts() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("telegram")).unwrap();
    std::fs::write(
        storage.path().join("telegram/manifest.toml"),
        HOST_API_MANIFEST,
    )
    .unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let mut contracts = HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(TestProductAdapterContract {
            id: HostApiId::new("ironclaw.product_adapter/v1").unwrap(),
        }))
        .unwrap();

    let registry = ExtensionDiscovery::discover_with_manifest_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &contracts,
    )
    .await
    .unwrap();

    let package = registry
        .get_extension(&ExtensionId::new("telegram").unwrap())
        .unwrap();
    assert_eq!(package.capabilities.len(), 0);
    assert_eq!(package.manifest.host_apis.len(), 1);
}

#[tokio::test]
async fn discovery_registers_capability_provider_projected_capabilities() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("telegram")).unwrap();
    std::fs::write(
        storage.path().join("telegram/manifest.toml"),
        CAPABILITY_PROVIDER_MANIFEST,
    )
    .unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let registry = ExtensionDiscovery::discover_with_manifest_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .await
    .unwrap();

    let package = registry
        .get_extension(&ExtensionId::new("telegram").unwrap())
        .unwrap();
    assert_eq!(package.capabilities.len(), 1);
    assert_eq!(package.capabilities[0].id.as_str(), "telegram.send_message");
    assert!(
        registry
            .get_capability(&CapabilityId::new("telegram.send_message").unwrap())
            .is_some()
    );
}

#[test]
fn capability_provider_host_api_contract_accepts_valid_manifest() {
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        CAPABILITY_PROVIDER_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap();

    assert_eq!(manifest.id.as_str(), "telegram");
    assert_eq!(manifest.host_apis.len(), 1);
    assert_eq!(
        manifest.host_apis[0].id.as_str(),
        CAPABILITY_PROVIDER_HOST_API_ID
    );
    assert_eq!(
        manifest.host_apis[0].section.as_str(),
        CAPABILITY_PROVIDER_SECTION
    );
    assert_eq!(manifest.capabilities.len(), 1);
    assert_eq!(
        manifest.capabilities[0].id.as_str(),
        "telegram.send_message"
    );

    let package = package_from_manifest(manifest, "telegram");
    assert_eq!(package.capabilities.len(), 1);
    let descriptor = &package.capabilities[0];
    assert_eq!(descriptor.id.as_str(), "telegram.send_message");
    assert_eq!(descriptor.provider.as_str(), "telegram");
    assert_eq!(descriptor.runtime, RuntimeKind::Wasm);
    assert_eq!(descriptor.trust_ceiling, TrustClass::UserTrusted);
    assert_eq!(descriptor.default_permission, PermissionMode::Ask);
    assert_eq!(descriptor.effects, vec![EffectKind::Network]);
    assert_eq!(
        descriptor.parameters_schema,
        serde_json::json!({"$ref": "schemas/telegram/send_message.input.v1.json"})
    );
}

#[test]
fn capability_provider_host_api_projects_runtime_credentials() {
    let manifest = CAPABILITY_PROVIDER_MANIFEST.replace(
        "effects = [\"network\"]",
        r#"effects = ["network", "use_secret"]
runtime_credentials = [
  { handle = "telegram_token", audience = { scheme = "https", host_pattern = "api.telegram.org" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
]"#,
    );
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap();

    let credential = manifest.capabilities[0].runtime_credentials[0].clone();
    assert_eq!(
        credential.handle,
        SecretHandle::new("telegram_token").unwrap()
    );
    assert_eq!(
        credential.audience,
        NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.telegram.org".to_string(),
            port: None,
        }
    );
    assert_eq!(
        credential.target,
        RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        }
    );
    assert!(credential.required);

    let package = package_from_manifest(manifest, "telegram");
    assert_eq!(package.capabilities[0].runtime_credentials.len(), 1);
    assert_eq!(package.capabilities[0].runtime_credentials[0], credential);
}

#[test]
fn capability_provider_host_api_preserves_runtime_credential_validation_errors() {
    let cases = [
        (
            r#"{ handle = "../telegram_token", audience = { scheme = "https", host_pattern = "api.telegram.org" }, target = { type = "header", name = "authorization" } }"#,
            "invalid secret",
        ),
        (
            r#"{ handle = "telegram_token", audience = { scheme = "http", host_pattern = "api.telegram.org" }, target = { type = "header", name = "authorization" } }"#,
            "https scheme",
        ),
        (
            r#"{ handle = "telegram_token", audience = { scheme = "https", host_pattern = "api.telegram.org" }, target = { type = "header", name = "bad header" } }"#,
            "invalid runtime credential target",
        ),
    ];

    for (credential, expected) in cases {
        let manifest = CAPABILITY_PROVIDER_MANIFEST.replace(
            "effects = [\"network\"]",
            &format!(
                r#"effects = ["network", "use_secret"]
runtime_credentials = [
  {credential},
]"#
            ),
        );
        let err = ExtensionManifest::parse_with_host_api_contracts(
            &manifest,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &capability_provider_contracts(),
        )
        .unwrap_err();

        assert!(
            matches!(
                err,
                ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { ref reason, .. })
                    if reason.contains(expected)
            ),
            "expected reason containing {expected:?}, got {err:?}"
        );
    }
}

#[test]
fn capability_provider_host_api_fails_without_registered_contract() {
    let err = ExtensionManifest::parse_with_host_api_contracts(
        CAPABILITY_PROVIDER_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &HostApiContractRegistry::new(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(ManifestV2Error::UnknownHostApi { id })
            if id.as_str() == CAPABILITY_PROVIDER_HOST_API_ID
    ));
}

#[test]
fn capability_provider_host_api_rejects_wrong_section_path() {
    let manifest = CAPABILITY_PROVIDER_MANIFEST
        .replace(
            "section = \"capability_provider.tools\"",
            "section = \"capability_provider.other\"",
        )
        .replace("[capability_provider.tools]", "[capability_provider.other]")
        .replace(
            "[[capability_provider.tools.capabilities]]",
            "[[capability_provider.other.capabilities]]",
        );
    let err = ExtensionManifest::parse_with_host_api_contracts(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { id, section, reason })
            if id.as_str() == CAPABILITY_PROVIDER_HOST_API_ID
                && section.as_str() == "capability_provider.other"
                && reason.contains("section path")
    ));
}

#[test]
fn capability_provider_host_api_rejects_unknown_section_fields() {
    let manifest = CAPABILITY_PROVIDER_MANIFEST.replace(
        "[capability_provider.tools]",
        "[capability_provider.tools]\nraw_secret = \"not allowed\"",
    );
    let err = ExtensionManifest::parse_with_host_api_contracts(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { id, reason, .. })
            if id.as_str() == CAPABILITY_PROVIDER_HOST_API_ID && reason.contains("unknown field")
    ));
}

#[test]
fn capability_provider_host_api_reuses_capability_validation() {
    let cases = [
        (
            CAPABILITY_PROVIDER_MANIFEST.replace("telegram.send_message", "other.send_message"),
            "provider-prefixed",
        ),
        (
            CAPABILITY_PROVIDER_MANIFEST.replace(
                "prompt_doc_ref = \"prompts/telegram/send_message.md\"\n",
                "",
            ),
            "prompt_doc_ref",
        ),
        (
            CAPABILITY_PROVIDER_MANIFEST.replace(
                "effects = [\"network\"]",
                "effects = [\"network\", \"network\"]",
            ),
            "duplicate effect",
        ),
    ];

    for (manifest, expected) in cases {
        let err = ExtensionManifest::parse_with_host_api_contracts(
            &manifest,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &capability_provider_contracts(),
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { ref reason, .. })
                    if reason.contains(expected)
            ),
            "expected reason containing {expected:?}, got {err:?}"
        );
    }
}

#[test]
fn capability_provider_host_api_rejects_duplicate_capability_ids() {
    let manifest = CAPABILITY_PROVIDER_MANIFEST.replace(
        "[[capability_provider.tools.capabilities]]",
        r#"[[capability_provider.tools.capabilities]]
id = "telegram.send_message"
description = "Send a duplicate Telegram message"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/telegram/send_message.input.v1.json"
output_schema_ref = "schemas/telegram/send_message.output.v1.json"
prompt_doc_ref = "prompts/telegram/send_message.md"

[[capability_provider.tools.capabilities]]"#,
    );
    let err = ExtensionManifest::parse_with_host_api_contracts(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { reason, .. })
            if reason.contains("duplicate capability id")
    ));
}

#[test]
fn capability_provider_host_api_rejects_contextless_validation() {
    let contract = CapabilityProviderHostApiContract::new().unwrap();
    let host_api = HostApiRefV2 {
        id: HostApiId::new(CAPABILITY_PROVIDER_HOST_API_ID).unwrap(),
        section: ManifestSectionPath::new(CAPABILITY_PROVIDER_SECTION).unwrap(),
    };
    let section = toml::Value::Table(toml::map::Map::new());

    let err = contract.validate_section(&host_api, &section).unwrap_err();

    assert!(err.contains("requires manifest context"));
}

#[test]
fn capability_provider_host_api_validates_required_host_ports() {
    let manifest = CAPABILITY_PROVIDER_MANIFEST.replace(
        "prompt_doc_ref = \"prompts/telegram/send_message.md\"\n",
        "prompt_doc_ref = \"prompts/telegram/send_message.md\"\nrequired_host_ports = [\"host.runtime.http_egress\"]\n",
    );

    let missing_port = ExtensionManifest::parse_with_host_api_contracts(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap_err();
    assert!(matches!(
        missing_port,
        ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { reason, .. })
            if reason.contains("unknown host port")
    ));

    let catalog = HostPortCatalog::new(vec![HostPortCatalogEntry::new(
        HostPortId::new("host.runtime.http_egress").unwrap(),
    )])
    .unwrap();
    ExtensionManifest::parse_with_host_api_contracts(
        &manifest,
        ManifestSource::InstalledLocal,
        &catalog,
        &capability_provider_contracts(),
    )
    .unwrap();
}

#[test]
fn capability_provider_host_api_rejects_empty_capability_list() {
    let manifest = CAPABILITY_PROVIDER_MANIFEST.replace(
        r#"
[[capability_provider.tools.capabilities]]
id = "telegram.send_message"
description = "Send a Telegram message"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/telegram/send_message.input.v1.json"
output_schema_ref = "schemas/telegram/send_message.output.v1.json"
prompt_doc_ref = "prompts/telegram/send_message.md"
"#,
        "",
    );
    let err = ExtensionManifest::parse_with_host_api_contracts(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { reason, .. })
            if reason.contains("at least one capability")
    ));
}

#[tokio::test]
async fn discovery_validates_capability_manifest_with_supplied_host_port_catalog() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();
    std::fs::write(
        storage.path().join("echo/manifest.toml"),
        WASM_MANIFEST_WITH_HOST_PORT,
    )
    .unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let catalog = HostPortCatalog::new(vec![HostPortCatalogEntry::new(
        HostPortId::new("host.runtime.http_egress").unwrap(),
    )])
    .unwrap();

    let registry = ExtensionDiscovery::discover_with_manifest_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
        ManifestSource::HostBundled,
        &catalog,
        &HostApiContractRegistry::new(),
    )
    .await
    .unwrap();

    let package = registry
        .get_extension(&ExtensionId::new("echo").unwrap())
        .unwrap();
    assert_eq!(
        package.manifest.capabilities[0].required_host_ports,
        vec![HostPortId::new("host.runtime.http_egress").unwrap()]
    );
}

#[tokio::test]
async fn discovery_rejects_manifest_id_mismatch_with_directory() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("wrong-dir")).unwrap();
    std::fs::write(
        storage.path().join("wrong-dir/manifest.toml"),
        WASM_MANIFEST,
    )
    .unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let err = ExtensionDiscovery::discover_with_manifest_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        &HostApiContractRegistry::new(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestIdMismatch {
            expected,
            actual,
            ..
        } if expected.as_str() == "wrong-dir" && actual.as_str() == "echo"
    ));
}

#[test]
fn registry_preserves_extension_and_capability_order() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(lifecycle_package("alpha", "alpha.say", "0.1.0"))
        .unwrap();
    registry
        .insert(lifecycle_package("beta", "beta.say", "0.1.0"))
        .unwrap();

    let extension_ids = registry
        .extensions()
        .map(|package| package.id.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(extension_ids, vec!["alpha", "beta"]);

    let capability_ids = registry
        .capabilities()
        .map(|descriptor| descriptor.id.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(capability_ids, vec!["alpha.say", "beta.say"]);
}

#[test]
fn registry_rejects_duplicate_extension_ids() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(lifecycle_package("alpha", "alpha.say", "0.1.0"))
        .unwrap();

    let err = registry
        .insert(lifecycle_package("alpha", "alpha.reply", "0.2.0"))
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::DuplicateExtension { id } if id.as_str() == "alpha"
    ));
}

#[tokio::test]
async fn lifecycle_service_records_update_disable_enable_and_remove_events() {
    let events = std::sync::Arc::new(RecordingExtensionLifecycleEventSink::default());
    let installed = lifecycle_package("echo", "echo.say", "0.1.0");
    let updated = lifecycle_package("echo", "echo.reply", "0.2.0");
    let extension_id = ExtensionId::new("echo").unwrap();
    let mut service = ExtensionLifecycleService::new(ExtensionRegistry::new())
        .with_event_sink(std::sync::Arc::clone(&events));

    service.install(installed).await.unwrap();
    service.update(updated).await.unwrap();
    service.disable(&extension_id).await.unwrap();
    assert!(!service.is_enabled(&extension_id));
    service.enable(&extension_id).await.unwrap();
    assert!(service.is_enabled(&extension_id));
    service.remove(&extension_id).await.unwrap();

    assert!(service.registry().get_extension(&extension_id).is_none());
    let recorded = events.events();
    assert_eq!(recorded.len(), 5);
    assert_eq!(recorded[0].operation, ExtensionLifecycleOperation::Install);
    assert_eq!(recorded[1].operation, ExtensionLifecycleOperation::Update);
    assert_eq!(recorded[1].version, "0.2.0");
    assert!(recorded[1].capability_surface_changed);
    assert_eq!(recorded[2].operation, ExtensionLifecycleOperation::Disable);
    assert_eq!(recorded[3].operation, ExtensionLifecycleOperation::Enable);
    assert_eq!(recorded[4].operation, ExtensionLifecycleOperation::Remove);
}

#[tokio::test]
async fn lifecycle_service_does_not_install_when_required_event_sink_fails() {
    let package = lifecycle_package("echo", "echo.say", "0.1.0");
    let mut service = ExtensionLifecycleService::new(ExtensionRegistry::new())
        .with_event_sink(std::sync::Arc::new(FailingExtensionLifecycleEventSink));

    let err = service.install(package).await.unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::LifecycleEventSink {
            ref extension_id,
            operation: ExtensionLifecycleOperation::Install,
        } if extension_id.as_str() == "echo"
    ));
    assert!(!err.to_string().contains("raw_sink_failure_sentinel_3022"));
    assert!(
        service
            .registry()
            .get_extension(&ExtensionId::new("echo").unwrap())
            .is_none()
    );
}

fn parse_manifest(raw: &str) -> ExtensionManifest {
    ExtensionManifest::parse(
        raw,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap()
}

fn package_from_manifest(manifest: ExtensionManifest, id: &str) -> ExtensionPackage {
    ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{id}")).unwrap(),
    )
    .unwrap()
}

fn capability_provider_contracts() -> HostApiContractRegistry {
    let mut contracts = HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            CapabilityProviderHostApiContract::new().unwrap(),
        ))
        .unwrap();
    contracts
}

fn lifecycle_package(id: &str, capability: &str, version: &str) -> ExtensionPackage {
    let manifest = v2_manifest(
        id,
        capability,
        version,
        "wasm",
        "module = \"wasm/tool.wasm\"",
    );
    package_from_manifest(parse_manifest(&manifest), id)
}

fn v2_manifest(
    id: &str,
    capability: &str,
    version: &str,
    runtime_kind: &str,
    runtime_fields: &str,
) -> String {
    let schema_prefix = capability.replace('.', "/");
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "{version}"
description = "{id} extension"
trust = "untrusted"

[runtime]
kind = "{runtime_kind}"
{runtime_fields}

[[capabilities]]
id = "{capability}"
description = "Run {capability}"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/{schema_prefix}.input.v1.json"
output_schema_ref = "schemas/{schema_prefix}.output.v1.json"
prompt_doc_ref = "prompts/{schema_prefix}.md"
"#
    )
}

#[derive(Default)]
struct RecordingExtensionLifecycleEventSink {
    events: std::sync::Mutex<Vec<ExtensionLifecycleEvent>>,
}

impl RecordingExtensionLifecycleEventSink {
    fn events(&self) -> Vec<ExtensionLifecycleEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ExtensionLifecycleEventSink for RecordingExtensionLifecycleEventSink {
    async fn record_extension_lifecycle_event(
        &self,
        event: ExtensionLifecycleEvent,
    ) -> Result<(), ExtensionError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

struct FailingExtensionLifecycleEventSink;

#[async_trait::async_trait]
impl ExtensionLifecycleEventSink for FailingExtensionLifecycleEventSink {
    async fn record_extension_lifecycle_event(
        &self,
        event: ExtensionLifecycleEvent,
    ) -> Result<(), ExtensionError> {
        let _ = event;
        Err(ExtensionError::InvalidManifest {
            reason: "raw_sink_failure_sentinel_3022".to_string(),
        })
    }
}

struct TestProductAdapterContract {
    id: HostApiId,
}

impl HostApiManifestContract for TestProductAdapterContract {
    fn id(&self) -> &HostApiId {
        &self.id
    }

    fn accepts_section_path(&self, section: &ManifestSectionPath) -> bool {
        section.as_str() == "product_adapter.inbound"
    }

    fn validate_section(
        &self,
        _host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), String> {
        let surface = section
            .get("surface_kind")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "surface_kind is required".to_string())?;
        if surface == "telegram" {
            Ok(())
        } else {
            Err("surface_kind must be telegram".to_string())
        }
    }
}

const WASM_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
prompt_doc_ref = "prompts/echo/say.md"
"#;

const WASM_MANIFEST_WITH_HOST_PORT: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
prompt_doc_ref = "prompts/echo/say.md"
required_host_ports = ["host.runtime.http_egress"]
"#;

const HOST_API_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "telegram"
name = "Telegram"
version = "0.1.0"
description = "Telegram adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/telegram.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "telegram"
"#;

const CAPABILITY_PROVIDER_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "telegram"
name = "Telegram"
version = "0.1.0"
description = "Telegram adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/telegram.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "telegram.send_message"
description = "Send a Telegram message"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/telegram/send_message.input.v1.json"
output_schema_ref = "schemas/telegram/send_message.output.v1.json"
prompt_doc_ref = "prompts/telegram/send_message.md"
"#;

const SCRIPT_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "project-tools"
name = "Project Tools"
version = "0.1.0"
description = "Project-local CLI helpers"
trust = "untrusted"

[runtime]
kind = "script"
runner = "docker"
image = "python:3.12-slim"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "project-tools.pytest"
description = "Run pytest"
effects = ["execute_code"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/project-tools/pytest.input.v1.json"
output_schema_ref = "schemas/project-tools/pytest.output.v1.json"
prompt_doc_ref = "prompts/project-tools/pytest.md"
"#;

const MCP_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP helper"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "stdio"
command = "github-mcp-server"
args = ["--stdio"]

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/github-mcp/search_issues.input.v1.json"
output_schema_ref = "schemas/github-mcp/search_issues.output.v1.json"
prompt_doc_ref = "prompts/github-mcp/search_issues.md"
"#;
