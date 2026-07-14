//! Registered-extension id-minting test support (T1 seam). Every read of a
//! `UserRegistered` descriptor is gated by `registered_package_has_minted_id`
//! recomputing `HostedMcpExtensionId::mint(tenant, owner, url, "")` and
//! matching it against the stored id — that minting logic is `pub(crate)` to
//! `extension_host::registered_extension_store`, so a cross-crate
//! integration-test fixture that seeds a registered MCP descriptor directly
//! on disk needs this wrapper to produce an id that survives the mint-gate
//! on read, instead of drifting a bare literal the gate silently filters out.
//! Mirrors the in-crate `extension_host::registered_test_support::minted_extension_id`
//! helper for callers outside this crate.

/// Mint the same id `registered_package_has_minted_id` recomputes on read.
/// `account_label` is fixed at `""`, matching every production call site.
#[cfg(feature = "test-support")]
pub fn mint_registered_mcp_extension_id_for_test(
    tenant_id: &ironclaw_host_api::TenantId,
    owner: &ironclaw_host_api::UserId,
    url: &str,
) -> ironclaw_host_api::ExtensionId {
    crate::extension_host::registered_extension_store::HostedMcpExtensionId::mint(
        tenant_id, owner, url, "",
    )
    .expect("mint hosted MCP id") // safety: test-only fixture minting.
    .into_extension_id()
}
