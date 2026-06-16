use serde::{Deserialize, Serialize};

use crate::ids::AuthSessionId;

/// Product surface that initiated or renders an auth flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSurface {
    Chat,
    Web,
    Cli,
    Tui,
    Api,
    SetupAdmin,
    Callback,
}

impl AuthSurface {
    pub const ALL: [Self; 7] = [
        Self::Api,
        Self::Callback,
        Self::Web,
        Self::Chat,
        Self::Cli,
        Self::Tui,
        Self::SetupAdmin,
    ];
}

/// Scoped product auth owner.
///
/// Durable implementations should key records by this scope plus the opaque
/// flow/interaction/account id. `session_id` is typed and validated so stored
/// records cannot rehydrate invalid UI/session refs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthProductScope {
    pub resource: ironclaw_host_api::ResourceScope,
    pub surface: AuthSurface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<AuthSessionId>,
}

impl AuthProductScope {
    pub fn new(resource: ironclaw_host_api::ResourceScope, surface: AuthSurface) -> Self {
        Self {
            resource,
            surface,
            session_id: None,
        }
    }

    pub fn with_session_id(mut self, session_id: AuthSessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Build an owner-granularity product-auth scope for credential-account
    /// lookups from a runtime `resource` scope.
    ///
    /// Credential accounts (OAuth tokens, manual tokens, runtime credential
    /// accounts) are owned by the tenant/user/agent/project — NOT by a single
    /// thread or mission. A credential a user authorizes in one chat thread
    /// must stay resolvable from every other thread of the same owner; "thread"
    /// is not an ownership class. This drops the transient `mission_id`/
    /// `thread_id` (via [`ResourceScope::without_thread_and_mission`]) so an
    /// owner-scoped account read matches across the owner's threads instead of
    /// binding the credential to the thread it was authorized in.
    ///
    /// This is the credential-ownership contract for the runtime-resolution
    /// path — the single source of truth shared by every runtime credential
    /// resolver (the host-runtime generic resolver and the first-party GSuite
    /// resolver). Do not re-derive the strip inline — that drift is exactly
    /// what bound Google credentials to a thread.
    ///
    /// [`ResourceScope::without_thread_and_mission`]: ironclaw_host_api::ResourceScope::without_thread_and_mission
    pub fn credential_owner(
        resource: &ironclaw_host_api::ResourceScope,
        surface: AuthSurface,
    ) -> Self {
        Self::new(resource.without_thread_and_mission(), surface)
    }

    /// Owner-granularity copy of this auth scope (drops the transient
    /// `thread_id`/`mission_id`), preserving `surface` and `session_id`.
    ///
    /// Use when projecting an existing [`AuthProductScope`] to credential-owner
    /// granularity while keeping its surface/session segmentation — e.g. an
    /// OAuth reconnect that must bind to the owner's existing account. See
    /// [`Self::credential_owner`] for the ownership contract this enforces.
    pub fn to_credential_owner(&self) -> Self {
        Self {
            resource: self.resource.without_thread_and_mission(),
            ..self.clone()
        }
    }
}
