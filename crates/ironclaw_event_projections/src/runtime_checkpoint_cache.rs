use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use ironclaw_events::EventCursor;

use crate::ProjectionScope;
use crate::runtime_projection::RuntimeProjectionState;

const RUNTIME_PROJECTION_CHECKPOINTS_PER_SCOPE: usize = 256;
const RUNTIME_PROJECTION_CHECKPOINT_SCOPES: usize = 256;

#[derive(Clone)]
pub(crate) struct RuntimeProjectionCheckpoint {
    pub(crate) cursor: EventCursor,
    pub(crate) state: RuntimeProjectionState,
}

#[derive(Default)]
struct RuntimeProjectionCheckpointMap {
    scopes: HashMap<ProjectionScope, BTreeMap<EventCursor, RuntimeProjectionState>>,
    scope_write_order: VecDeque<ProjectionScope>,
}

impl RuntimeProjectionCheckpointMap {
    fn touch_scope(&mut self, scope: &ProjectionScope) {
        self.scope_write_order
            .retain(|candidate| candidate != scope);
        self.scope_write_order.push_back(scope.clone());
    }

    fn evict_old_scopes(&mut self) {
        while self.scopes.len() > RUNTIME_PROJECTION_CHECKPOINT_SCOPES {
            let Some(candidate) = self.scope_write_order.pop_front() else {
                break;
            };
            self.scopes.remove(&candidate);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct RuntimeProjectionCheckpointCache {
    checkpoints: Arc<Mutex<RuntimeProjectionCheckpointMap>>,
}

impl RuntimeProjectionCheckpointCache {
    pub(crate) fn latest(&self, scope: &ProjectionScope) -> RuntimeProjectionCheckpoint {
        let checkpoints = self.lock();
        checkpoints
            .scopes
            .get(scope)
            .and_then(|scope_checkpoints| {
                scope_checkpoints
                    .last_key_value()
                    .map(|(cursor, state)| checkpoint(*cursor, state.clone()))
            })
            .unwrap_or_else(origin_runtime_checkpoint)
    }

    pub(crate) fn at_or_before(
        &self,
        scope: &ProjectionScope,
        cursor: EventCursor,
    ) -> RuntimeProjectionCheckpoint {
        let checkpoints = self.lock();
        checkpoints
            .scopes
            .get(scope)
            .and_then(|scope_checkpoints| {
                scope_checkpoints
                    .range(..=cursor)
                    .next_back()
                    .map(|(cursor, state)| checkpoint(*cursor, state.clone()))
            })
            .unwrap_or_else(origin_runtime_checkpoint)
    }

    pub(crate) fn store(&self, scope: &ProjectionScope, checkpoint: &RuntimeProjectionCheckpoint) {
        if checkpoint.cursor == EventCursor::origin() {
            return;
        }
        let mut checkpoints = self.lock();
        checkpoints.touch_scope(scope);
        {
            let scope_checkpoints = checkpoints.scopes.entry(scope.clone()).or_default();
            scope_checkpoints.insert(checkpoint.cursor, checkpoint.state.clone());
            while scope_checkpoints.len() > RUNTIME_PROJECTION_CHECKPOINTS_PER_SCOPE {
                let Some(first) = scope_checkpoints.keys().next().copied() else {
                    break;
                };
                scope_checkpoints.remove(&first);
            }
        }
        checkpoints.evict_old_scopes();
    }

    fn lock(&self) -> MutexGuard<'_, RuntimeProjectionCheckpointMap> {
        match self.checkpoints.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(crate) fn after_for_checkpoint(cursor: EventCursor) -> Option<EventCursor> {
    if cursor == EventCursor::origin() {
        None
    } else {
        Some(cursor)
    }
}

fn origin_runtime_checkpoint() -> RuntimeProjectionCheckpoint {
    checkpoint(
        EventCursor::origin(),
        RuntimeProjectionState::without_capability_activity_output_limit(),
    )
}

fn checkpoint(cursor: EventCursor, state: RuntimeProjectionState) -> RuntimeProjectionCheckpoint {
    RuntimeProjectionCheckpoint { cursor, state }
}

#[cfg(test)]
mod tests {
    use ironclaw_events::{EventStreamKey, ReadScope};
    use ironclaw_host_api::{AgentId, TenantId, UserId};

    use super::*;

    fn scope(index: usize) -> ProjectionScope {
        ProjectionScope {
            stream: EventStreamKey::new(
                TenantId::new("tenant").expect("tenant"),
                UserId::new(format!("user-{index}")).expect("user"),
                Some(AgentId::new("agent").expect("agent")),
            ),
            read_scope: ReadScope::any(),
        }
    }

    fn stored_checkpoint(cursor: u64) -> RuntimeProjectionCheckpoint {
        RuntimeProjectionCheckpoint {
            cursor: EventCursor::new(cursor),
            state: RuntimeProjectionState::without_capability_activity_output_limit(),
        }
    }

    #[test]
    fn evicts_oldest_checkpoints_within_scope() {
        let cache = RuntimeProjectionCheckpointCache::default();
        let scope = scope(0);

        for cursor in 1..=(RUNTIME_PROJECTION_CHECKPOINTS_PER_SCOPE as u64 + 1) {
            cache.store(&scope, &stored_checkpoint(cursor));
        }

        assert_eq!(cache.latest(&scope).cursor, EventCursor::new(257));
        assert_eq!(
            cache.at_or_before(&scope, EventCursor::new(1)).cursor,
            EventCursor::origin()
        );
        assert_eq!(
            cache.at_or_before(&scope, EventCursor::new(2)).cursor,
            EventCursor::new(2)
        );
    }

    #[test]
    fn evicts_oldest_scope_when_global_scope_cap_is_exceeded() {
        let cache = RuntimeProjectionCheckpointCache::default();
        let first_scope = scope(0);

        for index in 0..=RUNTIME_PROJECTION_CHECKPOINT_SCOPES {
            let scope = scope(index);
            cache.store(&scope, &stored_checkpoint(1));
        }

        assert_eq!(cache.latest(&first_scope).cursor, EventCursor::origin());
        assert_eq!(
            cache
                .latest(&scope(RUNTIME_PROJECTION_CHECKPOINT_SCOPES))
                .cursor,
            EventCursor::new(1)
        );
    }
}
