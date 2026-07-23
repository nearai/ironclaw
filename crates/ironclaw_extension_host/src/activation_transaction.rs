//! Owner-side extension activation transaction.
//!
//! Composition supplies concrete store/runtime operations through a statically
//! dispatched dependency-inversion port. This module owns their ordering:
//! readiness, hosted-MCP discovery outside the operation lock, authority
//! recheck, and cross-publication compensation.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::CredentialAccount;
use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationId,
    ExtensionManifestRecord, ExtensionPackage, is_hosted_http_mcp_package,
};
use ironclaw_host_api::{
    ExtensionId, ResourceScope, RuntimeCredentialAuthRequirement, RuntimeHttpEgress, UserId,
};
use tokio::sync::Mutex;

use crate::hosted_mcp_discovery_authority::HostedMcpDiscoveryAuthority;

/// Runtime lane selected for one internal readiness reconciliation.
#[derive(Clone)]
pub enum ExtensionActivationMode {
    Static,
    HostedMcpDiscovery {
        scope: ResourceScope,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    },
}

impl ExtensionActivationMode {
    pub fn from_dispatch_context(
        scope: ResourceScope,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    ) -> Self {
        match runtime_http_egress {
            Some(runtime_http_egress) => Self::HostedMcpDiscovery {
                scope,
                runtime_http_egress,
            },
            None => Self::Static,
        }
    }
}

/// Credential authority observed at one activation checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionActivationCredentialReadiness {
    Ready(Vec<CredentialAccount>),
    Missing(Vec<RuntimeCredentialAuthRequirement>),
}

/// Canonical installed state captured while the lifecycle operation lock is
/// held. It is a transaction checkpoint, not a transport projection.
struct ExtensionActivationSnapshot {
    package: ExtensionPackage,
    manifest: ExtensionManifestRecord,
}

/// Owner transaction result consumed by the product presentation adapter.
pub enum ExtensionActivationTransactionResult {
    CredentialsMissing(Vec<RuntimeCredentialAuthRequirement>),
    Activated(ExtensionPackage),
}

/// Concrete operations supplied by the composition root.
///
/// The transaction is generic over this port and never stores it behind
/// `dyn`; the boundary exists because the generic extension host cannot
/// depend on the app composition crate. Implementations must be thin
/// delegates to existing stores/services.
#[async_trait]
pub trait ExtensionActivationOperations: Send + Sync {
    type Error: Send;

    async fn load_installation(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<ExtensionInstallation, Self::Error>;

    fn ensure_caller_may_operate(
        &self,
        installation: &ExtensionInstallation,
        caller: &UserId,
    ) -> Result<(), Self::Error>;

    async fn lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionPackage, Self::Error>;

    async fn installed_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionManifestRecord, Self::Error>;

    async fn missing_account_setup(
        &self,
        extension_id: &ExtensionId,
        caller: &UserId,
    ) -> Result<Option<RuntimeCredentialAuthRequirement>, Self::Error>;

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, Self::Error>;

    async fn stage_hosted_mcp_discovery_authority(
        &self,
        scope: &ResourceScope,
        package: &ExtensionPackage,
    );

    async fn discover_hosted_mcp_package(
        &self,
        package: &ExtensionPackage,
        max_tools: u32,
        scope: ResourceScope,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    ) -> Result<ExtensionPackage, Self::Error>;

    fn package_is_published(&self, extension_id: &ExtensionId, package: &ExtensionPackage) -> bool;

    async fn enable_lifecycle_package(&self, extension_id: &ExtensionId)
    -> Result<(), Self::Error>;

    async fn disable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), Self::Error>;

    fn publish_active_package(&self, package: &ExtensionPackage) -> Result<(), Self::Error>;

    fn unpublish_active_package(&self, package: &ExtensionPackage) -> Result<(), Self::Error>;

    async fn publish_runtime_package(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        package: &ExtensionPackage,
    ) -> Result<(), Self::Error>;

    fn map_authority_error(&self, error: ExtensionInstallationError) -> Self::Error;

    fn discovery_recheck_error(&self, error: Option<Self::Error>) -> Self::Error;

    fn compensation_failure(
        &self,
        context: &'static str,
        original: Self::Error,
        compensation: Self::Error,
    ) -> Self::Error;
}

