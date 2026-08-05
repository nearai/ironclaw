use super::*;

/// The ONE construction seam for host HTTP egress.
///
/// Two arms, chosen at COMPILE time, so a shipped binary does not merely
/// refuse the test seam at runtime — it does not contain it.
///
/// * **Production** (release/dist: no `debug_assertions`, no `test-support`):
///   policy enforcement directly over the reqwest transport. The env-gated
///   host-rewrite wrapper is not compiled into the binary at all, so
///   `IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP` has nowhere to take effect.
/// * **Debug / `test-support`**: the same policy egress wrapped in the
///   rewrite transport, so E2E harnesses keep redirecting vendor calls to
///   local fakes. E2E builds are debug builds, so they get this implicitly.
///
/// This replaces a *runtime* profile-proxy check (`cfg!(debug_assertions)`
/// inside `from_env_value`, which still fails closed and stays as
/// defence-in-depth) with compile-time exclusion. Fail-closed either way: a
/// set-but-invalid map refuses composition.
#[cfg(any(debug_assertions, feature = "test-support"))]
pub(super) type HostHttpEgress = ironclaw_network::PolicyNetworkHttpEgress<
    ironclaw_network::RewriteNetworkTransport<ironclaw_network::ReqwestNetworkTransport>,
>;

/// Production shape: no rewrite wrapper in the type at all.
#[cfg(not(any(debug_assertions, feature = "test-support")))]
pub(super) type HostHttpEgress =
    ironclaw_network::PolicyNetworkHttpEgress<ironclaw_network::ReqwestNetworkTransport>;

#[cfg(any(debug_assertions, feature = "test-support"))]
pub(super) fn default_host_http_egress() -> Result<HostHttpEgress, RebornBuildError> {
    ironclaw_network::default_policy_http_egress().map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })
}

#[cfg(not(any(debug_assertions, feature = "test-support")))]
pub(super) fn default_host_http_egress() -> Result<HostHttpEgress, RebornBuildError> {
    Ok(ironclaw_network::PolicyNetworkHttpEgress::new(
        ironclaw_network::ReqwestNetworkTransport::default(),
    ))
}

pub(super) fn apply_post_edit_check_from_env<F, G>(
    services: HostRuntimeServices<F, G>,
) -> Result<HostRuntimeServices<F, G>, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
{
    match PostEditCheckConfig::from_env() {
        Ok(Some(post_edit_check)) => Ok(services.with_post_edit_check(post_edit_check)),
        Ok(None) => Ok(services),
        Err(error) => Err(RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }),
    }
}

pub(super) fn require_product_auth_runtime_ports<F, G>(
    services: &HostRuntimeServices<F, G>,
) -> Result<ProductAuthProviderRuntimePorts, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
{
    services
        .product_auth_provider_runtime_ports()
        .ok_or_else(|| RebornBuildError::InvalidConfig {
            reason: "product auth runtime ports unavailable; host runtime must be configured with HTTP egress and a secret store".to_string(),
        })
}

pub(super) fn attach_hosted_mcp_runtime<F, G>(
    services: HostRuntimeServices<F, G>,
) -> Result<HostRuntimeServices<F, G>, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
{
    // Soft-disable when host runtime HTTP egress is absent. Builds without
    // egress — in-memory test services, minimal compositions — must still
    // succeed; only hosted MCP capabilities go dark.
    let Some(runtime_ports) = services.product_auth_provider_runtime_ports() else {
        tracing::debug!(
            "skipping hosted MCP runtime: host runtime HTTP egress absent \
             (only affects hosted MCP extensions, e.g. Notion, NEAR AI)"
        );
        return Ok(services);
    };
    let runtime_http_egress = runtime_ports.runtime_http_egress();
    let registry = services.shared_extension_registry();

    Ok(services.with_mcp_runtime(Arc::new(hosted_http_mcp_runtime(
        registry,
        runtime_http_egress,
    ))))
}

pub(super) fn attach_wasm_runtime<F, G>(
    services: HostRuntimeServices<F, G>,
) -> Result<HostRuntimeServices<F, G>, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
{
    services
        .try_with_default_wasm_runtime()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("WASM runtime could not be initialized: {error}"),
        })
}

pub(crate) fn apply_production_runtime_process_binding<F, G>(
    services: HostRuntimeServices<F, G>,
    binding: RebornRuntimeProcessBinding,
) -> HostRuntimeServices<F, G>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
{
    match binding {
        RebornRuntimeProcessBinding::None => services,
        RebornRuntimeProcessBinding::UserSandbox { process_port } => {
            services.with_production_user_sandbox_process_port(process_port)
        }
    }
}
