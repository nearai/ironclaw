//! Runtime mount views selected from the resolved filesystem configuration.

use std::{collections::HashSet, path::Path, sync::Arc};

use ironclaw_filesystem::{CompositeRootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    error::HostApiError,
    ids::TenantUserWorkspaceKey,
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_memory::MemoryDocumentScope;

// One owner for the workspace mount alias: `ironclaw_attachments` also uses it
// to decide which model-text references become egress attachments.
pub(crate) use ironclaw_attachments::WORKSPACE_ALIAS;

const WORKSPACE_TARGET: &str = "/projects/workspace";

const HOST_ALIAS: &str = "/host";
const HOST_TARGET: &str = "/projects/host";
const MEMORY_ALIAS: &str = "/memory";
const MEMORY_TARGET: &str = "/memory";

pub(crate) fn workspace_mount_view(
    permissions: MountPermissions,
    host_home_aliases: &[&Path],
) -> Result<MountView, HostApiError> {
    ambient_workspace_mount_view(permissions, &[], host_home_aliases)
}

/// Build the workspace mount view used by capability grants.
///
/// `workspace_aliases` is load-bearing for ambient host coding tools:
/// callers must pass it only under a policy that grants ambient host access. Other
/// profiles must pass an empty slice so raw host workspace paths stay denied.
pub(crate) fn ambient_workspace_mount_view(
    permissions: MountPermissions,
    workspace_aliases: &[&Path],
    host_home_aliases: &[&Path],
) -> Result<MountView, HostApiError> {
    let mut mounts = vec![grant(
        WORKSPACE_ALIAS,
        WORKSPACE_TARGET,
        permissions.clone(),
    )?];
    push_raw_alias_mounts(
        &mut mounts,
        workspace_aliases,
        WORKSPACE_TARGET,
        permissions.clone(),
        "workspace alias",
    )?;
    if !host_home_aliases.is_empty() {
        mounts.push(grant(HOST_ALIAS, HOST_TARGET, permissions.clone())?);
        push_raw_alias_mounts(
            &mut mounts,
            host_home_aliases,
            HOST_TARGET,
            permissions.clone(),
            "confirmed host-home alias",
        )?;
    }
    MountView::new(mounts)
}

/// The single decision about where skills live. Every skill mount view derives from this.
///
/// nearai/ironclaw#7168 was three views over two trees: the agent's in-run port and discovery
/// resolved `/skills` to host disk while Settings → Skills resolved it to the database. So an
/// install reported success, showed in `skill_list` for that turn, never appeared in Settings, and
/// could not be activated again. Anything built from this function agrees by construction.
///
/// `/system/skills` is alias-equals-target on purpose: the composite decides which world it lands
/// in, so the bundle is reachable whether it was seeded to a host disk (local-dev, single-tenant)
/// or into Postgres (multi-tenant, where no tenant disk exists), and neither reader nor writer has
/// to know which.
fn db_backed_skill_grants(
    scope: &ResourceScope,
    user_skill_permissions: MountPermissions,
) -> Result<Vec<MountGrant>, HostApiError> {
    Ok(vec![
        grant(
            "/skills",
            &format!(
                "/tenants/{}/users/{}/skills",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ),
            user_skill_permissions,
        )?,
        grant(
            "/system/skills",
            "/system/skills",
            MountPermissions::read_only(),
        )?,
    ])
}

/// Read-side skill mounts: discovery, listing, and activation.
///
/// Adds the tenant-shared root, which has no writer. Its target is `/tenants/<t>/shared/skills`,
/// matching where `invocation_mount_view` puts every other tenant-shared root; repeating the alias
/// inside the target instead pointed at a subtree nothing populates. Pinned by
/// `tenant_shared_skills_resolve_under_the_canonical_shared_subtree`.
pub(crate) fn db_backed_skill_context_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, HostApiError> {
    let mut grants = db_backed_skill_grants(scope, MountPermissions::read_only())?;
    grants.push(grant(
        "/tenant-shared/skills",
        &format!("/tenants/{}/shared/skills", scope.tenant_id.as_str()),
        MountPermissions::read_only(),
    )?);
    MountView::new(grants)
}

/// Write-side skill mounts: `skill_install`, `skill_update`, `skill_remove`.
///
/// Resolves `/skills` to the same target as [`db_backed_skill_context_mount_view`]; a test pins
/// that, because divergence means a skill that exists and can never be found.
pub(crate) fn db_backed_skill_management_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, HostApiError> {
    MountView::new(db_backed_skill_grants(
        scope,
        MountPermissions::read_write_list_delete(),
    )?)
}

pub(crate) fn memory_mount_view(permissions: MountPermissions) -> Result<MountView, HostApiError> {
    MountView::new(vec![grant(MEMORY_ALIAS, MEMORY_TARGET, permissions)?])
}

#[cfg(test)]
pub(crate) fn system_extensions_lifecycle_mount_view() -> Result<MountView, HostApiError> {
    MountView::new(vec![grant(
        "/system/extensions",
        "/system/extensions",
        MountPermissions::read_write_list_delete(),
    )?])
}

/// Read-only mount view backing the standalone WebUI filesystem viewer.
///
/// Spans every mount the read-only browser can navigate — the workspace
/// (project working files + landed attachments) and the persistent memory store
/// — over the same targets the agent's own tools resolve through, so the viewer
/// shows exactly what the agent sees. Read-only by construction: the viewer is a
/// navigation + preview/download surface, never a write path. The aliases here
/// are the contract the browse reader confines against; keep them aligned with
/// [`BROWSE_MEMORY_ALIAS`]/[`WORKSPACE_ALIAS`].
pub(crate) const BROWSE_MEMORY_ALIAS: &str = MEMORY_ALIAS;

/// Per-caller workspace mount view for agent read/write filesystem access.
/// Maps `WORKSPACE_ALIAS` to the caller's own subtree under
/// `/projects/workspace/users/<tenant-user-digest>`, mirroring memory's
/// per-caller scoping, so agent tool/attachment writes land in the caller's
/// private subtree and the WebUI browser (which reads the same subtree) can
/// surface them. A missing subtree renders empty on read rather than falling
/// back to the shared root.
pub(crate) fn scoped_workspace_mount_view(
    scope: &ResourceScope,
    permissions: MountPermissions,
) -> Result<MountView, HostApiError> {
    let target = scoped_workspace_target(scope)?;
    MountView::new(vec![grant(WORKSPACE_ALIAS, target.as_str(), permissions)?])
}

/// How a deployment keys the workspace mount, resolved once at composition
/// from [`crate::deployment::DeploymentConfig::workspace_scoped_per_caller`].
///
/// Every workspace *write* lane reads this one value — capability grant
/// minting, approval lease terms, the WebUI attachment handle, and the
/// channel-inbound attachment lander — so the agent and the WebUI browser can
/// never address different subtrees. Under [`Self::PerCaller`] there is no
/// shared view to reach for: the shared `/projects/workspace` root is simply
/// not representable.
#[derive(Clone, Debug)]
pub(crate) enum WorkspaceMountPolicy {
    /// One ambient workspace view for the whole deployment, including any raw
    /// host aliases the runtime policy granted. Local single-user coding
    /// profiles depend on those aliases, so this view is never scoped.
    Shared(MountView),
    /// Workspace mounts key the caller's own subtree under
    /// `/projects/workspace/users/<tenant-user-digest>`.
    PerCaller,
}

impl WorkspaceMountPolicy {
    /// The deployment's single workspace scoping decision, turned into the
    /// policy every write lane reads.
    ///
    /// One constructor so the `PerCaller` / `Shared` branch cannot drift
    /// between assembly sites: a deployment that scopes per caller must never
    /// end up with an ambient shared view on one lane and a scoped view on
    /// another. `workspace_aliases` / `host_home_aliases` are the raw host
    /// aliases only ambient-access profiles may carry; a per-caller deployment
    /// gets no ambient view at all, so they are ignored on that branch.
    pub(crate) fn resolve(
        scoped_per_caller: bool,
        workspace_aliases: &[&Path],
        host_home_aliases: &[&Path],
    ) -> Result<Self, HostApiError> {
        if scoped_per_caller {
            return Ok(Self::PerCaller);
        }
        Ok(Self::Shared(ambient_workspace_mount_view(
            MountPermissions::read_write(),
            workspace_aliases,
            host_home_aliases,
        )?))
    }

    /// The workspace view used to mint capability grants and approval lease
    /// terms for a run/gate resolved to `scope`.
    ///
    /// The per-caller permissions match the ambient grant view's
    /// (`read_write`), so turning scoping on narrows *where* a grant points,
    /// never what it may do.
    pub(crate) fn capability_grant_view(
        &self,
        scope: &ResourceScope,
    ) -> Result<MountView, HostApiError> {
        match self {
            Self::Shared(view) => Ok(view.clone()),
            Self::PerCaller => scoped_workspace_mount_view(scope, MountPermissions::read_write()),
        }
    }

    /// The WebUI browser's read-only workspace target for one caller.
    ///
    /// Memory remains caller-scoped regardless of this decision. The browser
    /// must, however, use the same workspace policy as the capability grants
    /// and attachment writers: shared deployments browse the shared workspace
    /// root, while scoped deployments browse only the caller's digest leaf.
    fn browse_workspace_target(&self, scope: &ResourceScope) -> Result<VirtualPath, HostApiError> {
        match self {
            Self::Shared(_) => VirtualPath::new(WORKSPACE_TARGET),
            Self::PerCaller => scoped_workspace_target(scope),
        }
    }
}

/// The read-write workspace handle every write lane that lands *bytes* uses:
/// the WebUI attachment/upload seam, the channel-inbound attachment lander, and
/// the C-ATTACH test seam.
///
/// Single owner of this recipe, so the handles cannot drift. Deliberately not
/// the read-only `workspace_filesystem` handle, which backs setup-marker reads
/// and fails closed with `PermissionDenied` on write.
pub(crate) fn read_write_workspace_filesystem(
    extension_filesystem: &Arc<CompositeRootFilesystem>,
    policy: &WorkspaceMountPolicy,
) -> Option<Arc<ScopedFilesystem<CompositeRootFilesystem>>> {
    let permissions = MountPermissions::read_write_list_delete();
    match policy {
        // Per-caller: the resolver runs on every call, so each authenticated
        // caller keys its own `users/<tenant-user-digest>` subtree and the
        // shared `/projects/workspace` root is never exposed for writes.
        WorkspaceMountPolicy::PerCaller => Some(Arc::new(ScopedFilesystem::new(
            Arc::clone(extension_filesystem),
            move |scope| scoped_workspace_mount_view(scope, permissions.clone()),
        ))),
        WorkspaceMountPolicy::Shared(_) => Some(Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(extension_filesystem),
            // silent-ok: the alias-free workspace mount view is built from
            // compile-time constants with no caller input, so the only failure
            // mode is a malformed constant that every other call site would
            // reject at startup. `None` here disables the write lane rather
            // than serving an unscoped one.
            workspace_mount_view(permissions, &[]).ok()?,
        ))),
    }
}

