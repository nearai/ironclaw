use ironclaw_filesystem::FilesystemError;
use ironclaw_host_api::{ids::SecretHandle, path::ScopedPath, resource::ResourceScope};

use crate::{
    AuthFlowId, AuthInteractionId, AuthProductError, AuthProviderId, AuthSurface,
    CredentialAccountId,
};
use sha2::{Digest as _, Sha256};

pub(super) fn flow_path(
    scope: &crate::AuthProductScope,
    flow_id: AuthFlowId,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/flows/{flow_id}.json",
        product_auth_root(scope)
    ))
}

pub(super) fn flow_root(scope: &crate::AuthProductScope) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!("{}/flows", product_auth_root(scope)))
}

pub(super) fn setup_creation_coordination_path(
    scope: &crate::AuthProductScope,
    provider: &AuthProviderId,
) -> Result<ScopedPath, AuthProductError> {
    // Provider ids are validated public text, not path segments. Hash the
    // complete id so no provider-controlled punctuation can change the
    // coordination namespace.
    let provider_digest = hex::encode(Sha256::digest(provider.as_str().as_bytes()));
    scoped_path(&format!(
        "{}/.setup-creation/{provider_digest}.json",
        flow_root(scope)?.as_str()
    ))
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
    scope: &crate::AuthProductScope,
    interaction_id: AuthInteractionId,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/interactions/{interaction_id}.json",
        product_auth_root(scope)
    ))
}

/// Account addressing takes a [`ResourceScope`], not an `AuthProductScope`:
/// surface and session no longer segment an account path, so accepting a full
/// scope would leave a no-op parameter that reads as if addressing still
/// depended on it.
pub(super) fn account_path(
    resource: &ResourceScope,
    account_id: CredentialAccountId,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/accounts/{account_id}.json",
        credential_owner_root(resource)
    ))
}

pub(super) fn account_root(resource: &ResourceScope) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!("{}/accounts", credential_owner_root(resource)))
}

/// Legacy account root: the pre-migration layout, keyed by agent, project,
/// surface and session. Read-only — the migration copies records out of these
/// roots and nothing writes to them again.
pub(super) fn legacy_account_root(
    scope: &crate::AuthProductScope,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!("{}/accounts", product_auth_root(scope)))
}

/// Directory holding one entry per agent that ever owned auth records for this
/// user. Migration-only: the legacy layout keyed accounts by agent, so finding
/// them all means enumerating the agents rather than guessing the reader's.
pub(super) fn legacy_agents_root() -> Result<ScopedPath, AuthProductError> {
    scoped_path("/secrets/agents")
}

/// Directory holding one entry per project, optionally beneath a legacy agent.
pub(super) fn legacy_projects_root(agent: Option<&str>) -> Result<ScopedPath, AuthProductError> {
    match agent {
        Some(agent) => scoped_path(&format!("/secrets/agents/{agent}/projects")),
        None => scoped_path("/secrets/projects"),
    }
}

/// Marker recording that this owner's accounts have been migrated out of the
/// legacy per-agent/surface/session roots. Durable rather than process-local so
/// the scan runs once per owner across restarts, not once per process.
pub(super) fn account_migration_marker_path(
    resource: &ResourceScope,
) -> Result<ScopedPath, AuthProductError> {
    scoped_path(&format!(
        "{}/.accounts-migrated-v2",
        credential_owner_root(resource)
    ))
}

/// The single address a credential account lives at.
///
/// A credential belongs to a **tenant + user** — and, once project-scoped
/// credentials are elected at write time, optionally to a project underneath
/// that user. It deliberately does NOT key on `agent_id`, `thread_id`,
/// `mission_id`, `surface` or `session_id`: a user authorizes Notion once, not
/// once per agent, per browser session, or per screen they happened to use.
///
/// Tenant and user are not in the string because they are the mount itself
/// (`/tenants/{tenant}/users/{user}/secrets` — see `ScopedFilesystem`), which
/// is also what keeps one user's credentials unreachable from another's.
///
/// `/product-auth` stays because `/secrets` is a **shared mount**: `ironclaw_secrets`
/// owns `{owner}/secrets` and `{owner}/secret-leases` under it, so this segment is
/// what keeps two crates from colliding in one namespace.
fn credential_owner_root(resource: &ResourceScope) -> String {
    let mut base = String::from("/secrets");
    if let Some(project_id) = &resource.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    base.push_str("/product-auth");
    base
}

fn product_auth_root(scope: &crate::AuthProductScope) -> String {
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
        crate::AuthSurface::Chat => "chat",
        crate::AuthSurface::Web => "web",
        crate::AuthSurface::Cli => "cli",
        crate::AuthSurface::Tui => "tui",
        crate::AuthSurface::Api => "api",
        crate::AuthSurface::SetupAdmin => "setup-admin",
        crate::AuthSurface::Callback => "callback",
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
