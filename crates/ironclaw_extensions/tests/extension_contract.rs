use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_trust::TrustPolicyInput;
use tempfile::tempdir;

#[test]
fn valid_wasm_manifest_parses_and_extracts_capability_descriptor() {
    let manifest = ExtensionManifest::parse(WASM_MANIFEST).unwrap();
    assert_eq!(manifest.id.as_str(), "echo");
    assert_eq!(manifest.requested_trust, RequestedTrustClass::Untrusted);
    assert_eq!(manifest.trust, TrustClass::Sandbox);
    assert!(matches!(
        manifest.runtime,
        ExtensionRuntime::Wasm { ref module } if module.as_str() == "wasm/echo.wasm"
    ));

    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    assert_eq!(package.capabilities.len(), 1);

    let descriptor = &package.capabilities[0];
    assert_eq!(descriptor.id.as_str(), "echo.say");
    assert_eq!(descriptor.provider.as_str(), "echo");
    assert_eq!(descriptor.runtime, RuntimeKind::Wasm);
    assert_eq!(descriptor.trust_ceiling, TrustClass::Sandbox);
    assert_eq!(descriptor.default_permission, PermissionMode::Allow);
    assert_eq!(descriptor.effects, vec![EffectKind::DispatchCapability]);
    assert_eq!(descriptor.parameters_schema["type"], "object");
}

#[test]
fn manifest_privileged_trust_request_is_metadata_and_descriptor_stays_sandboxed() {
    let manifest = ExtensionManifest::parse(
        &WASM_MANIFEST.replace("trust = \"untrusted\"", "trust = \"first_party_requested\""),
    )
    .unwrap();

    assert_eq!(
        manifest.requested_trust,
        RequestedTrustClass::FirstPartyRequested
    );
    assert_eq!(manifest.trust, TrustClass::Sandbox);

    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    assert_eq!(package.capabilities[0].trust_ceiling, TrustClass::Sandbox);
}

#[test]
fn package_builds_trust_policy_input_from_requested_manifest_trust() {
    let manifest = ExtensionManifest::parse(
        &WASM_MANIFEST.replace("trust = \"untrusted\"", "trust = \"first_party_requested\""),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();

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
    assert_eq!(
        input.requested_trust,
        RequestedTrustClass::FirstPartyRequested
    );
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
    let manifest = ExtensionManifest::parse(WASM_MANIFEST).unwrap();
    let mut package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
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
fn missing_manifest_trust_defaults_to_untrusted() {
    let manifest =
        ExtensionManifest::parse(&WASM_MANIFEST.replace("trust = \"untrusted\"\n", "")).unwrap();

    assert_eq!(manifest.requested_trust, RequestedTrustClass::Untrusted);
    assert_eq!(manifest.trust, TrustClass::Sandbox);
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    assert_eq!(package.capabilities[0].trust_ceiling, TrustClass::Sandbox);
}

#[test]
fn legacy_manifest_trust_values_get_actionable_error() {
    let err = ExtensionManifest::parse(
        &WASM_MANIFEST.replace("trust = \"untrusted\"", "trust = \"sandbox\""),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestParse { reason }
            if reason.contains("trust = \"sandbox\" is obsolete")
                && reason.contains("use \"untrusted\"")
    ));
}

#[test]
fn invalid_extension_id_is_rejected() {
    let err =
        ExtensionManifest::parse(&WASM_MANIFEST.replace("id = \"echo\"", "id = \"Echo/Bad\""))
            .unwrap_err();
    assert!(matches!(err, ExtensionError::Contract(_)));
}

#[test]
fn capability_id_must_be_prefixed_by_provider_extension() {
    let manifest =
        ExtensionManifest::parse(&WASM_MANIFEST.replace("echo.say", "other.say")).unwrap();
    let err = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::InvalidManifest { reason } if reason.contains("provider-prefixed")
    ));
}

