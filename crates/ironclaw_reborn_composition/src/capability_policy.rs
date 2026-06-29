//! Multi-user capability policy adapters (#5385).
//!
//! Three host-composition pieces that turn the `ironclaw_product_workflow`
//! capability-policy ports into a running feature:
//!
//! * [`FilesystemUserDirectoryStore`] — a persistent, filesystem-backed
//!   [`UserDirectoryStore`] (one JSON document in `reborn-local-dev.db`).
//! * [`UserCapabilitySurfaceResolver`] — a [`CapabilitySurfaceProfileResolver`]
//!   that narrows each member's offered tool surface to their allow-set while
//!   owner/admin keep the full surface.
//! * [`DirectoryAuthenticator`] — a [`WebuiAuthenticator`] that authenticates
//!   the env owner OR a directory user (by login-token hash) and carries the
//!   operator capability for admins/owner.
//!
//! THE owner is env-configured (`IRONCLAW_REBORN_WEBUI_USER_ID`) and is never a
//! directory row; REST mints no owners.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, ContentType, Entry, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{CapabilityId, ResourceScope, ScopedPath, UserId};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
};
use ironclaw_product_workflow::{
    UserDirectoryError, UserDirectoryRecord, UserDirectoryStore, hash_login_token,
    member_allowed_capability_ids,
};
use ironclaw_turns::run_profile::LoopRunContext;
use serde::{Deserialize, Serialize};

use crate::extension_lifecycle_capabilities::ExtensionActivationAuthorizer;
use crate::webui_serve::{WebuiAuthentication, WebuiAuthenticator};

/// Where the directory document lives under the (system-scoped) filesystem.
/// `/authorization` is a writable mount alias (roles + capability grants are
/// authorization data); under the system scope it resolves to the
/// deployment-global `/tenants/__system__/users/__system__/authorization/...`
/// tree. (`/admin` is not a mounted alias.)
const USER_DIRECTORY_PATH: &str = "/authorization/user-directory.json";

fn directory_path() -> Result<ScopedPath, UserDirectoryError> {
    ScopedPath::new(USER_DIRECTORY_PATH)
        .map_err(|error| UserDirectoryError::Backend(error.to_string()))
}

/// The whole directory, persisted as a single JSON document keyed by user id.
/// A single small document keeps reads/writes trivial (no enumeration logic)
/// and is ample for the single-deployment directory this feature targets.
#[derive(Debug, Default, Serialize, Deserialize)]
struct DirectoryDocument {
    #[serde(default)]
    users: BTreeMap<String, UserDirectoryRecord>,
}

/// Persistent [`UserDirectoryStore`] backed by the universal filesystem fabric
/// (libSQL in local-dev). Writes serialize through an in-process mutex and a
/// whole-document read-modify-write, which is correct for the single-process
/// `ironclaw-reborn serve`.
pub(crate) struct FilesystemUserDirectoryStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    write_lock: tokio::sync::Mutex<()>,
}

impl<F> FilesystemUserDirectoryStore<F>
where
    F: RootFilesystem,
{
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            // System scope: the directory is a deployment-global, admin-owned
            // structure with no per-tenant/user identity (see ResourceScope::system).
            scope: ResourceScope::system(),
            write_lock: tokio::sync::Mutex::new(()),
        }
    }
}

impl<F> FilesystemUserDirectoryStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn load(&self) -> Result<DirectoryDocument, UserDirectoryError> {
        let path = directory_path()?;
        match self.filesystem.get(&self.scope, &path).await {
            Ok(Some(versioned)) => serde_json::from_slice(&versioned.entry.body)
                .map_err(|error| UserDirectoryError::Backend(error.to_string())),
            Ok(None) => Ok(DirectoryDocument::default()),
            Err(error) => Err(UserDirectoryError::Backend(error.to_string())),
        }
    }

    async fn store(&self, document: &DirectoryDocument) -> Result<(), UserDirectoryError> {
        let path = directory_path()?;
        let body = serde_json::to_vec(document)
            .map_err(|error| UserDirectoryError::Backend(error.to_string()))?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        // The write_lock serializes read-modify-write, so an unconditional
        // write is safe for the single-process serve.
        self.filesystem
            .put(&self.scope, &path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(|error| UserDirectoryError::Backend(error.to_string()))
    }
}