pub(crate) fn webui_browse_mount_view(
    policy: &WorkspaceMountPolicy,
    scope: &ResourceScope,
) -> Result<MountView, HostApiError> {
    let memory_target = scoped_memory_target(scope)?;
    let workspace_target = policy.browse_workspace_target(scope)?;
    MountView::new(vec![
        grant(
            WORKSPACE_ALIAS,
            workspace_target.as_str(),
            MountPermissions::read_only(),
        )?,
        grant(
            MEMORY_ALIAS,
            memory_target.as_str(),
            MountPermissions::read_only(),
        )?,
    ])
}

pub(crate) fn scoped_workspace_target(scope: &ResourceScope) -> Result<VirtualPath, HostApiError> {
    let workspace_key = TenantUserWorkspaceKey::from_scope(scope);
    VirtualPath::new(format!(
        "{WORKSPACE_TARGET}/users/{}",
        workspace_key.digest_segment()
    ))
}

fn scoped_memory_target(scope: &ResourceScope) -> Result<VirtualPath, HostApiError> {
    MemoryDocumentScope::new_with_agent(
        scope.tenant_id.as_str(),
        scope.user_id.as_str(),
        scope.agent_id.as_ref().map(|id| id.as_str()),
        scope.project_id.as_ref().map(|id| id.as_str()),
    )?
    .virtual_prefix()
}

fn grant(
    alias: &str,
    target: &str,
    permissions: MountPermissions,
) -> Result<MountGrant, HostApiError> {
    Ok(MountGrant::new(
        MountAlias::new(alias)?,
        VirtualPath::new(target)?,
        permissions,
    ))
}

fn push_raw_alias_mounts(
    mounts: &mut Vec<MountGrant>,
    aliases: &[&Path],
    target: &str,
    permissions: MountPermissions,
    label: &str,
) -> Result<(), HostApiError> {
    let mut seen_aliases = mounts
        .iter()
        .map(|mount| mount.alias.as_str().to_string())
        .collect::<HashSet<_>>();
    for alias in aliases {
        let Some(alias) = alias.to_str() else {
            return Err(HostApiError::InvalidPath {
                value: format!("<non-utf8-{label}>"),
                reason: format!("{label} must be valid UTF-8"),
            });
        };
        let raw_alias = MountAlias::new(alias.to_string())?;
        if !seen_aliases.insert(raw_alias.as_str().to_string()) {
            continue;
        }
        mounts.push(MountGrant::new(
            raw_alias,
            VirtualPath::new(target)?,
            permissions.clone(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_mounts/tests.rs"]
mod tests;
