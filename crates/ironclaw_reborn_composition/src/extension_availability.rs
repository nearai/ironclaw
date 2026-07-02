//! Collapsed model-facing connection-state projection for the extension
//! search path (#5416 Phase 0).
//!
//! A Google (or any credentialed) extension has two independent axes —
//! lifecycle (installed/enabled) and credential presence — and the search
//! surface used to answer "is it ready?" from the lifecycle axis alone. This
//! module is the single place that projects "is it installed?" plus "is the
//! credential present?" into the one value the model actually needs:
//! [`ExtensionAvailability`]. See
//! `docs/plans/2026-07-01-reborn-google-connection-state-5416.md` §4.1.

use ironclaw_extensions::ExtensionInstallation;
use ironclaw_host_api::CredentialStageError;
use ironclaw_product_workflow::{
    ExtensionAvailability, LifecycleSearchExtensionSummary, ProductWorkflowError,
};

use crate::available_extensions::AvailableExtensionPackage;
use crate::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_credential_requirements::package_runtime_credential_auth_requirements;

/// Fan-out width for per-extension credential-readiness checks in the search
/// loop. Mirrors `reborn_services/extensions.rs::EXTENSION_READINESS_CONCURRENCY`
/// (the list path's identical pattern) — kept as a separate constant because
/// the two live in different crates.
pub(crate) const EXTENSION_READINESS_CONCURRENCY: usize = 8;

/// Pure projection: `(is the extension installed, credential gate outcome) ->
/// ExtensionAvailability`.
///
/// `credentials_satisfied` is `None` when the extension has no required
/// credentials to gate on (nothing to check); otherwise it carries the
/// credential gate's `Ok(all_required_present)` / `Err(stage)` outcome.
pub(crate) fn project_availability(
    installed: bool,
    credentials_satisfied: Option<Result<bool, CredentialStageError>>,
) -> ExtensionAvailability {
    if !installed {
        return ExtensionAvailability::NotInstalled;
    }
    match credentials_satisfied {
        None | Some(Ok(true)) => ExtensionAvailability::Available,
        Some(Ok(false)) => ExtensionAvailability::NeedsAuth,
        Some(Err(CredentialStageError::Backend)) => ExtensionAvailability::Unknown,
        // Dead in practice: `missing_requirements` folds `AuthRequired` into
        // `Ok(false)` upstream (product_auth_runtime_credentials.rs). Handled
        // for exhaustiveness with the same fail-mode as a known-missing
        // credential rather than silently panicking on a future caller.
        Some(Err(CredentialStageError::AuthRequired)) => ExtensionAvailability::NeedsAuth,
    }
}

/// `ready` predicate: does at least one summary project to `Available`?
pub(crate) fn any_available(extensions: &[LifecycleSearchExtensionSummary]) -> bool {
    extensions
        .iter()
        .any(|extension| extension.availability == Some(ExtensionAvailability::Available))
}

/// Resolves the full `(installation present?, gate result)` fact for one
/// extension into `ExtensionAvailability`, consulting the credential gate only
/// when the package actually declares required credentials.
pub(crate) async fn resolve_extension_availability(
    extension: &AvailableExtensionPackage,
    installation: Option<&ExtensionInstallation>,
    credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
) -> Result<ExtensionAvailability, ProductWorkflowError> {
    if installation.is_none() {
        return Ok(project_availability(false, None));
    }
    let requirements = package_runtime_credential_auth_requirements(&extension.package);
    if requirements.is_empty() {
        return Ok(project_availability(true, None));
    }
    let Some(credential_gate) = credential_gate else {
        return Ok(project_availability(true, Some(Ok(false))));
    };
    let outcome = credential_gate
        .missing_requirements(requirements)
        .await
        .map(|missing| missing.is_empty());
    Ok(project_availability(true, Some(outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_installed_is_not_installed_regardless_of_credentials() {
        assert_eq!(
            project_availability(false, None),
            ExtensionAvailability::NotInstalled
        );
        assert_eq!(
            project_availability(false, Some(Ok(true))),
            ExtensionAvailability::NotInstalled
        );
        assert_eq!(
            project_availability(false, Some(Err(CredentialStageError::Backend))),
            ExtensionAvailability::NotInstalled
        );
    }

    #[test]
    fn installed_with_no_required_credentials_is_available() {
        assert_eq!(
            project_availability(true, None),
            ExtensionAvailability::Available
        );
    }

    #[test]
    fn installed_with_satisfied_credentials_is_available() {
        assert_eq!(
            project_availability(true, Some(Ok(true))),
            ExtensionAvailability::Available
        );
    }

    #[test]
    fn installed_with_missing_credentials_needs_auth() {
        assert_eq!(
            project_availability(true, Some(Ok(false))),
            ExtensionAvailability::NeedsAuth
        );
    }

    #[test]
    fn installed_with_backend_error_is_unknown() {
        assert_eq!(
            project_availability(true, Some(Err(CredentialStageError::Backend))),
            ExtensionAvailability::Unknown
        );
    }

    #[test]
    fn installed_with_auth_required_error_needs_auth() {
        assert_eq!(
            project_availability(true, Some(Err(CredentialStageError::AuthRequired))),
            ExtensionAvailability::NeedsAuth
        );
    }
}