#[async_trait]
impl<F> UserDirectoryStore for FilesystemUserDirectoryStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn get(
        &self,
        user_id: &UserId,
    ) -> Result<Option<UserDirectoryRecord>, UserDirectoryError> {
        Ok(self.load().await?.users.get(user_id.as_str()).cloned())
    }

    async fn list(&self) -> Result<Vec<UserDirectoryRecord>, UserDirectoryError> {
        Ok(self.load().await?.users.into_values().collect())
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<UserDirectoryRecord>, UserDirectoryError> {
        Ok(self
            .load()
            .await?
            .users
            .into_values()
            .find(|record| record.token_hash.as_deref() == Some(token_hash)))
    }

    async fn insert(&self, record: UserDirectoryRecord) -> Result<(), UserDirectoryError> {
        let _guard = self.write_lock.lock().await;
        let mut document = self.load().await?;
        let key = record.user_id.as_str().to_string();
        if document.users.contains_key(&key) {
            return Err(UserDirectoryError::AlreadyExists(key));
        }
        document.users.insert(key, record);
        self.store(&document).await
    }

    async fn set_role(
        &self,
        user_id: &UserId,
        role: ironclaw_product_workflow::UserRole,
    ) -> Result<UserDirectoryRecord, UserDirectoryError> {
        let _guard = self.write_lock.lock().await;
        let mut document = self.load().await?;
        let record = document
            .users
            .get_mut(user_id.as_str())
            .ok_or_else(|| UserDirectoryError::NotFound(user_id.as_str().to_string()))?;
        record.role = role;
        record.updated_at = Utc::now();
        let updated = record.clone();
        self.store(&document).await?;
        Ok(updated)
    }

    async fn set_capability(
        &self,
        user_id: &UserId,
        capability_id: &str,
        availability: ironclaw_product_workflow::CapabilityAvailability,
    ) -> Result<(), UserDirectoryError> {
        let _guard = self.write_lock.lock().await;
        let mut document = self.load().await?;
        let record = document
            .users
            .get_mut(user_id.as_str())
            .ok_or_else(|| UserDirectoryError::NotFound(user_id.as_str().to_string()))?;
        record
            .grants
            .insert(capability_id.to_string(), availability);
        record.updated_at = Utc::now();
        self.store(&document).await
    }

    async fn delete(&self, user_id: &UserId) -> Result<(), UserDirectoryError> {
        let _guard = self.write_lock.lock().await;
        let mut document = self.load().await?;
        if document.users.remove(user_id.as_str()).is_none() {
            return Err(UserDirectoryError::NotFound(user_id.as_str().to_string()));
        }
        self.store(&document).await
    }
}

/// Resolves each turn's offered capability surface from the user directory:
/// owner/admin keep the full surface (`All`); a member is narrowed to their
/// essential baseline plus `available` grants.
pub(crate) struct UserCapabilitySurfaceResolver {
    directory: Arc<dyn UserDirectoryStore>,
    owner_user_id: UserId,
}

impl UserCapabilitySurfaceResolver {
    pub(crate) fn new(directory: Arc<dyn UserDirectoryStore>, owner_user_id: UserId) -> Self {
        Self {
            directory,
            owner_user_id,
        }
    }
}

fn allowlist_from_capability_ids(ids: BTreeSet<String>) -> CapabilityAllowSet {
    let caps: BTreeSet<CapabilityId> = ids
        .into_iter()
        // A granted id that is not a structurally-valid capability id simply
        // does not enter the allow-set (it could never be offered anyway).
        .filter_map(|id| CapabilityId::new(id).ok())
        .collect();
    CapabilityAllowSet::Allowlist(caps)
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for UserCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        // Turns with no actor (internal/system) are not policy-restricted.
        let Some(actor) = run_context.actor() else {
            return Ok(CapabilityAllowSet::All);
        };
        // THE owner keeps the full surface and is never a directory row.
        if actor.user_id == self.owner_user_id {
            return Ok(CapabilityAllowSet::All);
        }
        let record = self
            .directory
            .get(&actor.user_id)
            .await
            .map_err(|error| CapabilityResolveError::unavailable(error.to_string()))?;
        match record {
            Some(record) if record.role.is_admin() => Ok(CapabilityAllowSet::All),
            Some(record) => Ok(allowlist_from_capability_ids(
                member_allowed_capability_ids(&record.grants),
            )),
            // Authenticated but unknown principal: fail closed to the essential
            // baseline (a directory authenticator never produces this).
            None => Ok(allowlist_from_capability_ids(
                member_allowed_capability_ids(&BTreeMap::new()),
            )),
        }
    }
}

