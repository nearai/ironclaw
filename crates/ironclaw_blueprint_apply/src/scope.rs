//! Apply scope + authority gating.
//!
//! The epic invariant: applying anything beyond `scope = { user = self }`
//! requires admin authority, and scope can never *widen* authority. This module
//! keeps the rule self-contained and dependency-light; the real wiring binds
//! [`Actor::is_admin`] to `AdminScope` from `src/tenant.rs` at the call site.

use ironclaw_blueprint::Blueprint;

use crate::error::AuthorityError;

/// The scope an apply targets, lifted from the blueprint's `[scope]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyScope {
    pub tenant: Option<String>,
    pub user: Option<String>,
    pub project: Option<String>,
    pub agent: Option<String>,
}

impl ApplyScope {
    pub fn from_blueprint(blueprint: &Blueprint) -> Self {
        let scope = &blueprint.scope;
        Self {
            tenant: scope.tenant.clone(),
            user: scope.user.clone(),
            project: scope.project.clone(),
            agent: scope.agent.clone(),
        }
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = &self.tenant {
            parts.push(format!("tenant={t}"));
        }
        if let Some(u) = &self.user {
            parts.push(format!("user={u}"));
        }
        if let Some(p) = &self.project {
            parts.push(format!("project={p}"));
        }
        if let Some(a) = &self.agent {
            parts.push(format!("agent={a}"));
        }
        if parts.is_empty() {
            "system".to_string()
        } else {
            parts.join(",")
        }
    }
}

/// The actor performing the apply.
#[derive(Debug, Clone)]
pub struct Actor {
    pub user_id: String,
    pub is_admin: bool,
}

impl Actor {
    pub fn user(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            is_admin: false,
        }
    }

    pub fn admin(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            is_admin: true,
        }
    }
}

/// Authorize an actor to apply at a scope. Fails closed.
///
/// A non-admin may apply only to themselves: no tenant/project/agent scope, and
/// `user` either absent or equal to their own id. Everything else needs admin.
pub fn authorize(actor: &Actor, scope: &ApplyScope) -> Result<(), AuthorityError> {
    if actor.is_admin {
        return Ok(());
    }

    let targets_self_only = scope.tenant.is_none()
        && scope.project.is_none()
        && scope.agent.is_none()
        && scope.user.as_ref().is_none_or(|u| *u == actor.user_id);

    if targets_self_only {
        Ok(())
    } else {
        Err(AuthorityError {
            scope: scope.describe(),
            reason: "non-admin actors may only apply to their own user scope".to_string(),
        })
    }
}
