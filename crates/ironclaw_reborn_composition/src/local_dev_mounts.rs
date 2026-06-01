use std::{collections::HashSet, path::Path};

use ironclaw_host_api::{
    HostApiError, MountAlias, MountGrant, MountPermissions, MountView, VirtualPath,
};

const WORKSPACE_ALIAS: &str = "/workspace";
const WORKSPACE_TARGET: &str = "/projects/workspace";
const HOST_ALIAS: &str = "/host";
const HOST_TARGET: &str = "/projects/host";

pub(crate) fn workspace_mount_view(
    permissions: MountPermissions,
    host_home_aliases: &[&Path],
) -> Result<MountView, HostApiError> {
    ambient_workspace_mount_view(permissions, &[], host_home_aliases)
}

/// Build the workspace mount view used by local-dev capability grants.
///
/// `workspace_aliases` is load-bearing for local-dev-yolo ambient coding tools:
/// callers must pass it only under a yolo runtime policy. Non-yolo local-dev
/// must pass an empty slice so raw host workspace paths stay denied.
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

pub(crate) fn skill_context_mount_view() -> Result<MountView, HostApiError> {
    MountView::new(vec![
        grant("/skills", "/projects/skills", MountPermissions::read_only())?,
        grant(
            "/tenant-shared/skills",
            "/projects/tenant-shared/skills",
            MountPermissions::read_only(),
        )?,
        grant(
            "/system/skills",
            "/projects/system/skills",
            MountPermissions::read_only(),
        )?,
    ])
}

pub(crate) fn skill_management_mount_view() -> Result<MountView, HostApiError> {
    MountView::new(vec![
        grant(
            "/skills",
            "/projects/skills",
            MountPermissions::read_write_list_delete(),
        )?,
        grant(
            "/system/skills",
            "/projects/system/skills",
            MountPermissions::read_only(),
        )?,
    ])
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
mod tests {
    use super::*;

    #[test]
    fn ambient_workspace_mount_rejects_invalid_workspace_alias() {
        let err = ambient_workspace_mount_view(
            MountPermissions::read_write(),
            &[Path::new(r"C:\Users\alice\project")],
            &[],
        )
        .expect_err("invalid workspace alias should fail loudly");

        assert!(
            err.to_string().contains("backslashes are not allowed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn workspace_mount_rejects_host_home_alias_that_is_not_mount_shaped() {
        let err = workspace_mount_view(
            MountPermissions::read_write(),
            &[Path::new(r"C:\Users\alice")],
        )
        .expect_err("invalid raw alias should fail loudly");

        assert!(
            err.to_string().contains("backslashes are not allowed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ambient_workspace_mount_deduplicates_workspace_alias_against_canonical_workspace() {
        let mounts = ambient_workspace_mount_view(
            MountPermissions::read_write(),
            &[Path::new(WORKSPACE_ALIAS)],
            &[],
        )
        .expect("mount view builds");

        assert_eq!(
            mounts
                .mounts
                .iter()
                .filter(|mount| mount.alias.as_str() == WORKSPACE_ALIAS)
                .count(),
            1
        );
    }

    #[test]
    fn workspace_mount_deduplicates_normalized_host_home_aliases() {
        let mounts = workspace_mount_view(
            MountPermissions::read_write(),
            &[
                Path::new("/Users/alice"),
                Path::new("/Users/alice/"),
                Path::new("/Users/alice/."),
            ],
        )
        .expect("mount view builds");

        assert_eq!(
            mounts
                .mounts
                .iter()
                .filter(|mount| mount.alias.as_str() == "/Users/alice")
                .count(),
            1
        );
    }

    #[test]
    fn ambient_workspace_mount_includes_raw_workspace_alias() {
        let mounts = ambient_workspace_mount_view(
            MountPermissions::read_write(),
            &[Path::new("/Users/alice/project")],
            &[Path::new("/Users/alice")],
        )
        .expect("mount view builds");

        let mount_for = |alias: &str| {
            mounts
                .mounts
                .iter()
                .find(|mount| mount.alias.as_str() == alias)
                .unwrap_or_else(|| panic!("missing mount alias {alias}"))
        };
        assert_eq!(
            mount_for("/Users/alice/project").target.as_str(),
            WORKSPACE_TARGET
        );
        assert_eq!(mount_for("/Users/alice").target.as_str(), HOST_TARGET);
    }
}