#[test]
fn script_runtime_keeps_runner_metadata_without_execution() {
    let manifest = ExtensionManifest::parse(SCRIPT_MANIFEST).unwrap();
    assert_eq!(manifest.runtime_kind(), RuntimeKind::Script);
    assert!(matches!(
        manifest.runtime,
        ExtensionRuntime::Script {
            ref runner,
            image: Some(ref image),
            ref command,
            ref args,
        } if runner == "docker" && image == "python:3.12-slim" && command == "pytest" && args == &["tests/".to_string()]
    ));

    let descriptor = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/project-tools").unwrap(),
    )
    .unwrap()
    .capabilities
    .remove(0);
    assert_eq!(descriptor.runtime, RuntimeKind::Script);
    assert_eq!(descriptor.effects, vec![EffectKind::ExecuteCode]);
}

#[test]
fn mcp_runtime_keeps_transport_metadata_without_connecting() {
    let manifest = ExtensionManifest::parse(MCP_MANIFEST).unwrap();
    assert_eq!(manifest.runtime_kind(), RuntimeKind::Mcp);
    assert_eq!(manifest.requested_trust, RequestedTrustClass::ThirdParty);
    assert_eq!(manifest.trust, TrustClass::UserTrusted);
    assert!(matches!(
        manifest.runtime,
        ExtensionRuntime::Mcp {
            ref transport,
            ref command,
            ref args,
            url: None,
        } if transport == "stdio" && command.as_deref() == Some("github-mcp-server") && args == &["--stdio".to_string()]
    ));
}

#[test]
fn invalid_manifest_asset_paths_are_rejected() {
    for invalid in [
        "/Users/alice/echo.wasm",
        "/workspace/echo.wasm",
        "../echo.wasm",
        "wasm\\\\echo.wasm",
        "https://example.com/echo.wasm",
        "wasm/has\\u0000nul.wasm",
        "C:evil.wasm",
    ] {
        let manifest = WASM_MANIFEST.replace("wasm/echo.wasm", invalid);
        assert!(
            matches!(
                ExtensionManifest::parse(&manifest),
                Err(ExtensionError::InvalidAssetPath { .. })
            ),
            "{invalid:?} should be rejected"
        );
    }
}

#[test]
fn registry_rejects_duplicate_extension_ids_and_mutated_packages() {
    let package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let duplicate_extension = package.clone();
    let mut mutated_package = package.clone();
    mutated_package.id = ExtensionId::new("echo2").unwrap();
    mutated_package.capabilities[0].provider = ExtensionId::new("echo2").unwrap();

    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();

    assert!(matches!(
        registry.insert(duplicate_extension),
        Err(ExtensionError::DuplicateExtension { .. })
    ));
    assert!(matches!(
        registry.insert(mutated_package),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("does not match")
    ));
}

#[test]
fn registry_revalidates_public_package_descriptors_against_manifest() {
    let mut package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    package.capabilities[0].runtime = RuntimeKind::System;

    let mut registry = ExtensionRegistry::new();
    assert!(matches!(
        registry.insert(package),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("manifest")
    ));
}

#[test]
fn registry_rejects_duplicate_capability_ids_within_inserted_package() {
    let mut package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    package.capabilities.push(package.capabilities[0].clone());

    let mut registry = ExtensionRegistry::new();
    assert!(matches!(
        registry.insert(package),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("manifest")
    ));
}

#[test]
fn registry_capabilities_iterate_in_extension_and_manifest_order() {
    let alpha = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(ORDERED_CAPABILITIES_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/ordered").unwrap(),
    )
    .unwrap();
    let beta_manifest = ORDERED_CAPABILITIES_MANIFEST
        .replace("id = \"ordered\"", "id = \"beta\"")
        .replace("ordered.", "beta.");
    let beta = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(&beta_manifest).unwrap(),
        VirtualPath::new("/system/extensions/beta").unwrap(),
    )
    .unwrap();

    let mut registry = ExtensionRegistry::new();
    registry.insert(alpha).unwrap();
    registry.insert(beta).unwrap();

    let ids: Vec<_> = registry
        .capabilities()
        .map(|descriptor| descriptor.id.as_str().to_string())
        .collect();

    assert_eq!(
        ids,
        vec![
            "ordered.alpha",
            "ordered.bravo",
            "ordered.charlie",
            "ordered.delta",
            "ordered.echo",
            "beta.alpha",
            "beta.bravo",
            "beta.charlie",
            "beta.delta",
            "beta.echo",
        ]
    );
}