/// Authenticates the env owner OR a directory user. The env authenticator is
/// tried first (it owns THE owner's bearer); any other bearer is matched
/// against the directory by `sha256(token)`. Admins and the owner carry the
/// operator capability so they reach the admin command surface; members do not.
pub struct DirectoryAuthenticator {
    env: Arc<dyn WebuiAuthenticator>,
    directory: Arc<dyn UserDirectoryStore>,
}

impl DirectoryAuthenticator {
    pub fn new(env: Arc<dyn WebuiAuthenticator>, directory: Arc<dyn UserDirectoryStore>) -> Self {
        Self { env, directory }
    }
}

#[async_trait]
impl WebuiAuthenticator for DirectoryAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        // Inner authenticator (env owner bearer + SSO session) is tried first.
        if let Some(authentication) = self.env.authenticate(token).await {
            // The env owner is already operator. An SSO/session user is `::user`
            // even when their directory role is admin — the session layer has no
            // notion of the capability-policy role. Derive operator FROM THE ROLE
            // here so an SSO admin reaches the operator command plane exactly like
            // a token admin. The directory IS the real admin authorization
            // boundary, so this is auth-method-agnostic, not an SSO bypass.
            if authentication.capabilities.operator_webui_config {
                return Some(authentication);
            }
            if let Ok(Some(record)) = self.directory.get(&authentication.user_id).await
                && record.role.is_admin()
            {
                return Some(WebuiAuthentication::operator(record.user_id));
            }
            return Some(authentication);
        }
        // No inner match: a directory-token user's minted login bearer, matched
        // by sha256(token). (SSO users are token-less, so they never reach here.)
        let token_hash = hash_login_token(token);
        let record = self
            .directory
            .find_by_token_hash(&token_hash)
            .await
            .ok()
            .flatten()?;
        if record.role.is_admin() {
            Some(WebuiAuthentication::operator(record.user_id))
        } else {
            Some(WebuiAuthentication::user(record.user_id))
        }
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        // The directory carries a real admin/owner authorization boundary, so
        // the operator-gated admin routes are mounted; per-request authority is
        // still re-checked from the matched token's capabilities + role.
        true
    }
}

/// Gates extension discovery/install/activation by the member's grants (#5385):
/// a member may only engage with an extension they hold at least one capability
/// for. Owner/admin: every extension. Backs the extension-lifecycle handler's
/// `ExtensionActivationAuthorizer` so a member's attempt to use an ungranted
/// extension returns a clean, model-visible denial instead of dead-ending on a
/// credential gate or an unproductive loop.
pub(crate) struct DirectoryExtensionActivationAuthorizer {
    directory: Arc<dyn UserDirectoryStore>,
    owner_user_id: UserId,
}

impl DirectoryExtensionActivationAuthorizer {
    pub(crate) fn new(directory: Arc<dyn UserDirectoryStore>, owner_user_id: UserId) -> Self {
        Self {
            directory,
            owner_user_id,
        }
    }
}

