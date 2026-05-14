//! Extension Manifest v2 contract tests.

use ironclaw_extensions::{
    CapabilityVisibility, ExtensionManifestV2, ExtensionRuntimeV2, MANIFEST_SCHEMA_VERSION,
    ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::{
    CapabilityProfileId, ExtensionId, HostPortCatalog, HostPortCatalogEntry, HostPortId,
    PermissionMode, RequestedTrustClass, RuntimeKind, TrustClass,
};

const TELEGRAM_TOKEN_PORT: &str = "host.secrets.telegram_bot_token";
const AUDIT_PORT: &str = "host.events.audit";
const SQL_TX_PORT: &str = "host.storage.sql_transaction.first_party";

fn catalog() -> HostPortCatalog {
    HostPortCatalog::new(vec![
        HostPortCatalogEntry::new(HostPortId::new(AUDIT_PORT).unwrap()),
        HostPortCatalogEntry::new(HostPortId::new(SQL_TX_PORT).unwrap()),
        HostPortCatalogEntry::new(HostPortId::new(TELEGRAM_TOKEN_PORT).unwrap()),
    ])
    .unwrap()
}

fn third_party_wasm_manifest(extension_id: &str, capability_id: &str) -> String {
    format!(
        r#"
schema_version = "{schema}"
id = "{ext}"
name = "Example Extension"
version = "0.1.0"
description = "test"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/example.wasm"

[[capabilities]]
id = "{cap}"
description = "Echoes input"
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/example/echo.input.v1.json"
output_schema_ref = "schemas/example/echo.output.v1.json"
prompt_doc_ref = "prompt/example/echo.md"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
        ext = extension_id,
        cap = capability_id,
    )
}

#[test]
fn parses_minimum_valid_v2_manifest_for_installed_third_party_extension() {
    let toml = third_party_wasm_manifest("acme-tools", "acme-tools.echo");
    let manifest =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap();

    assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
    assert_eq!(manifest.id, ExtensionId::new("acme-tools").unwrap());
    assert_eq!(manifest.source, ManifestSource::InstalledLocal);
    assert_eq!(manifest.requested_trust, RequestedTrustClass::ThirdParty);
    assert_eq!(manifest.descriptor_trust_default, TrustClass::UserTrusted);
    assert_eq!(manifest.runtime.kind(), RuntimeKind::Wasm);
    assert_eq!(manifest.capabilities.len(), 1);
    let cap = &manifest.capabilities[0];
    assert_eq!(cap.visibility, CapabilityVisibility::Model);
    assert_eq!(cap.default_permission, PermissionMode::Allow);
    assert!(cap.prompt_doc_ref.is_some());
}

#[test]
fn rejects_unknown_top_level_fields() {
    let toml = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"
oops = true

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#;
    let err =
        ExtensionManifestV2::parse(toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(matches!(err, ManifestV2Error::Parse { .. }), "{err:?}");
}

#[test]
fn rejects_first_party_trust_for_installed_source() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(
            err,
            ManifestV2Error::TrustForbiddenForSource {
                manifest_source: ManifestSource::InstalledLocal,
                requested: RequestedTrustClass::FirstPartyRequested,
            }
        ),
        "{err:?}"
    );
}

#[test]
fn rejects_first_party_runtime_for_installed_source() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "first_party"
service = "native_memory_provider"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(
            err,
            ManifestV2Error::RuntimeForbiddenForSource {
                manifest_source: ManifestSource::InstalledLocal,
                kind: RuntimeKind::FirstParty,
            }
        ),
        "{err:?}"
    );
}

#[test]
fn host_bundled_source_may_assert_first_party_and_reserved_id() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "ironclaw.memory.native"
name = "Reborn Native Memory"
version = "0.1.0"
description = "host-bundled"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "native_memory_provider"

[[capabilities]]
id = "ironclaw.memory.native.context.retrieve"
implements = ["memory.context_retrieval.v1"]
description = "Retrieve bounded provider-neutral memory context."
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/memory/context-retrieve.input.v1.json"
output_schema_ref = "schemas/memory/context-retrieve.output.v1.json"
required_host_ports = [
  "host.storage.sql_transaction.first_party",
  "host.events.audit",
]
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let manifest =
        ExtensionManifestV2::parse(&toml, ManifestSource::HostBundled, &catalog()).unwrap();
    assert_eq!(
        manifest.requested_trust,
        RequestedTrustClass::FirstPartyRequested
    );
    // Lock in the v2 contract: `descriptor_trust_default` is a safe
    // pre-policy default. Privileged requests *intentionally* surface as
    // Sandbox here even for HostBundled — effective trust must come from a
    // host trust-policy evaluation, never from this field.
    assert_eq!(manifest.descriptor_trust_default, TrustClass::Sandbox);
    assert!(matches!(
        manifest.runtime,
        ExtensionRuntimeV2::FirstParty { .. }
    ));
    let cap = &manifest.capabilities[0];
    assert_eq!(
        cap.implements,
        vec![CapabilityProfileId::new("memory.context_retrieval.v1").unwrap()]
    );
    assert_eq!(cap.required_host_ports.len(), 2);
}

