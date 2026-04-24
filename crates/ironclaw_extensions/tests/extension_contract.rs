use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use tempfile::tempdir;

#[test]
fn valid_wasm_manifest_parses_and_extracts_capability_descriptor() {
    let manifest = ExtensionManifest::parse(WASM_MANIFEST).unwrap();
    assert_eq!(manifest.id.as_str(), "echo");
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
fn script_runtime_keeps_docker_metadata_without_execution() {
    let manifest = ExtensionManifest::parse(SCRIPT_MANIFEST).unwrap();
    assert_eq!(manifest.runtime_kind(), RuntimeKind::Script);
    assert!(matches!(
        manifest.runtime,
        ExtensionRuntime::Script {
            ref backend,
            ref image,
            ref command,
            ref args,
        } if backend == "docker" && image == "python:3.12-slim" && command == "pytest" && args == &["tests/".to_string()]
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
fn registry_rejects_duplicate_extension_ids_and_capability_ids() {
    let package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let duplicate_extension = package.clone();
    let mut duplicate_capability = package.clone();
    duplicate_capability.id = ExtensionId::new("echo2").unwrap();
    duplicate_capability.capabilities[0].provider = ExtensionId::new("echo2").unwrap();

    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();

    assert!(matches!(
        registry.insert(duplicate_extension),
        Err(ExtensionError::DuplicateExtension { .. })
    ));
    assert!(matches!(
        registry.insert(duplicate_capability),
        Err(ExtensionError::DuplicateCapability { .. })
    ));
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
trust = "sandbox"

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

const SCRIPT_MANIFEST: &str = r#"
id = "project-tools"
name = "Project Tools"
version = "0.1.0"
description = "Project-local CLI helpers"
trust = "sandbox"

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

const MCP_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "user_trusted"

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
