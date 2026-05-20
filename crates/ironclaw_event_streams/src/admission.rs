use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_event_projections::ProjectionScope;
use ironclaw_host_api::TenantId;
use ironclaw_turns::TurnActor;
use serde::{Deserialize, Serialize};

use crate::{
    error::ProjectionStreamError,
    keys::{ScopeAdmissionKey, scope_key},
    types::{ProjectionTarget, ProjectionViewClass},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionAccessRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
}

#[async_trait]
pub trait ProjectionAccessPolicy: Send + Sync {
    async fn authorize(
        &self,
        request: ProjectionAccessRequest,
    ) -> Result<(), ProjectionStreamError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionStreamAdmissionRequest {
    pub actor: TurnActor,
    pub tenant_id: TenantId,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
}

#[async_trait]
pub trait ProjectionStreamAdmissionPolicy: Send + Sync {
    async fn admit(
        &self,
        request: ProjectionStreamAdmissionRequest,
    ) -> Result<ProjectionStreamAdmissionPermit, ProjectionStreamError>;
}

pub struct ProjectionStreamAdmissionPermit {
    release: Option<AdmissionRelease>,
}

impl ProjectionStreamAdmissionPermit {
    pub fn detached() -> Self {
        Self { release: None }
    }
}

impl Drop for ProjectionStreamAdmissionPermit {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release.release();
        }
    }
}

struct AdmissionRelease {
    state: Arc<Mutex<AdmissionState>>,
    tenant_key: TenantAdmissionKey,
    actor_key: ActorAdmissionKey,
    scope_key: ScopeAdmissionKey,
}

impl AdmissionRelease {
    fn release(self) {
        if let Ok(mut state) = self.state.lock() {
            decrement(&mut state.by_tenant, &self.tenant_key);
            decrement(&mut state.by_actor, &self.actor_key);
            decrement(&mut state.by_scope, &self.scope_key);
            state.global = state.global.saturating_sub(1);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionStreamLimits {
    pub per_tenant: usize,
    pub per_actor: usize,
    pub per_scope: usize,
    pub global: usize,
}

impl Default for ProjectionStreamLimits {
    fn default() -> Self {
        Self {
            per_tenant: 64,
            per_actor: 16,
            per_scope: 8,
            global: 512,
        }
    }
}

#[derive(Default)]
pub struct InMemoryProjectionStreamAdmissionPolicy {
    limits: ProjectionStreamLimits,
    state: Arc<Mutex<AdmissionState>>,
}

#[derive(Default)]
struct AdmissionState {
    global: usize,
    by_tenant: HashMap<TenantAdmissionKey, usize>,
    by_actor: HashMap<ActorAdmissionKey, usize>,
    by_scope: HashMap<ScopeAdmissionKey, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TenantAdmissionKey(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActorAdmissionKey {
    tenant_id: String,
    user_id: String,
}

impl InMemoryProjectionStreamAdmissionPolicy {
    pub fn new(limits: ProjectionStreamLimits) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(AdmissionState::default())),
        }
    }
}

#[async_trait]
impl ProjectionStreamAdmissionPolicy for InMemoryProjectionStreamAdmissionPolicy {
    async fn admit(
        &self,
        request: ProjectionStreamAdmissionRequest,
    ) -> Result<ProjectionStreamAdmissionPermit, ProjectionStreamError> {
        let tenant_key = TenantAdmissionKey(request.tenant_id.to_string());
        let actor_key = ActorAdmissionKey {
            tenant_id: request.tenant_id.to_string(),
            user_id: request.actor.user_id.to_string(),
        };
        let scope_key = scope_key(&request.scope, &request.target);
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProjectionStreamError::Source)?;
        if state.global >= self.limits.global
            || count(&state.by_tenant, &tenant_key) >= self.limits.per_tenant
            || count(&state.by_actor, &actor_key) >= self.limits.per_actor
            || count(&state.by_scope, &scope_key) >= self.limits.per_scope
        {
            return Err(ProjectionStreamError::AdmissionDenied);
        }
        state.global += 1;
        increment(&mut state.by_tenant, &tenant_key);
        increment(&mut state.by_actor, &actor_key);
        increment(&mut state.by_scope, &scope_key);
        Ok(ProjectionStreamAdmissionPermit {
            release: Some(AdmissionRelease {
                state: Arc::clone(&self.state),
                tenant_key,
                actor_key,
                scope_key,
            }),
        })
    }
}

#[derive(Default)]
pub struct AllowAllProjectionAccessPolicy;

#[async_trait]
impl ProjectionAccessPolicy for AllowAllProjectionAccessPolicy {
    async fn authorize(
        &self,
        _request: ProjectionAccessRequest,
    ) -> Result<(), ProjectionStreamError> {
        Ok(())
    }
}

fn count<K>(map: &HashMap<K, usize>, key: &K) -> usize
where
    K: Eq + Hash,
{
    map.get(key).copied().unwrap_or(0)
}

fn increment<K>(map: &mut HashMap<K, usize>, key: &K)
where
    K: Clone + Eq + Hash,
{
    *map.entry(key.clone()).or_insert(0) += 1;
}

fn decrement<K>(map: &mut HashMap<K, usize>, key: &K)
where
    K: Eq + Hash,
{
    if let Some(value) = map.get_mut(key) {
        *value = value.saturating_sub(1);
        if *value == 0 {
            map.remove(key);
        }
    }
}
