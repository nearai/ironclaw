use super::*;

/// Nearai-shaped `HostBundled` fixture manifest: `[mcp]` connection plus a
/// static `[[tools]]` template capability, matching
/// `crates/extensions/packages/nearai-mcp/manifest.toml`'s
/// shape (source, root binding, and discovered-schema path convention),
/// without depending on the real nearai asset files.
fn hosted_mcp_first_party_manifest_toml(id: &str) -> String {
    format!(
        r#"
schema_version = "reborn.extension_manifest.v3"
id = "{id}"
name = "Hosted MCP fixture"
version = "0.1.0"
description = "hosted MCP discovery fixture mirroring the nearai bundled provider"
trust = "first_party_requested"

[mcp]
server = "https://mcp.example.test/mcp"
namespace = "{id}"
max_tools = 8
default_permission = "ask"
effects = ["network"]

[[tools]]
id = "{id}.web_search"
description = "Static template tool"
default_permission = "ask"
input_schema_ref = "schemas/{id}/web_search.input.v1.json"
"#,
        id = id,
    )
}

/// Regression test for the production incident: "The run failed while
/// preparing the runtime host" /
/// `missing input_schema_ref at /system/extensions/nearai/schemas/nearai/dynamic/web_search.input.v1.json`.
///
/// Drives the real production caller chain for a `HostBundled` hosted-MCP
/// provider shaped exactly like `nearai` (source `HostBundled`,
/// `root_binding: Materialized`, `descriptor_schema_mode: InlineDynamic`
/// after discovery):
///
/// 1. `discover_hosted_mcp_package` (the real discovery path activation
///    uses) against a stubbed MCP server returning one tool with a
///    non-trivial input schema.
/// 2. `effective_resolved_for_package` (the real activation-publish
///    helper) — pins defect 1: the persisted `ResolvedExtensionManifest`
///    must carry the discovered schema in `mcp.dynamic_input_schemas`.
/// 3. `rebuild_package_from_resolved` from ONLY that durable record (no
///    live discovery) — pins defect 2: rebuild must choose the
///    inline-dynamic constructor, not the `ManifestRefs` one.
/// 4. `publish_hot_capability_catalog` (the real capability-catalog path)
///    against a filesystem that does NOT contain the schema file —
///    reproduces the exact production absence and must still succeed,
///    with `parameters_schema` equal to the originally discovered schema.
#[tokio::test]
async fn hosted_mcp_discovered_schema_survives_persist_and_rebuild_without_filesystem_schema() {
    let id = "hosted-mcp-fixture";
    let toml = hosted_mcp_first_party_manifest_toml(id);
    let root = VirtualPath::new(format!("/system/extensions/{id}")).expect("test root");
    let record = ExtensionManifestRecord::from_toml_with_root_binding(
        toml,
        ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
        None,
        &crate::product_extension_host_api_contract_registry().expect("test contracts"),
        ironclaw_extension_registry::PackageRootBinding::Materialized(root.clone()),
    )
    .expect("hosted MCP fixture manifest resolves");
    let base_resolved = record.resolved().clone();
    assert!(
        base_resolved
            .mcp
            .as_ref()
            .is_some_and(|mcp| mcp.dynamic_input_schemas.is_empty()),
        "the pre-discovery durable record must not yet carry a discovered schema"
    );

    // Build the pre-discovery package exactly as the production loader
    // would (`to_internal` + `try_from`, no TOML reparse), in
    // `ManifestRefs` mode (the ordinary Materialized shape before
    // discovery ever runs).
    let manifest_v2 = base_resolved
        .to_internal(ManifestSource::HostBundled)
        .expect("resolved contract rebuilds to v2");
    let manifest = ironclaw_extension_registry::ExtensionManifest::try_from(manifest_v2)
        .expect("v2 manifest rebuilds to v1");
    let initial_package = ExtensionPackage::from_manifest(manifest, root.clone())
        .expect("pre-discovery package constructs");

    let discovered_schema = serde_json::json!({
        "type": "object",
        "properties": {"query": {"type": "string"}},
        "required": ["query"]
    });
    let scope = ironclaw_host_api::resource::ResourceScope::local_default(
        ironclaw_host_api::ids::UserId::new("hosted-mcp-fixture-user").expect("test user"),
        ironclaw_host_api::ids::InvocationId::new(),
    )
    .expect("test scope");
    let discovered = crate::discover_hosted_mcp_package(
        &initial_package,
        8,
        scope,
        Arc::new(OneToolEgress {
            tool_name: "web_search",
            input_schema: discovered_schema.clone(),
        }),
    )
    .await
    .expect("stubbed discovery succeeds");
    assert_eq!(
        discovered.descriptor_schema_mode,
        ironclaw_extension_registry::CapabilityDescriptorSchemaMode::InlineDynamic,
        "a HostBundled hosted-MCP package built from discovery is InlineDynamic, \
         exactly like nearai's `package_with_discovered_hosted_mcp_tools`"
    );

    // Defect 1: the persisted record must capture the discovered schema
    // for THIS package shape, not only for `Virtual` packages.
    let effective = effective_resolved_for_package(&base_resolved, &discovered);
    let capability_id = format!("{id}.web_search");
    assert_eq!(
        effective
            .mcp
            .as_ref()
            .expect("hosted MCP declaration persists")
            .dynamic_input_schemas
            .get(capability_id.as_str()),
        Some(&discovered_schema),
        "effective_resolved_for_package must persist the discovered schema for a \
         Materialized + InlineDynamic package, not only for Virtual packages"
    );

    // Defect 2: rebuilding from ONLY the durable record (no live
    // discovery) must reconstruct an InlineDynamic package whose
    // descriptor already carries the discovered schema.
    let rebuild_manifest_v2 = effective
        .to_internal(ManifestSource::HostBundled)
        .expect("persisted resolved contract rebuilds to v2");
    let rebuild_manifest =
        ironclaw_extension_registry::ExtensionManifest::try_from(rebuild_manifest_v2)
            .expect("v2 manifest rebuilds to v1");
    let rebuilt = rebuild_package_from_resolved(rebuild_manifest, &effective, id)
        .expect("rebuild from the durable record alone succeeds");
    assert_eq!(
        rebuilt.descriptor_schema_mode,
        ironclaw_extension_registry::CapabilityDescriptorSchemaMode::InlineDynamic,
        "rebuilding a Materialized package with a persisted discovered-schema map must \
         choose the inline-dynamic constructor, not `from_manifest`'s ManifestRefs"
    );

    // Records written before exact MCP wire names were persisted omit
    // `tool_names`. They must continue to deserialize and rebuild with
    // the historical capability-suffix fallback.
    let mut legacy_json = serde_json::to_value(&effective).expect("resolved record serializes");
    legacy_json
        .get_mut("mcp")
        .and_then(serde_json::Value::as_object_mut)
        .expect("resolved record has MCP declaration")
        .remove("tool_names");
    let legacy_resolved: ResolvedExtensionManifest =
        serde_json::from_value(legacy_json).expect("legacy resolved record deserializes");
    let legacy_manifest_v2 = legacy_resolved
        .to_internal(ManifestSource::HostBundled)
        .expect("legacy resolved contract rebuilds to v2");
    let legacy_manifest =
        ironclaw_extension_registry::ExtensionManifest::try_from(legacy_manifest_v2)
            .expect("legacy v2 manifest rebuilds to v1");
    let legacy_package = rebuild_package_from_resolved(legacy_manifest, &legacy_resolved, id)
        .expect("legacy record without tool-name bindings rebuilds");
    assert!(
        legacy_package.hosted_mcp_tool_names().is_empty(),
        "missing persisted bindings select the historical suffix-derived wire name"
    );

    // Reproduce the exact production absence: publish through the real
    // capability-catalog path against a filesystem with no schema file at
    // all (not even the directory), and confirm success with the
    // originally discovered schema.
    let fs = ironclaw_filesystem::InMemoryBackend::new();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(rebuilt)
        .expect("rebuilt package inserts into the registry");
    let catalog = ironclaw_host_runtime::publish_hot_capability_catalog(&fs, &registry)
        .await
        .expect(
            "publishing the rebuilt package must succeed even though \
             /system/extensions/hosted-mcp-fixture/schemas/hosted-mcp-fixture/dynamic/\
             web_search.input.v1.json was never written to the filesystem",
        );
    let record = catalog
        .get(&CapabilityId::new(capability_id.as_str()).expect("capability id"))
        .expect("discovered capability publishes");
    assert_eq!(
        record.descriptor.parameters_schema, discovered_schema,
        "the published descriptor must carry the originally discovered schema"
    );
}

