use axum::http::StatusCode;

use crate::channels::web::auth::UserIdentity;
use crate::channels::web::handlers::workspaces::ResolvedWorkspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Member,
    Admin,
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    WorkspaceRead,
    WorkspaceWrite,
    WorkspaceManageMembers,
    WorkspaceManageSettings,
    WorkspaceManageAdmins,
    WorkspaceDelete,
    SystemManageUsers,
    SystemViewAll,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Member => "member",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn parse(role: &str) -> Result<Self, (StatusCode, String)> {
        match role {
            "viewer" => Ok(Self::Viewer),
            "member" => Ok(Self::Member),
            "admin" => Ok(Self::Admin),
            "owner" => Ok(Self::Owner),
            _ => Err((
                StatusCode::FORBIDDEN,
                format!("Unknown workspace role '{role}'"),
            )),
        }
    }

    pub fn has_permission(self, permission: Permission) -> bool {
        match permission {
            Permission::WorkspaceRead => true,
            Permission::WorkspaceWrite => self >= Self::Member,
            Permission::WorkspaceManageMembers => self >= Self::Admin,
            Permission::WorkspaceManageSettings => self >= Self::Admin,
            Permission::WorkspaceManageAdmins => self >= Self::Owner,
            Permission::WorkspaceDelete => self >= Self::Owner,
            Permission::SystemManageUsers | Permission::SystemViewAll => false,
        }
    }
}

pub fn superadmin_workspace_role() -> Role {
    Role::Owner
}

pub fn require_system_permission(
    user: &UserIdentity,
    permission: Permission,
) -> Result<(), (StatusCode, String)> {
    if user.is_superadmin {
        return Ok(());
    }

    Err((
        StatusCode::FORBIDDEN,
        match permission {
            Permission::SystemManageUsers | Permission::SystemViewAll => "Superadmin required",
            _ => "Insufficient permissions",
        }
        .to_string(),
    ))
}

pub fn require_workspace_permission(
    user: &UserIdentity,
    scope: Option<&ResolvedWorkspace>,
    permission: Permission,
) -> Result<(), (StatusCode, String)> {
    let Some(scope) = scope else {
        return Ok(());
    };

    if user.is_superadmin {
        return Ok(());
    }

    let role = Role::parse(&scope.role)?;
    if role.has_permission(permission) {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, permission_denied_message(permission)))
    }
}

fn permission_denied_message(permission: Permission) -> String {
    match permission {
        Permission::WorkspaceRead => "Workspace read access required".to_string(),
        Permission::WorkspaceWrite => "Workspace member role required".to_string(),
        Permission::WorkspaceManageMembers | Permission::WorkspaceManageSettings => {
            "Workspace admin or owner role required".to_string()
        }
        Permission::WorkspaceManageAdmins | Permission::WorkspaceDelete => {
            "Workspace owner role required".to_string()
        }
        Permission::SystemManageUsers | Permission::SystemViewAll => {
            "Superadmin required".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Permission, Role};

    #[test]
    fn workspace_permission_matrix_matches_issue_1608() {
        let matrix = [
            (Role::Viewer, Permission::WorkspaceRead, true),
            (Role::Viewer, Permission::WorkspaceWrite, false),
            (Role::Viewer, Permission::WorkspaceManageMembers, false),
            (Role::Viewer, Permission::WorkspaceManageSettings, false),
            (Role::Viewer, Permission::WorkspaceManageAdmins, false),
            (Role::Viewer, Permission::WorkspaceDelete, false),
            (Role::Member, Permission::WorkspaceRead, true),
            (Role::Member, Permission::WorkspaceWrite, true),
            (Role::Member, Permission::WorkspaceManageMembers, false),
            (Role::Member, Permission::WorkspaceManageSettings, false),
            (Role::Member, Permission::WorkspaceManageAdmins, false),
            (Role::Member, Permission::WorkspaceDelete, false),
            (Role::Admin, Permission::WorkspaceRead, true),
            (Role::Admin, Permission::WorkspaceWrite, true),
            (Role::Admin, Permission::WorkspaceManageMembers, true),
            (Role::Admin, Permission::WorkspaceManageSettings, true),
            (Role::Admin, Permission::WorkspaceManageAdmins, false),
            (Role::Admin, Permission::WorkspaceDelete, false),
            (Role::Owner, Permission::WorkspaceRead, true),
            (Role::Owner, Permission::WorkspaceWrite, true),
            (Role::Owner, Permission::WorkspaceManageMembers, true),
            (Role::Owner, Permission::WorkspaceManageSettings, true),
            (Role::Owner, Permission::WorkspaceManageAdmins, true),
            (Role::Owner, Permission::WorkspaceDelete, true),
        ];

        for (role, permission, allowed) in matrix {
            assert_eq!(
                role.has_permission(permission),
                allowed,
                "role={} permission={permission:?}",
                role.as_str()
            );
        }
    }
}
