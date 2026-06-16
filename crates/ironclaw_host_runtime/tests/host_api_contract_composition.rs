use ironclaw_extensions::{
    CAPABILITY_PROVIDER_HOST_API_ID, CAPABILITY_PROVIDER_SECTION, ExtensionError, ManifestV2Error,
};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostPath, HostPortId, VirtualPath,
};
use ironclaw_host_runtime::discover_extensions_with_default_host_api_contracts;
use ironclaw_product_adapter_registry::PRODUCT_ADAPTER_HOST_API_ID;
use tempfile::tempdir;

#[tokio::test]
async fn default_host_api_contracts_discover_capability_provider_manifest() {
    let (_storage, fs) = mounted_extension_fs("telegram", CAPABILITY_PROVIDER_MANIFEST);

    let registry = discover_extensions_with_default_host_api_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .unwrap();

    let package = registry
        .get_extension(&ExtensionId::new("telegram").unwrap())
        .unwrap();
    assert_eq!(package.manifest.host_apis.len(), 1);
    assert_eq!(
        package.manifest.host_apis[0].id.as_str(),
        CAPABILITY_PROVIDER_HOST_API_ID
    );
    assert_eq!(
        package.manifest.host_apis[0].section.as_str(),
        CAPABILITY_PROVIDER_SECTION
    );
    assert_eq!(package.capabilities.len(), 1);
    assert_eq!(
        package.manifest.capabilities[0].required_host_ports,
        vec![HostPortId::new(HOST_RUNTIME_HTTP_EGRESS_PORT_ID).unwrap()]
    );

    let capability = registry
        .get_capability(&CapabilityId::new("telegram.send_message").unwrap())
        .unwrap();
    assert_eq!(capability.provider.as_str(), "telegram");
    assert_eq!(capability.description, "Send a Telegram message");
}

#[tokio::test]
async fn default_host_api_contracts_discover_product_adapter_manifest() {
    let (_storage, fs) = mounted_extension_fs("telegram-v2", PRODUCT_ADAPTER_MANIFEST);

    let registry = discover_extensions_with_default_host_api_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .unwrap();

    let package = registry
        .get_extension(&ExtensionId::new("telegram-v2").unwrap())
        .unwrap();
    assert_eq!(package.manifest.host_apis.len(), 1);
    assert_eq!(
        package.manifest.host_apis[0].id.as_str(),
        PRODUCT_ADAPTER_HOST_API_ID
    );
    assert_eq!(
        package.manifest.host_apis[0].section.as_str(),
        "product_adapter.inbound"
    );
    assert!(package.capabilities.is_empty());
}

#[tokio::test]
async fn default_host_port_catalog_rejects_unknown_required_port() {
    let manifest = CAPABILITY_PROVIDER_MANIFEST.replace(
        HOST_RUNTIME_HTTP_EGRESS_PORT_ID,
        "host.runtime.not_supported",
    );
    let (_storage, fs) = mounted_extension_fs("telegram", &manifest);

    let err = discover_extensions_with_default_host_api_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { ref reason, .. })
                if reason.contains("unknown host port 'host.runtime.not_supported'")
        ),
        "unexpected error: {err:?}"
    );
}

fn mounted_extension_fs(id: &str, manifest: &str) -> (tempfile::TempDir, LocalFilesystem) {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join(id)).unwrap();
    std::fs::write(storage.path().join(id).join("manifest.toml"), manifest).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    (storage, fs)
}

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
required_host_ports = ["host.runtime.http_egress"]
"#;

const PRODUCT_ADAPTER_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "telegram-v2"
name = "Telegram"
version = "0.1.0"
description = "Telegram product adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/telegram-v2.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "shared_secret_header"
header_name = "X-Telegram-Bot-Api-Secret-Token"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages", "external_final_reply_push"]

[[product_adapter.inbound.required_credentials]]
handle = "telegram_bot_token"

[[product_adapter.inbound.egress]]
host = "api.telegram.org"
credential_handle = "telegram_bot_token"
"#;
