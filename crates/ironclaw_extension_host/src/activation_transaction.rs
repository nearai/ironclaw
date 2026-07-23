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
    ExtensionManifestRecord, ExtensionPackage, ExtensionRuntime, is_hosted_http_mcp_package,
};
use ironclaw_host_api::{
    ExtensionId, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    RuntimeCredentialAuthRequirement, RuntimeHttpEgress, UserId,
};
use tokio::sync::Mutex;

use crate::hosted_mcp_discovery_authority::McpDiscoveryFence;

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
    Activated(Box<ExtensionPackage>),
}

/// Outcome of the hosted-MCP discovery step within an activation.
pub enum HostedMcpDiscoveryOutcome {
    /// Discovery produced a bounded catalog; the package carries the discovered
    /// tools and is the candidate for publication after the authority recheck.
    /// Boxed to keep the enum small (the package dwarfs the rejection payload).
    Discovered(Box<ExtensionPackage>),
    /// The provider rejected the staged credentials during discovery (a
    /// mid-`tools/list` 401/403). No catalog can be published, so the
    /// transaction routes the caller back through credential setup / OAuth
    /// exactly like a pre-discovery missing credential — never a retry-forever
    /// transient.
    CredentialsRejected(Vec<RuntimeCredentialAuthRequirement>),
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
    type HostedMcpDiscoveryAuthority: Send;

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
        network_policy: NetworkPolicy,
    ) -> Self::HostedMcpDiscoveryAuthority;

    async fn discover_hosted_mcp_package(
        &self,
        package: &ExtensionPackage,
        max_tools: u32,
        scope: ResourceScope,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    ) -> Result<HostedMcpDiscoveryOutcome, Self::Error>;

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
    let network_policy = hosted_mcp_discovery_network_policy(&initial.package)
        .map_err(|error| operations.map_authority_error(error))?;
    let _discovery_authority_guard = operations
        .stage_hosted_mcp_discovery_authority(&scope, &initial.package, network_policy)
        .await;
    let credential_accounts = match operations.credential_readiness(&initial.package).await? {
        ExtensionActivationCredentialReadiness::Ready(accounts) => accounts,
        ExtensionActivationCredentialReadiness::Missing(missing) => {
            return Ok(ExtensionActivationTransactionResult::CredentialsMissing(
                missing,
            ));
        }
    };
    let discovery_authority =
        McpDiscoveryFence::capture(initial.package, &initial.manifest, credential_accounts)
            .map_err(|error| operations.map_authority_error(error))?;
    let active_package = match operations
        .discover_hosted_mcp_package(
            discovery_authority.package(),
            discovery_authority.max_tools(),
            scope,
            runtime_http_egress,
        )
        .await?
    {
        HostedMcpDiscoveryOutcome::Discovered(package) => *package,
        HostedMcpDiscoveryOutcome::CredentialsRejected(missing) => {
            // The provider rejected the staged credentials mid-discovery.
            // Nothing was published, so route the caller back through
            // credential setup / OAuth — the same outcome as a pre-discovery
            // missing credential — instead of surfacing a retry-forever
            // transient that re-hits the same rejection.
            return Ok(ExtensionActivationTransactionResult::CredentialsMissing(
                missing,
            ));
        }
    };

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
        McpDiscoveryFence::capture(current.package, &current.manifest, current_accounts).map_err(
            |error| {
                let error = operations.map_authority_error(error);
                operations.discovery_recheck_error(Some(error))
            },
        )?;
    if !discovery_authority.still_authorizes(&current_authority) {
        return Err(operations.discovery_recheck_error(None));
    }
    commit_activation(operations, extension_id, installation_id, active_package).await
}

