use std::{path::PathBuf, sync::Arc};

use axum::http::StatusCode;
use sha2::{Digest, Sha256};

use crate::channels::web::auth::UserIdentity;
use crate::channels::web::platform::state::GatewayState;

type HandlerResult<T> = Result<T, (StatusCode, String)>;
type RegistryResult<T> = Result<T, ironclaw_skills::SkillRegistryError>;

pub(super) enum ScopedSkillRegistry {
    Shared(Arc<std::sync::RwLock<ironclaw_skills::SkillRegistry>>),
    User(ironclaw_skills::SkillRegistry),
}

pub(super) async fn scoped_skill_registry(
    state: &GatewayState,
    user: &UserIdentity,
) -> HandlerResult<ScopedSkillRegistry> {
    let registry = Arc::clone(state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?);

    if !state.multi_tenant_mode || user.user_id == state.owner_id {
        return Ok(ScopedSkillRegistry::Shared(registry));
    }

    let mut scoped = {
        let guard = registry.read().map_err(lock_error_response)?;
        scoped_registry_from_template(&guard, user)
    };
    scoped.discover_all().await;
    Ok(ScopedSkillRegistry::User(scoped))
}

impl ScopedSkillRegistry {
    pub(super) fn skills_snapshot(
        &self,
    ) -> HandlerResult<Vec<ironclaw_skills::types::LoadedSkill>> {
        self.read(|registry| registry.skills().to_vec())
    }

    pub(super) fn has(&self, name: &str) -> HandlerResult<bool> {
        self.read(|registry| registry.has(name))
    }

    pub(super) fn install_target_dir(&self) -> HandlerResult<PathBuf> {
        self.read(|registry| registry.install_target_dir().to_path_buf())
    }

    pub(super) fn validate_remove(&self, name: &str) -> HandlerResult<RegistryResult<PathBuf>> {
        self.try_read(|registry| registry.validate_remove(name))
    }

    pub(super) fn validate_update(
        &self,
        name: &str,
    ) -> HandlerResult<
        RegistryResult<(
            PathBuf,
            ironclaw_skills::SkillTrust,
            ironclaw_skills::types::SkillSource,
        )>,
    > {
        self.try_read(|registry| registry.validate_update(name))
    }

    pub(super) fn commit_install(
        &mut self,
        name: &str,
        skill: ironclaw_skills::types::LoadedSkill,
    ) -> HandlerResult<RegistryResult<()>> {
        self.try_write(|registry| registry.commit_install(name, skill))
    }

    pub(super) fn commit_remove(&mut self, name: &str) -> HandlerResult<RegistryResult<()>> {
        self.try_write(|registry| registry.commit_remove(name))
    }

    pub(super) fn commit_update(
        &mut self,
        name: &str,
        skill: ironclaw_skills::types::LoadedSkill,
    ) -> HandlerResult<RegistryResult<()>> {
        self.try_write(|registry| registry.commit_update(name, skill))
    }

    fn read<T>(
        &self,
        operation: impl FnOnce(&ironclaw_skills::SkillRegistry) -> T,
    ) -> HandlerResult<T> {
        match self {
            Self::Shared(registry) => {
                let guard = registry.read().map_err(lock_error_response)?;
                Ok(operation(&guard))
            }
            Self::User(registry) => Ok(operation(registry)),
        }
    }

    fn try_read<T>(
        &self,
        operation: impl FnOnce(&ironclaw_skills::SkillRegistry) -> RegistryResult<T>,
    ) -> HandlerResult<RegistryResult<T>> {
        match self {
            Self::Shared(registry) => {
                let guard = registry.read().map_err(lock_error_response)?;
                Ok(operation(&guard))
            }
            Self::User(registry) => Ok(operation(registry)),
        }
    }

    fn try_write<T>(
        &mut self,
        operation: impl FnOnce(&mut ironclaw_skills::SkillRegistry) -> RegistryResult<T>,
    ) -> HandlerResult<RegistryResult<T>> {
        match self {
            Self::Shared(registry) => {
                let mut guard = registry.write().map_err(lock_error_response)?;
                Ok(operation(&mut guard))
            }
            Self::User(registry) => Ok(operation(registry)),
        }
    }
}

fn scoped_registry_from_template(
    template: &ironclaw_skills::SkillRegistry,
    user: &UserIdentity,
) -> ironclaw_skills::SkillRegistry {
    let segment = scoped_user_skills_segment(&user.user_id);
    let user_root = template
        .user_dir()
        .parent()
        .unwrap_or_else(|| template.user_dir())
        .join("users")
        .join(&segment);
    let user_dir = user_root.join("skills");
    let installed_dir = Some(user_root.join("installed_skills"));
    template.clone_config_for_user_dirs(user_dir, installed_dir)
}

fn scoped_user_skills_segment(user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn lock_error_response(e: std::sync::PoisonError<impl Sized>) -> (StatusCode, String) {
    tracing::error!("Skill registry lock poisoned: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Can't access skills right now".to_string(),
    )
}