#[test]
fn rejects_reserved_id_prefix_for_installed_source() {
    let toml = third_party_wasm_manifest("ironclaw.fake", "ironclaw.fake.echo");
    let err = ExtensionManifestV2::parse(&toml, ManifestSource::RegistryInstalled, &catalog())
        .unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::ReservedIdForInstalledSource { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_capability_id_without_provider_prefix() {
    let toml = third_party_wasm_manifest("acme-tools", "other.echo");
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::CapabilityIdNotPrefixed { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_unknown_host_ports() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
required_host_ports = ["host.does.not.exist"]
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::UnknownHostPort { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_model_visible_capability_without_prompt_doc_ref() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::MissingPromptDocRef { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_schema_ref_with_absolute_or_url_or_traversal_paths() {
    for bad_ref in [
        "/schemas/abs.json",
        "../escape.json",
        "https://example.com/schema.json",
        "schemas/with:colon.json",
    ] {
        let toml = format!(
            r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "{bad}"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
            schema = MANIFEST_SCHEMA_VERSION,
            bad = bad_ref,
        );
        let err = ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog())
            .unwrap_err();
        assert!(
            matches!(
                err,
                ManifestV2Error::InvalidSchemaRef {
                    field: "input_schema_ref",
                    ..
                }
            ),
            "{bad_ref:?} should be rejected via InvalidSchemaRef, got {err:?}"
        );
    }
}

#[test]
fn rejects_wrong_schema_version() {
    let toml = r#"
schema_version = "reborn.extension_manifest.v1"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#;
    let err =
        ExtensionManifestV2::parse(toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::SchemaVersion { .. }),
        "{err:?}"
    );
}

#[test]
fn default_trust_is_untrusted_when_field_is_omitted() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let manifest =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap();
    assert_eq!(manifest.requested_trust, RequestedTrustClass::Untrusted);
    assert_eq!(manifest.descriptor_trust_default, TrustClass::Sandbox);
}

#[test]
fn rejects_empty_top_level_name_version_or_description() {
    for (field, value) in [("name", ""), ("version", ""), ("description", "")] {
        let toml = format!(
            r#"
schema_version = "{schema}"
id = "acme-tools"
name = "{name}"
version = "{version}"
description = "{description}"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
            schema = MANIFEST_SCHEMA_VERSION,
            name = if field == "name" { value } else { "x" },
            version = if field == "version" { value } else { "0.1" },
            description = if field == "description" { value } else { "x" },
        );
        let err = ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog())
            .unwrap_err();
        assert!(
            matches!(err, ManifestV2Error::Invalid { .. }),
            "{field}={value:?} should be rejected, got {err:?}"
        );
    }
}

#[test]
fn rejects_wasm_module_with_host_or_url_or_traversal_paths() {
    for bad in [
        "",
        " ",
        "/abs/path.wasm",
        "../escape.wasm",
        "foo/../bar.wasm",
        "https://evil.example.com/x.wasm",
        "file:///tmp/x.wasm",
        "C:\\windows.wasm",
        "c:/win.wasm",
        "has space.wasm",
        "wasm/./echo.wasm",
    ] {
        let toml = format!(
            r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "{bad}"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
            schema = MANIFEST_SCHEMA_VERSION,
            bad = bad.replace('\\', "\\\\"),
        );
        let err = ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog())
            .unwrap_err();
        assert!(
            matches!(err, ManifestV2Error::InvalidWasmModuleRef { .. }),
            "wasm module {bad:?} should be rejected, got {err:?}"
        );
    }
}

#[test]
fn mcp_runtime_enforces_transport_and_shape() {
    let cap_block = r#"
[[capabilities]]
id = "acme-mcp.search"
description = "search"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/search.input.v1.json"
output_schema_ref = "schemas/acme/search.output.v1.json"
"#;
    let header = format!(
        r#"
schema_version = "{schema}"
id = "acme-mcp"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );

    // accepts: stdio with command, http with absolute https url
    for runtime in [
        "[runtime]\nkind = \"mcp\"\ntransport = \"stdio\"\ncommand = \"server\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"http\"\nurl = \"https://example.com/mcp\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"sse\"\nurl = \"https://example.com/mcp\"\n",
    ] {
        let toml = format!("{header}\n{runtime}\n{cap_block}");
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog())
            .unwrap_or_else(|err| panic!("valid mcp runtime rejected: {err:?}\n{runtime}"));
    }

    // rejects: stdio with url; http without url; http with command; unknown transport; ftp url.
    for runtime in [
        "[runtime]\nkind = \"mcp\"\ntransport = \"stdio\"\nurl = \"https://x.com\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"stdio\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"http\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"http\"\ncommand = \"x\"\nurl = \"https://x.com\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"telnet\"\nurl = \"telnet://x.com\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"http\"\nurl = \"ftp://example.com\"\n",
        "[runtime]\nkind = \"mcp\"\ntransport = \"\"\n",
    ] {
        let toml = format!("{header}\n{runtime}\n{cap_block}");
        let err = ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog())
            .unwrap_err();
        assert!(
            matches!(err, ManifestV2Error::InvalidMcpRuntime { .. }),
            "runtime should be rejected:\n{runtime}\n got {err:?}"
        );
    }
}