fn hosted_mcp_discovery_network_policy(
    package: &ExtensionPackage,
) -> Result<NetworkPolicy, ExtensionInstallationError> {
    const MCP_NETWORK_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;

    let ExtensionRuntime::Mcp {
        transport,
        command: None,
        args,
        url: Some(url),
    } = &package.manifest.runtime
    else {
        return Err(ExtensionInstallationError::InvalidInstallation {
            reason: format!(
                "hosted MCP extension {} has no hosted HTTP runtime",
                package.id.as_str()
            ),
        });
    };
    if transport != "http" || !args.is_empty() {
        return Err(ExtensionInstallationError::InvalidInstallation {
            reason: format!(
                "hosted MCP extension {} has an unsupported discovery transport",
                package.id.as_str()
            ),
        });
    }
    let parsed =
        url::Url::parse(url).map_err(|_| ExtensionInstallationError::InvalidInstallation {
            reason: format!(
                "hosted MCP extension {} has an invalid discovery endpoint",
                package.id.as_str()
            ),
        })?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ExtensionInstallationError::InvalidInstallation {
            reason: format!(
                "hosted MCP extension {} has an invalid discovery endpoint",
                package.id.as_str()
            ),
        });
    }
    let host =
        parsed
            .host_str()
            .ok_or_else(|| ExtensionInstallationError::InvalidInstallation {
                reason: format!(
                    "hosted MCP extension {} has no discovery endpoint host",
                    package.id.as_str()
                ),
            })?;
    Ok(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: host.to_ascii_lowercase(),
            port: parsed.port(),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(MCP_NETWORK_EGRESS_LIMIT),
    })
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
        return Ok(ExtensionActivationTransactionResult::Activated(Box::new(
            package,
        )));
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
    Ok(ExtensionActivationTransactionResult::Activated(Box::new(
        package,
    )))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_auth::{
        AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId, CredentialAccountLabel,
        CredentialAccountStatus, CredentialOwnership, ProviderScope,
    };
    use ironclaw_extensions::{
        ExtensionInstallation, ExtensionManifest, ExtensionManifestRecord, ExtensionManifestRef,
        ExtensionPackage, HostApiContractRegistry, InstallationOwner, ManifestSource,
    };
    use ironclaw_host_api::{
        HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostPortCatalog, HostPortCatalogEntry, HostPortId,
        InvocationId, NetworkPolicy, RuntimeCredentialAccountSetup, RuntimeHttpEgressError,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle, VendorId, VirtualPath,
    };
    use tokio::sync::Notify;

    use super::*;

    #[tokio::test]
    async fn hosted_discovery_authority_is_revoked_after_success() {
        let operations = ActivationOperationsFixture::success();
        let drops = Arc::clone(&operations.guard_drops);
        let policies = Arc::clone(&operations.staged_policies);

        let result = run_extension_activation(
            &Mutex::new(()),
            &operations,
            &fixture_extension_id(),
            &fixture_installation_id(),
            &fixture_user_id(),
            fixture_discovery_mode(),
        )
        .await
        .expect("hosted MCP discovery should succeed");

        assert!(matches!(
            result,
            ExtensionActivationTransactionResult::Activated(_)
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            policies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)[0]
                .max_egress_bytes,
            Some(2 * 1024 * 1024),
            "hosted discovery must use the normal bounded MCP egress ceiling"
        );
    }

    #[tokio::test]
    async fn hosted_discovery_authority_is_revoked_after_error() {
        let operations = ActivationOperationsFixture::failure();
        let drops = Arc::clone(&operations.guard_drops);

        let error = match run_extension_activation(
            &Mutex::new(()),
            &operations,
            &fixture_extension_id(),
            &fixture_installation_id(),
            &fixture_user_id(),
            fixture_discovery_mode(),
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("scripted discovery failure should propagate"),
        };

        assert_eq!(error, "scripted discovery failure");
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hosted_discovery_credentials_rejected_routes_to_credentials_missing() {
        // A provider that rejects the staged credentials mid-discovery must not
        // surface as an error (which would leave the extension retrying the
        // same rejection forever). It routes to the credentials-missing outcome
        // so the caller is sent back through OAuth, publishes nothing, and the
        // staged discovery authority is still revoked.
        let requirement = RuntimeCredentialAuthRequirement {
            provider: VendorId::new("hosted-provider").expect("vendor id"),
            setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            requester_extension: fixture_extension_id(),
            provider_scopes: Vec::new(),
        };
        let operations =
            ActivationOperationsFixture::credentials_rejected(vec![requirement.clone()]);
        let drops = Arc::clone(&operations.guard_drops);

        let result = run_extension_activation(
            &Mutex::new(()),
            &operations,
            &fixture_extension_id(),
            &fixture_installation_id(),
            &fixture_user_id(),
            fixture_discovery_mode(),
        )
        .await
        .expect("a provider credential rejection must not fail activation as a retry dead-end");

        match result {
            ExtensionActivationTransactionResult::CredentialsMissing(missing) => {
                assert_eq!(
                    missing,
                    vec![requirement],
                    "the re-auth requirements must be forwarded, not discarded"
                );
            }
            ExtensionActivationTransactionResult::Activated(_) => {
                panic!("a provider credential rejection must never yield a false activation")
            }
        }
        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "the staged discovery authority must still be revoked on re-auth routing"
        );
    }

    #[tokio::test]
    async fn hosted_discovery_authority_is_revoked_when_caller_cancels() {
        let entered_discovery = Arc::new(Notify::new());
        let operations = Arc::new(ActivationOperationsFixture::pending(Arc::clone(
            &entered_discovery,
        )));
        let drops = Arc::clone(&operations.guard_drops);
        let task_operations = Arc::clone(&operations);

        let task = tokio::spawn(async move {
            run_extension_activation(
                &Mutex::new(()),
                task_operations.as_ref(),
                &fixture_extension_id(),
                &fixture_installation_id(),
                &fixture_user_id(),
                fixture_discovery_mode(),
            )
            .await
        });
        entered_discovery.notified().await;
        task.abort();
        match task.await {
            Err(error) if error.is_cancelled() => {}
            Err(_) => panic!("task should be canceled, not fail while joining"),
            Ok(_) => panic!("canceled task unexpectedly completed"),
        }

        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "dropping the caller future must revoke staged discovery authority"
        );
    }

    #[tokio::test]
    async fn hosted_discovery_fence_tolerates_benign_timestamp_bump() {
        // A benign write (e.g. a last-used bump) can touch the credential row
        // between the pre-discovery capture and the post-discovery recheck,
        // changing only `updated_at`. That must not trip the authority fence
        // and force a spurious `discovery_recheck` retry — the discovered
        // catalog is still authorized, so activation must commit.
        let base = fence_credential_account(
            CredentialAccountId::new(),
            CredentialAccountStatus::Configured,
            "hosted-access-1",
            &["read"],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("timestamp"),
        );
        let mut bumped = base.clone();
        bumped.updated_at = Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 5, 0)
            .single()
            .expect("timestamp");

        let operations =
            ActivationOperationsFixture::success().with_credential_readiness_sequence(vec![
                ExtensionActivationCredentialReadiness::Ready(vec![base]),
                ExtensionActivationCredentialReadiness::Ready(vec![bumped]),
            ]);

        let result = run_extension_activation(
            &Mutex::new(()),
            &operations,
            &fixture_extension_id(),
            &fixture_installation_id(),
            &fixture_user_id(),
            fixture_discovery_mode(),
        )
        .await
        .expect("a benign timestamp bump must not fail the discovery authority fence");

        assert!(
            matches!(result, ExtensionActivationTransactionResult::Activated(_)),
            "the discovered catalog is still authorized and must activate"
        );
    }

    #[tokio::test]
    async fn hosted_discovery_fence_rejects_real_authority_change() {
        // The fence stays fail-closed: a scope, secret, or status change to the
        // authorizing credential between capture and recheck must still reject
        // the discovered generation with a `discovery_recheck` error.
        let base_id = CredentialAccountId::new();
        let captured = || {
            fence_credential_account(
                base_id,
                CredentialAccountStatus::Configured,
                "hosted-access-1",
                &["read"],
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                    .single()
                    .expect("timestamp"),
            )
        };

        let mut scope_changed = captured();
        scope_changed.scopes = vec![
            ProviderScope::new("read").expect("scope"),
            ProviderScope::new("write").expect("scope"),
        ];
        let mut secret_changed = captured();
        secret_changed.access_secret = Some(SecretHandle::new("hosted-access-2").expect("secret"));
        let mut status_changed = captured();
        status_changed.status = CredentialAccountStatus::Revoked;

        for rechecked in [scope_changed, secret_changed, status_changed] {
            let operations = ActivationOperationsFixture::success()
                .with_credential_readiness_sequence(vec![
                    ExtensionActivationCredentialReadiness::Ready(vec![captured()]),
                    ExtensionActivationCredentialReadiness::Ready(vec![rechecked]),
                ]);

            match run_extension_activation(
                &Mutex::new(()),
                &operations,
                &fixture_extension_id(),
                &fixture_installation_id(),
                &fixture_user_id(),
                fixture_discovery_mode(),
            )
            .await
            {
                Err(error) => assert_eq!(error, "discovery authority changed"),
                Ok(_) => panic!("a real credential authority change must fail the fence"),
            }
        }
    }

    fn fence_credential_account(
        id: CredentialAccountId,
        status: CredentialAccountStatus,
        access_secret: &str,
        scopes: &[&str],
        updated_at: chrono::DateTime<Utc>,
    ) -> CredentialAccount {
        CredentialAccount {
            id,
            scope: AuthProductScope::new(
                ResourceScope::local_default(fixture_user_id(), InvocationId::new())
                    .expect("resource scope"),
                AuthSurface::Web,
            ),
            provider: AuthProviderId::new("hosted-provider").expect("provider id"),
            label: CredentialAccountLabel::new("primary").expect("label"),
            status,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new(access_secret).expect("secret handle")),
            refresh_secret: None,
            scopes: scopes
                .iter()
                .map(|scope| ProviderScope::new(*scope).expect("provider scope"))
                .collect(),
            provider_identity: None,
            created_at: Utc
                .with_ymd_and_hms(2025, 12, 1, 0, 0, 0)
                .single()
                .expect("timestamp"),
            updated_at,
        }
    }

    struct DropProbeGuard {
        drops: Arc<AtomicUsize>,
    }

    impl Drop for DropProbeGuard {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    enum DiscoveryScript {
        Success,
        Failure,
        CredentialsRejected(Vec<RuntimeCredentialAuthRequirement>),
        Pending(Arc<Notify>),
    }

    struct ActivationOperationsFixture {
        installation: ExtensionInstallation,
        package: ExtensionPackage,
        manifest: ExtensionManifestRecord,
        discovery: DiscoveryScript,
        guard_drops: Arc<AtomicUsize>,
        staged_policies: Arc<StdMutex<Vec<NetworkPolicy>>>,
        // Per-call credential readiness. Each `credential_readiness` call pops
        // the next scripted result; an empty queue defaults to the always-ready
        // empty account set the non-fence tests rely on. This lets a fence test
        // return distinct account snapshots on the pre-discovery capture and the
        // post-discovery recheck.
        credential_readiness_sequence:
            Arc<StdMutex<VecDeque<ExtensionActivationCredentialReadiness>>>,
    }

    impl ActivationOperationsFixture {
        fn success() -> Self {
            Self::new(DiscoveryScript::Success)
        }

        fn failure() -> Self {
            Self::new(DiscoveryScript::Failure)
        }

        fn credentials_rejected(requirements: Vec<RuntimeCredentialAuthRequirement>) -> Self {
            Self::new(DiscoveryScript::CredentialsRejected(requirements))
        }

        fn pending(entered: Arc<Notify>) -> Self {
            Self::new(DiscoveryScript::Pending(entered))
        }

        fn new(discovery: DiscoveryScript) -> Self {
            let (package, manifest) = hosted_activation_package();
            Self {
                installation: installation("hosted", &["fixture-user"]),
                package,
                manifest,
                discovery,
                guard_drops: Arc::new(AtomicUsize::new(0)),
                staged_policies: Arc::new(StdMutex::new(Vec::new())),
                credential_readiness_sequence: Arc::new(StdMutex::new(VecDeque::new())),
            }
        }

        /// Script the credential readiness returned by successive
        /// `credential_readiness` calls (pre-discovery capture, then
        /// post-discovery recheck).
        fn with_credential_readiness_sequence(
            self,
            sequence: Vec<ExtensionActivationCredentialReadiness>,
        ) -> Self {
            *self
                .credential_readiness_sequence
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = sequence.into();
            self
        }
    }

    #[async_trait]
    impl ExtensionActivationOperations for ActivationOperationsFixture {
        type Error = String;
        type HostedMcpDiscoveryAuthority = DropProbeGuard;

        async fn load_installation(
            &self,
            _extension_id: &ExtensionId,
            _installation_id: &ExtensionInstallationId,
        ) -> Result<ExtensionInstallation, Self::Error> {
            Ok(self.installation.clone())
        }

        fn ensure_caller_may_operate(
            &self,
            _installation: &ExtensionInstallation,
            _caller: &UserId,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn lifecycle_package(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<ExtensionPackage, Self::Error> {
            Ok(self.package.clone())
        }

        async fn installed_manifest(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<ExtensionManifestRecord, Self::Error> {
            Ok(self.manifest.clone())
        }

        async fn missing_account_setup(
            &self,
            _extension_id: &ExtensionId,
            _caller: &UserId,
        ) -> Result<Option<RuntimeCredentialAuthRequirement>, Self::Error> {
            Ok(None)
        }

        async fn credential_readiness(
            &self,
            _package: &ExtensionPackage,
        ) -> Result<ExtensionActivationCredentialReadiness, Self::Error> {
            Ok(self
                .credential_readiness_sequence
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| ExtensionActivationCredentialReadiness::Ready(Vec::new())))
        }

        async fn stage_hosted_mcp_discovery_authority(
            &self,
            _scope: &ResourceScope,
            _package: &ExtensionPackage,
            network_policy: NetworkPolicy,
        ) -> Self::HostedMcpDiscoveryAuthority {
            self.staged_policies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(network_policy);
            DropProbeGuard {
                drops: Arc::clone(&self.guard_drops),
            }
        }

        async fn discover_hosted_mcp_package(
            &self,
            package: &ExtensionPackage,
            _max_tools: u32,
            _scope: ResourceScope,
            _runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
        ) -> Result<HostedMcpDiscoveryOutcome, Self::Error> {
            match &self.discovery {
                DiscoveryScript::Success => Ok(HostedMcpDiscoveryOutcome::Discovered(Box::new(
                    package.clone(),
                ))),
                DiscoveryScript::Failure => Err("scripted discovery failure".to_string()),
                DiscoveryScript::CredentialsRejected(requirements) => Ok(
                    HostedMcpDiscoveryOutcome::CredentialsRejected(requirements.clone()),
                ),
                DiscoveryScript::Pending(entered) => {
                    entered.notify_one();
                    std::future::pending().await
                }
            }
        }

        fn package_is_published(
            &self,
            _extension_id: &ExtensionId,
            _package: &ExtensionPackage,
        ) -> bool {
            true
        }

        async fn enable_lifecycle_package(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn disable_lifecycle_package(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn publish_active_package(&self, _package: &ExtensionPackage) -> Result<(), Self::Error> {
            Ok(())
        }

        fn unpublish_active_package(&self, _package: &ExtensionPackage) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn publish_runtime_package(
            &self,
            _extension_id: &ExtensionId,
            _installation_id: &ExtensionInstallationId,
            _package: &ExtensionPackage,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn map_authority_error(&self, error: ExtensionInstallationError) -> Self::Error {
            error.to_string()
        }

        fn discovery_recheck_error(&self, error: Option<Self::Error>) -> Self::Error {
            error.unwrap_or_else(|| "discovery authority changed".to_string())
        }

        fn compensation_failure(
            &self,
            context: &'static str,
            original: Self::Error,
            compensation: Self::Error,
        ) -> Self::Error {
            format!("{context}: {original}; {compensation}")
        }
    }

    struct UnreachableRuntimeHttpEgress;

    #[async_trait]
    impl RuntimeHttpEgress for UnreachableRuntimeHttpEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            panic!("fixture discovery never delegates to runtime HTTP")
        }
    }

    fn fixture_discovery_mode() -> ExtensionActivationMode {
        ExtensionActivationMode::HostedMcpDiscovery {
            scope: ResourceScope::local_default(fixture_user_id(), InvocationId::new())
                .expect("resource scope"),
            runtime_http_egress: Arc::new(UnreachableRuntimeHttpEgress),
        }
    }

    fn fixture_extension_id() -> ExtensionId {
        ExtensionId::new("hosted").expect("extension id")
    }

    fn fixture_installation_id() -> ExtensionInstallationId {
        ExtensionInstallationId::new("hosted").expect("installation id")
    }

    fn fixture_user_id() -> UserId {
        UserId::new("fixture-user").expect("user id")
    }

    fn hosted_activation_package() -> (ExtensionPackage, ExtensionManifestRecord) {
        let raw_manifest = r#"
schema_version = "reborn.extension_manifest.v3"
id = "hosted"
name = "Hosted"
version = "1.0.0"
description = "hosted MCP authority fixture"
trust = "third_party"

[mcp]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
server = "https://hosted.example.test/mcp"
namespace = "hosted"
max_tools = 3
default_permission = "ask"
effects = ["network"]
"#;
        let contracts = HostApiContractRegistry::new();
        let manifest = ExtensionManifestRecord::from_toml(
            raw_manifest,
            ManifestSource::HostBundled,
            &HostPortCatalog::new(vec![HostPortCatalogEntry::new(
                HostPortId::new(HOST_RUNTIME_HTTP_EGRESS_PORT_ID).expect("host port id"),
            )])
            .expect("host port catalog"),
            None,
            &contracts,
        )
        .expect("manifest record");
        let package_manifest: ExtensionManifest = manifest
            .manifest()
            .clone()
            .try_into()
            .expect("package manifest");
        let package = ExtensionPackage::from_manifest_toml(
            package_manifest,
            VirtualPath::new("/system/extensions/hosted").expect("package root"),
            raw_manifest,
        )
        .expect("package");
        (package, manifest)
    }

    fn installation(extension_id: &str, members: &[&str]) -> ExtensionInstallation {
        let extension_id = ExtensionId::new(extension_id).expect("extension id");
        let owner = InstallationOwner::users(
            members
                .iter()
                .map(|member| UserId::new(*member).expect("user id"))
                .collect::<BTreeSet<_>>(),
        )
        .expect("non-empty owner");
        ExtensionInstallation::new(
            ExtensionInstallationId::new(extension_id.as_str()).expect("installation id"),
            extension_id.clone(),
            ExtensionManifestRef::new(extension_id, None),
            Vec::new(),
            Utc::now(),
            owner,
        )
        .expect("installation")
    }
}