#[test]
fn hosted_mcp_rebuild_rejects_partial_and_corrupt_tool_name_bindings() {
    let id = "hosted-mcp-binding-fixture";
    let root = VirtualPath::new(format!("/system/extensions/{id}")).expect("test root");
    let record = ExtensionManifestRecord::from_toml_with_root_binding(
        hosted_mcp_first_party_manifest_toml(id),
        ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
        None,
        &crate::product_extension_host_api_contract_registry().expect("test contracts"),
        ironclaw_extension_registry::PackageRootBinding::Materialized(root.clone()),
    )
    .expect("hosted MCP fixture manifest resolves");
    let manifest_v2 = record
        .resolved()
        .to_internal(ManifestSource::HostBundled)
        .expect("resolved contract rebuilds to v2");
    let manifest = ironclaw_extension_registry::ExtensionManifest::try_from(manifest_v2)
        .expect("v2 manifest rebuilds to v1");
    let initial =
        ExtensionPackage::from_manifest(manifest, root).expect("pre-discovery package constructs");
    let discovered = ironclaw_extension_registry::package_with_discovered_hosted_mcp_tools(
        &initial,
        &[
            ironclaw_extension_contracts::hosted_mcp::HostedMcpDiscoveredTool {
                name: "authHealth".to_string(),
                description: "Check auth health".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                annotations: Default::default(),
            },
            ironclaw_extension_contracts::hosted_mcp::HostedMcpDiscoveredTool {
                name: "ping".to_string(),
                description: "Ping".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                annotations: Default::default(),
            },
        ],
    )
    .expect("two-tool discovered package builds");
    let effective = effective_resolved_for_package(record.resolved(), &discovered);

    let mut partial = effective.clone();
    partial
        .mcp
        .as_mut()
        .expect("MCP declaration")
        .tool_names
        .remove(&format!("{id}.ping"));
    let mut alias_mismatch = effective.clone();
    alias_mismatch
        .mcp
        .as_mut()
        .expect("MCP declaration")
        .tool_names
        .insert(format!("{id}.authhealth"), "differentName".to_string());
    let mut invalid_wire_name = effective;
    invalid_wire_name
        .mcp
        .as_mut()
        .expect("MCP declaration")
        .tool_names
        .insert(format!("{id}.authhealth"), "invalid name".to_string());

    for (case, corrupt) in [
        ("partial", partial),
        ("alias mismatch", alias_mismatch),
        ("invalid wire grammar", invalid_wire_name),
    ] {
        let manifest_v2 = corrupt
            .to_internal(ManifestSource::HostBundled)
            .unwrap_or_else(|error| panic!("{case} record remains structurally readable: {error}"));
        let manifest = ironclaw_extension_registry::ExtensionManifest::try_from(manifest_v2)
            .unwrap_or_else(|error| {
                panic!("{case} manifest remains structurally readable: {error}")
            });
        let error = rebuild_package_from_resolved(manifest, &corrupt, id)
            .expect_err("corrupt non-empty MCP tool-name bindings must fail closed");
        assert!(
            error.contains("tool-name bindings do not match discovered capabilities"),
            "{case} must fail at the binding integrity check, got: {error}"
        );
    }
}