/// Execute one activation through the owner-defined transaction order.
pub async fn run_extension_activation<O>(
    operation_lock: &Mutex<()>,
    operations: &O,
    extension_id: &ExtensionId,
    installation_id: &ExtensionInstallationId,
    caller: &UserId,
    mode: ExtensionActivationMode,
) -> Result<ExtensionActivationTransactionResult, O::Error>
where
    O: ExtensionActivationOperations,
{
    let initial = {
        let _operation_guard = operation_lock.lock().await;
        let snapshot = capture_snapshot(operations, extension_id, installation_id, caller).await?;
        if let Some(missing) = operations
            .missing_account_setup(extension_id, caller)
            .await?
        {
            return Ok(ExtensionActivationTransactionResult::CredentialsMissing(
                vec![missing],
            ));
        }
        match mode {
            ExtensionActivationMode::HostedMcpDiscovery {
                scope,
                runtime_http_egress,
            } if is_hosted_http_mcp_package(&snapshot.package) => {
                (snapshot, scope, runtime_http_egress)
            }
            _ => {
                let readiness = operations.credential_readiness(&snapshot.package).await?;
                if let ExtensionActivationCredentialReadiness::Missing(missing) = readiness {
                    return Ok(ExtensionActivationTransactionResult::CredentialsMissing(
                        missing,
                    ));
                }
                return commit_activation(
                    operations,
                    extension_id,
                    installation_id,
                    snapshot.package,
                )
                .await;
            }
        }
    };

    let (initial, scope, runtime_http_egress) = initial;
    operations
        .stage_hosted_mcp_discovery_authority(&scope, &initial.package)
        .await;
    let credential_accounts = match operations.credential_readiness(&initial.package).await? {
        ExtensionActivationCredentialReadiness::Ready(accounts) => accounts,
        ExtensionActivationCredentialReadiness::Missing(missing) => {
            return Ok(ExtensionActivationTransactionResult::CredentialsMissing(
                missing,
            ));
        }
    };
    let discovery_authority = HostedMcpDiscoveryAuthority::capture(
        initial.package,
        &initial.manifest,
        credential_accounts,
    )
    .map_err(|error| operations.map_authority_error(error))?;
    let active_package = operations
        .discover_hosted_mcp_package(
            discovery_authority.package(),
            discovery_authority.max_tools(),
            scope,
            runtime_http_egress,
        )
        .await?;

    let _operation_guard = operation_lock.lock().await;
    let current = capture_snapshot(operations, extension_id, installation_id, caller)
        .await
        .map_err(|error| operations.discovery_recheck_error(Some(error)))?;
    if let Some(missing) = operations
        .missing_account_setup(extension_id, caller)
        .await?
    {
        return Ok(ExtensionActivationTransactionResult::CredentialsMissing(
            vec![missing],
        ));
    }
    let current_accounts = match operations.credential_readiness(&current.package).await? {
        ExtensionActivationCredentialReadiness::Ready(accounts) => accounts,
        ExtensionActivationCredentialReadiness::Missing(missing) => {
            return Ok(ExtensionActivationTransactionResult::CredentialsMissing(
                missing,
            ));
        }
    };
    let current_authority =
        HostedMcpDiscoveryAuthority::capture(current.package, &current.manifest, current_accounts)
            .map_err(|error| {
                let error = operations.map_authority_error(error);
                operations.discovery_recheck_error(Some(error))
            })?;
    if !discovery_authority.still_authorizes(&current_authority) {
        return Err(operations.discovery_recheck_error(None));
    }
    commit_activation(operations, extension_id, installation_id, active_package).await
}

async fn capture_snapshot<O>(
    operations: &O,
    extension_id: &ExtensionId,
    installation_id: &ExtensionInstallationId,
    caller: &UserId,
) -> Result<ExtensionActivationSnapshot, O::Error>
where
    O: ExtensionActivationOperations,
{
    let installation = operations
        .load_installation(extension_id, installation_id)
        .await?;
    operations.ensure_caller_may_operate(&installation, caller)?;
    let package = operations.lifecycle_package(extension_id).await?;
    let manifest = operations.installed_manifest(extension_id).await?;
    Ok(ExtensionActivationSnapshot { package, manifest })
}

async fn commit_activation<O>(
    operations: &O,
    extension_id: &ExtensionId,
    installation_id: &ExtensionInstallationId,
    package: ExtensionPackage,
) -> Result<ExtensionActivationTransactionResult, O::Error>
where
    O: ExtensionActivationOperations,
{
    if operations.package_is_published(extension_id, &package) {
        return Ok(ExtensionActivationTransactionResult::Activated(package));
    }
    operations.enable_lifecycle_package(extension_id).await?;
    if let Err(error) = operations.publish_active_package(&package) {
        if let Err(rollback_error) = operations.disable_lifecycle_package(extension_id).await {
            return Err(operations.compensation_failure(
                "extension activation failed to publish active package and lifecycle disable rollback failed",
                error,
                rollback_error,
            ));
        }
        return Err(error);
    }
    if let Err(error) = operations
        .publish_runtime_package(extension_id, installation_id, &package)
        .await
    {
        if let Err(rollback_error) = operations.unpublish_active_package(&package) {
            return Err(operations.compensation_failure(
                "extension activation failed to publish the dispatch snapshot and registry unpublish failed",
                error,
                rollback_error,
            ));
        }
        if let Err(rollback_error) = operations.disable_lifecycle_package(extension_id).await {
            return Err(operations.compensation_failure(
                "extension runtime publication failed and lifecycle rollback failed",
                error,
                rollback_error,
            ));
        }
        return Err(error);
    }
    Ok(ExtensionActivationTransactionResult::Activated(package))
}