#[tokio::test]
async fn discovery_reads_manifests_from_filesystem_virtual_root() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();
    std::fs::write(storage.path().join("echo/manifest.toml"), WASM_MANIFEST).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
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
async fn discovery_rejects_missing_manifest() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let err = ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(err, ExtensionError::Filesystem(_)));
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

    let err = ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
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

const WASM_MANIFEST: &str = r#"
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
parameters_schema = { type = "object" }
"#;

const ORDERED_CAPABILITIES_MANIFEST: &str = r#"
id = "ordered"
name = "Ordered"
version = "0.1.0"
description = "Ordered capability demo extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/ordered.wasm"

[[capabilities]]
id = "ordered.alpha"
description = "First capability"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }

[[capabilities]]
id = "ordered.bravo"
description = "Second capability"
effects = ["dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }

[[capabilities]]
id = "ordered.charlie"
description = "Third capability"
effects = ["dispatch_capability"]
default_permission = "deny"
parameters_schema = { type = "object" }

[[capabilities]]
id = "ordered.delta"
description = "Fourth capability"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }

[[capabilities]]
id = "ordered.echo"
description = "Fifth capability"
effects = ["dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "project-tools"
name = "Project Tools"
version = "0.1.0"
description = "Project-local CLI helpers"
trust = "untrusted"

[runtime]
kind = "script"
backend = "docker"
image = "python:3.12-slim"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "project-tools.pytest"
description = "Run pytest"
effects = ["execute_code"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const SCRIPT_RUNNER_MANIFEST: &str = r#"
id = "project-tools"
name = "Project Tools"
version = "0.1.0"
description = "Project-local CLI helpers"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "project-tools.pytest"
description = "Run pytest"
effects = ["execute_code"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const MCP_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
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
parameters_schema = { type = "object" }
"#;

#[test]
fn malformed_or_incomplete_manifest_fails() {
    assert!(matches!(
        ExtensionManifest::parse("not = [valid"),
        Err(ExtensionError::ManifestParse { .. })
    ));

    let missing_name = WASM_MANIFEST.replace("name = \"Echo\"\n", "");
    assert!(matches!(
        ExtensionManifest::parse(&missing_name),
        Err(ExtensionError::ManifestParse { .. })
    ));

    let unknown_field =
        WASM_MANIFEST.replace("version = \"0.1.0\"", "version = \"0.1.0\"\nunknown = true");
    assert!(matches!(
        ExtensionManifest::parse(&unknown_field),
        Err(ExtensionError::ManifestParse { .. })
    ));

    let no_capabilities = r#"
id = "empty"
name = "Empty"
version = "0.1.0"
description = "No capabilities"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/empty.wasm"
"#;
    assert!(matches!(
        ExtensionManifest::parse(no_capabilities),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("capability")
    ));
}

#[test]
fn package_root_must_match_manifest_id() {
    let manifest = ExtensionManifest::parse(WASM_MANIFEST).unwrap();
    let err = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/not-echo").unwrap(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::ManifestIdMismatch { expected, actual, .. }
            if expected.as_str() == "not-echo" && actual.as_str() == "echo"
    ));
}

#[test]
fn package_root_must_be_direct_extension_directory() {
    for root in [
        "/system/extensions",
        "/system/extensions/echo/nested",
        "/projects/echo",
    ] {
        let manifest = ExtensionManifest::parse(WASM_MANIFEST).unwrap();
        let err =
            ExtensionPackage::from_manifest(manifest, VirtualPath::new(root).unwrap()).unwrap_err();

        assert!(
            matches!(
                err,
                ExtensionError::InvalidManifest { reason }
                    if reason.contains("/system/extensions/<extension>")
            ),
            "{root:?} should be rejected as an invalid package root"
        );
    }
}

#[test]
fn package_rejects_duplicate_capabilities_within_manifest() {
    let duplicate_manifest = WASM_MANIFEST.to_string()
        + r#"
[[capabilities]]
id = "echo.say"
description = "Duplicate Echo"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
    let manifest = ExtensionManifest::parse(&duplicate_manifest).unwrap();
    let err = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::DuplicateCapability { id } if id.as_str() == "echo.say"
    ));
}

