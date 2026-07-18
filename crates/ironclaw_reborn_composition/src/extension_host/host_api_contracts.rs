// One definition, shared with serve-time ingress projection: the registry
// builder moved to `ironclaw_channel_host` (channel host crates project
// ingress descriptors from bundled manifests with the SAME parsing context as
// bundled-extension installation, so the two paths cannot drift).
pub(crate) use ironclaw_channel_host::host_ingress::product_extension_host_api_contract_registry;
