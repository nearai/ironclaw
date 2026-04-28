//! Mount view contracts for scoped filesystem authority.
//!
//! A [`MountView`] is the filesystem authority visible to one execution
//! context. It maps extension-facing aliases such as `/workspace` or
//! `/extension/state` to canonical [`VirtualPath`] roots with explicit
//! [`MountPermissions`]. Resolution is lexical and fail-closed; backend-specific
//! symlink and storage containment checks belong in `ironclaw_filesystem`.

use serde::{Deserialize, Serialize};

use crate::{HostApiError, MountAlias, ScopedPath, VirtualPath};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountPermissions {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub list: bool,
    pub execute: bool,
}

impl MountPermissions {
    pub fn none() -> Self {
        Self {
            read: false,
            write: false,
            delete: false,
            list: false,
            execute: false,
        }
    }

    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            delete: false,
            list: true,
            execute: false,
        }
    }

    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            delete: false,
            list: true,
            execute: false,
        }
    }

    pub fn is_subset_of(&self, parent: &Self) -> bool {
        (!self.read || parent.read)
            && (!self.write || parent.write)
            && (!self.delete || parent.delete)
            && (!self.list || parent.list)
            && (!self.execute || parent.execute)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountGrant {
    pub alias: MountAlias,
    pub target: VirtualPath,
    pub permissions: MountPermissions,
}

impl MountGrant {
    pub fn new(alias: MountAlias, target: VirtualPath, permissions: MountPermissions) -> Self {
        Self {
            alias,
            target,
            permissions,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountView {
    pub mounts: Vec<MountGrant>,
}

impl MountView {
    pub fn new(mounts: Vec<MountGrant>) -> Result<Self, HostApiError> {
        let view = Self { mounts };
        view.validate()?;
        Ok(view)
    }

    pub fn validate(&self) -> Result<(), HostApiError> {
        let mut seen = std::collections::HashSet::new();
        for mount in &self.mounts {
            if !seen.insert(mount.alias.as_str()) {
                return Err(HostApiError::invalid_mount(
                    mount.alias.as_str(),
                    "duplicate mount alias",
                ));
            }
        }
        Ok(())
    }

    pub fn resolve(&self, path: &ScopedPath) -> Result<VirtualPath, HostApiError> {
        self.resolve_with_grant(path)
            .map(|(virtual_path, _grant)| virtual_path)
    }

    pub fn resolve_with_grant(
        &self,
        path: &ScopedPath,
    ) -> Result<(VirtualPath, &MountGrant), HostApiError> {
        let raw = path.as_str();
        let mount = self
            .mounts
            .iter()
            .filter(|mount| alias_matches(mount.alias.as_str(), raw))
            .max_by_key(|mount| mount.alias.as_str().len())
            .ok_or_else(|| {
                HostApiError::invalid_mount(raw, "no mount alias matches scoped path")
            })?;

        let tail = raw
            .strip_prefix(mount.alias.as_str())
            .unwrap_or_default()
            .trim_start_matches('/');
        Ok((mount.target.join_tail(tail)?, mount))
    }

    /// Returns true when every child mount is present in the parent with the
    /// same alias and exact same target plus no broader permissions.
    ///
    /// V1 intentionally does not treat a narrower child target (for example, a
    /// child `/workspace -> /projects/p1/subdir`) as a subset of a parent
    /// `/workspace -> /projects/p1`; callers must issue an explicit mount for
    /// each delegated target.
    pub fn is_subset_of(&self, parent: &Self) -> bool {
        self.mounts.iter().all(|child| {
            parent.mounts.iter().any(|parent_mount| {
                child.alias == parent_mount.alias
                    && child.target.as_str() == parent_mount.target.as_str()
                    && child.permissions.is_subset_of(&parent_mount.permissions)
            })
        })
    }
}

fn alias_matches(alias: &str, path: &str) -> bool {
    path == alias || path.starts_with(&format!("{alias}/"))
}