#[test]
fn script_runtime_accepts_semantic_runner_without_docker_backend() {
    let manifest = ExtensionManifest::parse(SCRIPT_RUNNER_MANIFEST).unwrap();

    assert!(matches!(
        manifest.runtime,
        ExtensionRuntime::Script {
            ref runner,
            image: None,
            ref command,
            ref args,
        } if runner == "sandboxed_process" && command == "pytest" && args == &["tests/".to_string()]
    ));
}

#[test]
fn script_runtime_rejects_runner_and_legacy_backend_together() {
    let manifest = SCRIPT_MANIFEST.replace(
        "backend = \"docker\"",
        "backend = \"docker\"\nrunner = \"sandboxed_process\"",
    );
    let err = ExtensionManifest::parse(&manifest).unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::InvalidManifest { reason } if reason.contains("either runner or legacy backend")
    ));
}

#[test]
fn first_party_and_system_runtimes_cannot_be_self_asserted_by_manifests() {
    assert!(matches!(
        ExtensionManifest::parse(FIRST_PARTY_MANIFEST),
        Err(ExtensionError::ManifestParse { .. })
    ));
    assert!(matches!(
        ExtensionManifest::parse(SYSTEM_MANIFEST),
        Err(ExtensionError::ManifestParse { .. })
    ));
    assert!(matches!(
        ExtensionManifest::parse(SANDBOX_FIRST_PARTY_RUNTIME_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("host-assigned")
    ));
    assert!(matches!(
        ExtensionManifest::parse(SANDBOX_SYSTEM_RUNTIME_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("host-assigned")
    ));
}

#[test]
fn mcp_runtime_requires_endpoint_shape_for_transport() {
    assert!(matches!(
        ExtensionManifest::parse(MCP_STDIO_WITHOUT_COMMAND_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("stdio") && reason.contains("command")
    ));
    assert!(matches!(
        ExtensionManifest::parse(MCP_STDIO_WITH_URL_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("stdio") && reason.contains("url")
    ));
    assert!(matches!(
        ExtensionManifest::parse(MCP_HTTP_WITHOUT_URL_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("http") && reason.contains("url")
    ));
    assert!(matches!(
        ExtensionManifest::parse(MCP_HTTP_WITH_COMMAND_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("http") && reason.contains("command")
    ));
    assert!(matches!(
        ExtensionManifest::parse(MCP_HTTP_INVALID_URL_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("http") && reason.contains("URL")
    ));
    assert!(matches!(
        ExtensionManifest::parse(MCP_SSE_UNSUPPORTED_URL_SCHEME_MANIFEST),
        Err(ExtensionError::InvalidManifest { reason }) if reason.contains("sse") && reason.contains("http")
    ));
}

#[test]
fn asset_path_resolves_under_extension_root_only() {
    let asset = ExtensionAssetPath::new("wasm/echo.wasm").unwrap();
    let resolved = asset
        .resolve_under(&VirtualPath::new("/system/extensions/echo").unwrap())
        .unwrap();

    assert_eq!(resolved.as_str(), "/system/extensions/echo/wasm/echo.wasm");
}

#[test]
fn registry_lookup_returns_declared_package_and_descriptor() {
    let package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();

    let extension = registry
        .get_extension(&ExtensionId::new("echo").unwrap())
        .unwrap();
    assert_eq!(extension.root.as_str(), "/system/extensions/echo");

    let capability = registry
        .get_capability(&CapabilityId::new("echo.say").unwrap())
        .unwrap();
    assert_eq!(capability.description, "Echo text");
    assert_eq!(capability.provider.as_str(), "echo");
}

#[tokio::test]
async fn discovery_ignores_non_directory_entries_in_extension_root() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();
    std::fs::write(storage.path().join("echo/manifest.toml"), WASM_MANIFEST).unwrap();
    std::fs::write(storage.path().join(".DS_Store"), b"not an extension").unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();

    assert!(
        registry
            .get_extension(&ExtensionId::new("echo").unwrap())
            .is_some()
    );
}

#[tokio::test]
async fn discovery_ignores_non_extension_directories_with_invalid_ids() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo")).unwrap();
    std::fs::write(storage.path().join("echo/manifest.toml"), WASM_MANIFEST).unwrap();
    std::fs::create_dir_all(storage.path().join(".cache")).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();

    assert!(
        registry
            .get_extension(&ExtensionId::new("echo").unwrap())
            .is_some()
    );
}

#[tokio::test]
async fn discovery_returns_extensions_in_deterministic_name_order() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("zeta")).unwrap();
    std::fs::create_dir_all(storage.path().join("alpha")).unwrap();
    std::fs::write(
        storage.path().join("zeta/manifest.toml"),
        WASM_MANIFEST
            .replace("id = \"echo\"", "id = \"zeta\"")
            .replace("echo.say", "zeta.say"),
    )
    .unwrap();
    std::fs::write(
        storage.path().join("alpha/manifest.toml"),
        WASM_MANIFEST
            .replace("id = \"echo\"", "id = \"alpha\"")
            .replace("echo.say", "alpha.say"),
    )
    .unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let ids: Vec<_> = registry
        .extensions()
        .map(|package| package.id.as_str().to_string())
        .collect();

    assert_eq!(ids, vec!["alpha", "zeta"]);
}

const FIRST_PARTY_MANIFEST: &str = r#"
id = "conversation"
name = "Conversation"
version = "0.1.0"
description = "Conversation service"
trust = "first_party"

[runtime]
kind = "first_party"
service = "conversation"

[[capabilities]]
id = "conversation.ingest"
description = "Ingest normalized messages"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const SYSTEM_MANIFEST: &str = r#"
id = "audit"
name = "Audit"
version = "0.1.0"
description = "Audit service"
trust = "system"

[runtime]
kind = "system"
service = "audit"

[[capabilities]]
id = "audit.write"
description = "Write audit event"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const SANDBOX_FIRST_PARTY_RUNTIME_MANIFEST: &str = r#"
id = "conversation"
name = "Conversation"
version = "0.1.0"
description = "Conversation service"
trust = "untrusted"

[runtime]
kind = "first_party"
service = "conversation"

[[capabilities]]
id = "conversation.ingest"
description = "Ingest normalized messages"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const SANDBOX_SYSTEM_RUNTIME_MANIFEST: &str = r#"
id = "audit"
name = "Audit"
version = "0.1.0"
description = "Audit service"
trust = "untrusted"

[runtime]
kind = "system"
service = "audit"

[[capabilities]]
id = "audit.write"
description = "Write audit event"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const MCP_STDIO_WITHOUT_COMMAND_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "stdio"

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const MCP_STDIO_WITH_URL_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "stdio"
url = "http://localhost:3000"

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const MCP_HTTP_WITHOUT_URL_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const MCP_HTTP_WITH_COMMAND_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
command = "github-mcp-server"

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const MCP_HTTP_INVALID_URL_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "not a url"

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const MCP_SSE_UNSUPPORTED_URL_SCHEME_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "sse"
url = "file:///tmp/mcp.sock"

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;
