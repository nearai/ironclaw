use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use crate::channels::web::auth::UserIdentity;
use crate::channels::web::platform::state::GatewayState;
use axum::http::StatusCode;

type HandlerResult<T> = Result<T, (StatusCode, String)>;
type RegistryResult<T> = Result<T, ironclaw_skills::SkillRegistryError>;
type SharedSkillRegistry = Arc<RwLock<ironclaw_skills::SkillRegistry>>;
type ScopedRegistryBuildLock = Arc<tokio::sync::Mutex<()>>;

const SCOPED_REGISTRY_CACHE_TTL: Duration = Duration::from_secs(30);
const SCOPED_REGISTRY_CACHE_MAX_ENTRIES: usize = 1024;

static SCOPED_SKILL_REGISTRIES: std::sync::LazyLock<
    std::sync::Mutex<HashMap<ScopedSkillRegistryCacheKey, CachedScopedSkillRegistry>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
static SCOPED_SKILL_REGISTRY_BUILD_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<HashMap<ScopedSkillRegistryCacheKey, std::sync::Weak<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ScopedSkillRegistryCacheKey {
    template_ptr: usize,
    tenant_segment: String,
    user_segment: String,
}

struct CachedScopedSkillRegistry {
    registry: SharedSkillRegistry,
    discovered_at: Instant,
}

pub(super) enum ScopedSkillRegistry {
    Shared(SharedSkillRegistry),
    User(SharedSkillRegistry),
}

pub(super) async fn scoped_skill_registry(
    state: &GatewayState,
    user: &UserIdentity,
) -> HandlerResult<ScopedSkillRegistry> {
    let registry = Arc::clone(state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?);

    if !state.multi_tenant_mode {
        return Ok(ScopedSkillRegistry::Shared(registry));
    }

    let tenant_segment = state.owner_id.clone();
    Ok(ScopedSkillRegistry::User(
        cached_scoped_registry(&registry, &tenant_segment, user).await?,
    ))
}

async fn cached_scoped_registry(
    template: &SharedSkillRegistry,
    tenant_segment: &str,
    user: &UserIdentity,
) -> HandlerResult<SharedSkillRegistry> {
    let user_segment =
        ironclaw_skills::SkillRegistry::tenant_user_scope_segment(tenant_segment, &user.user_id);
    let key = ScopedSkillRegistryCacheKey {
        template_ptr: Arc::as_ptr(template) as usize,
        tenant_segment: tenant_segment.to_string(),
        user_segment,
    };
    let now = Instant::now();
    if let Some(registry) = cached_registry(&key, now)? {
        return Ok(registry);
    }

    let build_lock = scoped_registry_build_lock(&key)?;
    let _build_guard = build_lock.lock().await;
    let now = Instant::now();
    if let Some(registry) = cached_registry(&key, now)? {
        return Ok(registry);
    }

    let mut scoped = {
        let guard = template.read().map_err(lock_error_response)?;
        scoped_registry_from_template(&guard, tenant_segment, &user.user_id)
    };
    scoped.discover_all().await;
    let scoped = Arc::new(RwLock::new(scoped));
    cache_registry(key, Arc::clone(&scoped), now)?;
    Ok(scoped)
}

fn cached_registry(
    key: &ScopedSkillRegistryCacheKey,
    now: Instant,
) -> HandlerResult<Option<SharedSkillRegistry>> {
    let mut cache = SCOPED_SKILL_REGISTRIES.lock().map_err(|error| {
        tracing::error!("Scoped skill registry cache lock poisoned: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't access skills right now".to_string(),
        )
    })?;
    if let Some(cached) = cache.get(key)
        && now.duration_since(cached.discovered_at) < SCOPED_REGISTRY_CACHE_TTL
    {
        return Ok(Some(Arc::clone(&cached.registry)));
    }
    cache.remove(key);
    Ok(None)
}

fn cache_registry(
    key: ScopedSkillRegistryCacheKey,
    registry: SharedSkillRegistry,
    now: Instant,
) -> HandlerResult<()> {
    let mut cache = SCOPED_SKILL_REGISTRIES.lock().map_err(|error| {
        tracing::error!("Scoped skill registry cache lock poisoned: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't access skills right now".to_string(),
        )
    })?;
    prune_scoped_registry_cache(&mut cache, now);
    cache.insert(
        key,
        CachedScopedSkillRegistry {
            registry,
            discovered_at: now,
        },
    );
    Ok(())
}

fn prune_scoped_registry_cache(
    cache: &mut HashMap<ScopedSkillRegistryCacheKey, CachedScopedSkillRegistry>,
    now: Instant,
) {
    cache.retain(|_, cached| now.duration_since(cached.discovered_at) < SCOPED_REGISTRY_CACHE_TTL);
    while cache.len() >= SCOPED_REGISTRY_CACHE_MAX_ENTRIES {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, cached)| cached.discovered_at)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

fn scoped_registry_build_lock(
    key: &ScopedSkillRegistryCacheKey,
) -> HandlerResult<ScopedRegistryBuildLock> {
    let mut locks = SCOPED_SKILL_REGISTRY_BUILD_LOCKS.lock().map_err(|error| {
        tracing::error!("Scoped skill registry build lock map poisoned: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't access skills right now".to_string(),
        )
    })?;
    locks.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = locks.get(key).and_then(std::sync::Weak::upgrade) {
        return Ok(existing);
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key.clone(), Arc::downgrade(&lock));
    Ok(lock)
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
            Self::User(registry) => {
                let guard = registry.read().map_err(lock_error_response)?;
                Ok(operation(&guard))
            }
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
            Self::User(registry) => {
                let guard = registry.read().map_err(lock_error_response)?;
                Ok(operation(&guard))
            }
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
            Self::User(registry) => {
                let mut guard = registry.write().map_err(lock_error_response)?;
                Ok(operation(&mut guard))
            }
        }
    }
}

fn scoped_registry_from_template(
    template: &ironclaw_skills::SkillRegistry,
    tenant_id: &str,
    user_id: &str,
) -> ironclaw_skills::SkillRegistry {
    template.clone_config_for_tenant_user_scope(tenant_id, user_id)
}

fn lock_error_response(e: std::sync::PoisonError<impl Sized>) -> (StatusCode, String) {
    tracing::error!("Skill registry lock poisoned: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Can't access skills right now".to_string(),
    )
}
