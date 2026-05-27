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
    let mut mounts = vec![grant(
        WORKSPACE_ALIAS,
        WORKSPACE_TARGET,
        permissions.clone(),
    )?];
    if !host_home_aliases.is_empty() {
        mounts.push(grant(HOST_ALIAS, HOST_TARGET, permissions.clone())?);
        let mut seen_aliases = HashSet::new();
        for host_home_alias in host_home_aliases {
            let Some(host_home_alias) = host_home_alias.to_str() else {
                return Err(HostApiError::InvalidPath {
                    value: "<non-utf8-host-home-alias>".to_string(),
                    reason: "confirmed host-home alias must be valid UTF-8".to_string(),
                });
            };
            let raw_host_home_alias = MountAlias::new(host_home_alias.to_string())?;
            if !seen_aliases.insert(raw_host_home_alias.as_str().to_string()) {
                continue;
            }
            mounts.push(MountGrant::new(
                raw_host_home_alias,
                VirtualPath::new(HOST_TARGET)?,
                permissions.clone(),
            ));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