#[test]
fn rejects_duplicate_required_host_ports_in_one_capability() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
required_host_ports = ["host.events.audit", "host.events.audit"]
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::DuplicateRequiredHostPort { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_duplicate_implements_in_one_capability() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
implements = ["memory.context_retrieval.v1", "memory.context_retrieval.v1"]
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::DuplicateImplementedProfile { .. }),
        "{err:?}"
    );
}

#[test]
fn capability_rejects_unknown_fields_on_deserialize() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
sneaky = true
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(matches!(err, ManifestV2Error::Parse { .. }), "{err:?}");
}

#[test]
fn rejects_duplicate_capability_ids() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "x"
version = "0.1"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"

[[capabilities]]
id = "acme-tools.echo"
description = "echo (dup)"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::DuplicateCapability { .. }),
        "{err:?}"
    );
}

// ---------------------------------------------------------------------------
// Issue-driven coverage (zmanian review, slice 2a).
// ---------------------------------------------------------------------------

#[test]
fn host_bundled_accepts_non_reserved_id() {
    // Spec: the `ironclaw.` prefix is reserved *for* HostBundled. It is not
    // *required* of HostBundled. A host-bundled extension may legitimately
    // ship under any id; lock that in so the reserved-prefix rule does not
    // accidentally become a "must use" rule downstream.
    let toml = third_party_wasm_manifest("memory-native", "memory-native.echo");
    let manifest =
        ExtensionManifestV2::parse(&toml, ManifestSource::HostBundled, &catalog()).unwrap();
    assert_eq!(manifest.source, ManifestSource::HostBundled);
    assert_eq!(manifest.id, ExtensionId::new("memory-native").unwrap());
}

#[test]
fn parses_multi_capability_manifest_with_distinct_implements() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "Acme"
version = "0.1.0"
description = "two capabilities"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/acme.wasm"

[[capabilities]]
id = "acme-tools.echo"
implements = ["acme.echo.v1"]
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"

[[capabilities]]
id = "acme-tools.reverse"
implements = ["acme.reverse.v1", "acme.string_ops.v1"]
description = "reverse a string"
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acme/reverse.input.v1.json"
output_schema_ref = "schemas/acme/reverse.output.v1.json"
prompt_doc_ref = "prompt/acme/reverse.md"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let manifest =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap();
    assert_eq!(manifest.capabilities.len(), 2);
    assert_eq!(
        manifest.capabilities[0].implements,
        vec![CapabilityProfileId::new("acme.echo.v1").unwrap()]
    );
    assert_eq!(
        manifest.capabilities[1].implements,
        vec![
            CapabilityProfileId::new("acme.reverse.v1").unwrap(),
            CapabilityProfileId::new("acme.string_ops.v1").unwrap(),
        ]
    );
}

#[test]
fn rejects_manifest_exceeding_max_size() {
    use ironclaw_extensions::{MAX_MANIFEST_BYTES, ManifestV2Error};
    // Construct an input strictly larger than MAX_MANIFEST_BYTES *before*
    // reaching the TOML parser. The check must fail closed without parsing.
    let mut huge = String::with_capacity(MAX_MANIFEST_BYTES + 1024);
    huge.push_str("# pad\n");
    while huge.len() <= MAX_MANIFEST_BYTES {
        huge.push_str("# filler line to defeat short-circuit eval\n");
    }
    let err =
        ExtensionManifestV2::parse(&huge, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::ManifestTooLarge { bytes, max } if bytes == huge.len() && max == MAX_MANIFEST_BYTES),
        "{err:?}"
    );
}

#[test]
fn rejects_duplicate_effect_in_capability() {
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "Acme"
version = "0.1.0"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/acme.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
effects = ["read_filesystem", "read_filesystem"]
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(
        matches!(err, ManifestV2Error::DuplicateEffect { .. }),
        "{err:?}"
    );
}

#[test]
fn schema_ref_errors_carry_field_context() {
    // Absolute schema refs are rejected by CapabilityProfileSchemaRef::new.
    // The parser must wrap the underlying error with the offending field name
    // so hand-edited manifests get an actionable error.
    let toml = format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "Acme"
version = "0.1.0"
description = "x"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/acme.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "echo"
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "/abs/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let err =
        ExtensionManifestV2::parse(&toml, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    match err {
        ManifestV2Error::InvalidSchemaRef { field, .. } => {
            assert_eq!(field, "input_schema_ref");
        }
        other => panic!("expected InvalidSchemaRef, got {other:?}"),
    }
}
