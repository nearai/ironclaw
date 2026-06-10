use ironclaw_filesystem::FilesystemError;
use ironclaw_host_api::{ResourceScope, ScopedPath, SecretHandle};

use ironclaw_auth::{
    AuthFlowId, AuthInteractionId, AuthProductError, AuthSurface, CredentialAccountId,
};

pub(super) fn flow_path(
    scope: &ironclaw_auth::AuthProductScope,
    flow_id: AuthFlowId,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/flows/{flow_id}.json",
        product_auth_root(scope)
    ))
}

pub(super) fn flow_root(
    scope: &ironclaw_auth::AuthProductScope,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!("{}/flows", product_auth_root(scope)))
}

pub(super) fn surface_sessions_root(
    resource: &ResourceScope,
    surface: AuthSurface,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/{}/sessions",
        product_auth_base_root(resource),
        surface_path_segment(surface)
    ))
}

pub(super) fn interaction_path(
    scope: &ironclaw_auth::AuthProductScope,
    interaction_id: AuthInteractionId,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/interactions/{interaction_id}.json",
        product_auth_root(scope)
    ))
}

pub(super) fn account_path(
    scope: &ironclaw_auth::AuthProductScope,
    account_id: CredentialAccountId,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/accounts/{account_id}.json",
        product_auth_root(scope)
    ))
}

pub(super) fn account_root(
    scope: &ironclaw_auth::AuthProductScope,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!("{}/accounts", product_auth_root(scope)))
}

fn product_auth_root(scope: &ironclaw_auth::AuthProductScope) -> String {
    let mut base = product_auth_base_root(&scope.resource);
    base.push('/');
    base.push_str(surface_path_segment(scope.surface));
    if let Some(session_id) = &scope.session_id {
        base.push_str("/sessions/");
        base.push_str(session_id.as_str());
    }
    base
}

fn product_auth_base_root(resource: &ResourceScope) -> String {
    let mut base = String::from("/secrets");
    if let Some(agent_id) = &resource.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &resource.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    base.push_str("/product-auth");
    base
}

fn surface_path_segment(surface: AuthSurface) -> &'static str {
    match surface {
        ironclaw_auth::AuthSurface::Chat => "chat",
        ironclaw_auth::AuthSurface::Web => "web",
        ironclaw_auth::AuthSurface::Cli => "cli",
        ironclaw_auth::AuthSurface::Tui => "tui",
        ironclaw_auth::AuthSurface::Api => "api",
        ironclaw_auth::AuthSurface::SetupAdmin => "setup-admin",
        ironclaw_auth::AuthSurface::Callback => "callback",
    }
}

fn scoped_path(raw: &str) -> Result<ScopedPath, AuthProductError> {
    ScopedPath::new(raw).map_err(|_| AuthProductError::BackendUnavailable)
}

pub(super) fn join_scoped(prefix: &ScopedPath, leaf: &str) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/{}",
        prefix.as_str().trim_end_matches('/'),
        leaf
    ))
}

pub(super) fn manual_token_secret_handle(
    account_id: CredentialAccountId,
    interaction_id: AuthInteractionId,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("product-auth-manual-{account_id}-{interaction_id}"))
        .map_err(|_| AuthProductError::BackendUnavailable)
}

pub(super) fn fs_error(error: FilesystemError) -> AuthProductError {
    match error {
        // CAS precondition failure — callers can detect and retry on BackendConflict.
        FilesystemError::VersionMismatch { .. } => AuthProductError::BackendConflict,
        _ => AuthProductError::BackendUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_token_secret_handle_is_uuid_derived_name_segment() {
        let interaction_id = AuthInteractionId::new();
        let account_id = CredentialAccountId::from_uuid(interaction_id.as_uuid());

        let handle = manual_token_secret_handle(account_id, interaction_id)
            .expect("UUID-derived handle should be a valid SecretHandle");

        assert_eq!(
            handle.as_str(),
            format!("product-auth-manual-{account_id}-{interaction_id}")
        );
    }
}
