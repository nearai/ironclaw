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
}