#[async_trait]
impl ExtensionActivationAuthorizer for DirectoryExtensionActivationAuthorizer {
    async fn may_use_extension(&self, user_id: &UserId, extension_id: &str) -> bool {
        // THE owner keeps every extension and is never a directory row.
        if *user_id == self.owner_user_id {
            return true;
        }
        match self.directory.get(user_id).await {
            // Admins are unrestricted.
            Ok(Some(record)) if record.role.is_admin() => true,
            // A member may use the extension iff their allow-set holds a
            // capability under that extension's namespace (`<extension_id>.*`).
            Ok(Some(record)) => {
                let namespace = format!("{extension_id}.");
                member_allowed_capability_ids(&record.grants)
                    .iter()
                    .any(|capability| {
                        capability == extension_id || capability.starts_with(&namespace)
                    })
            }
            // Unknown principal or a backend read error: fail closed (deny) —
            // a directory authenticator never produces an unknown principal.
            Ok(None) | Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use ironclaw_product_workflow::{CapabilityAvailability, UserRole};

    use super::*;

    struct StubDirectory {
        records: HashMap<String, UserDirectoryRecord>,
    }

    #[async_trait]
    impl UserDirectoryStore for StubDirectory {
        async fn get(
            &self,
            user_id: &UserId,
        ) -> Result<Option<UserDirectoryRecord>, UserDirectoryError> {
            Ok(self.records.get(user_id.as_str()).cloned())
        }
        async fn list(&self) -> Result<Vec<UserDirectoryRecord>, UserDirectoryError> {
            Ok(self.records.values().cloned().collect())
        }
        async fn find_by_token_hash(
            &self,
            _token_hash: &str,
        ) -> Result<Option<UserDirectoryRecord>, UserDirectoryError> {
            Ok(None)
        }
        async fn insert(&self, _record: UserDirectoryRecord) -> Result<(), UserDirectoryError> {
            Ok(())
        }
        async fn set_role(
            &self,
            _user_id: &UserId,
            _role: UserRole,
        ) -> Result<UserDirectoryRecord, UserDirectoryError> {
            Err(UserDirectoryError::Backend("stub".to_string()))
        }
        async fn set_capability(
            &self,
            _user_id: &UserId,
            _capability_id: &str,
            _availability: CapabilityAvailability,
        ) -> Result<(), UserDirectoryError> {
            Ok(())
        }
        async fn delete(&self, _user_id: &UserId) -> Result<(), UserDirectoryError> {
            Ok(())
        }
    }

    fn record(
        user_id: &str,
        role: UserRole,
        grants: &[(&str, CapabilityAvailability)],
    ) -> UserDirectoryRecord {
        let now = Utc::now();
        UserDirectoryRecord {
            user_id: UserId::new(user_id).expect("valid test user id"),
            role,
            token_hash: Some("hash".to_string()),
            grants: grants
                .iter()
                .map(|(cap, availability)| ((*cap).to_string(), *availability))
                .collect(),
            created_at: now,
            updated_at: now,
        }
    }

    fn uid(value: &str) -> UserId {
        UserId::new(value).expect("valid test user id")
    }

    #[tokio::test]
    async fn extension_activation_is_gated_by_member_grants() {
        let owner = uid("director");
        let mut records = HashMap::new();
        records.insert("carl".to_string(), record("carl", UserRole::Member, &[]));
        records.insert(
            "bob".to_string(),
            record(
                "bob",
                UserRole::Member,
                &[("google-drive.list_files", CapabilityAvailability::Available)],
            ),
        );
        records.insert(
            "officer".to_string(),
            record("officer", UserRole::Admin, &[]),
        );
        let directory: Arc<dyn UserDirectoryStore> = Arc::new(StubDirectory { records });
        let authorizer = DirectoryExtensionActivationAuthorizer::new(directory, owner.clone());

        // THE owner may use any extension.
        assert!(authorizer.may_use_extension(&owner, "google-drive").await);
        assert!(authorizer.may_use_extension(&owner, "web-access").await);

        // An admin may use any extension.
        assert!(
            authorizer
                .may_use_extension(&uid("officer"), "google-drive")
                .await
        );

        // bob (granted google-drive.*) may use google-drive — but not web-access.
        assert!(
            authorizer
                .may_use_extension(&uid("bob"), "google-drive")
                .await
        );
        assert!(
            !authorizer
                .may_use_extension(&uid("bob"), "web-access")
                .await
        );

        // carl (no grants) may use neither — the reported bug.
        assert!(
            !authorizer
                .may_use_extension(&uid("carl"), "google-drive")
                .await
        );
        assert!(
            !authorizer
                .may_use_extension(&uid("carl"), "web-access")
                .await
        );

        // Unknown principal: fail closed.
        assert!(
            !authorizer
                .may_use_extension(&uid("ghost"), "google-drive")
                .await
        );
    }
}
