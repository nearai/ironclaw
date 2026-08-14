//! Profile-based selection between replaceable turn-run executor implementations.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_processes::ProcessTransitionPort;
use ironclaw_turns::{RunProfileId, TurnError, runner::ClaimedTurnRun};

use crate::turn_scheduler::{TurnRunExecutor, TurnRunExecutorError};

/// Routes explicitly registered run profiles to alternate executors and sends
/// every other profile to the canonical default executor.
pub struct ProfileRoutingTurnRunExecutor {
    default_executor: Arc<dyn TurnRunExecutor>,
    executors_by_profile: HashMap<RunProfileId, Arc<dyn TurnRunExecutor>>,
}

impl ProfileRoutingTurnRunExecutor {
    pub fn new(default_executor: Arc<dyn TurnRunExecutor>) -> Self {
        Self {
            default_executor,
            executors_by_profile: HashMap::new(),
        }
    }

    pub fn with_routes(
        mut self,
        profile_ids: impl IntoIterator<Item = String>,
        executor: Arc<dyn TurnRunExecutor>,
    ) -> Result<Self, String> {
        let mut route_count = 0_usize;
        for raw_profile_id in profile_ids {
            let profile_id = RunProfileId::new(&raw_profile_id).map_err(|error| {
                format!("turn executor route profile id `{raw_profile_id}` is invalid: {error}")
            })?;
            if self
                .executors_by_profile
                .insert(profile_id.clone(), Arc::clone(&executor))
                .is_some()
            {
                return Err(format!(
                    "turn executor route for profile `{raw_profile_id}` is duplicated"
                ));
            }
            route_count = route_count.saturating_add(1);
        }
        if route_count == 0 {
            return Err("turn executor route requires at least one run profile".to_string());
        }
        Ok(self)
    }

    fn executor_for_profile(&self, profile_id: &RunProfileId) -> Arc<dyn TurnRunExecutor> {
        self.executors_by_profile
            .get(profile_id)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&self.default_executor))
    }
}

#[async_trait]
impl TurnRunExecutor for ProfileRoutingTurnRunExecutor {
    async fn execute_claimed_run(
        &self,
        claimed: ClaimedTurnRun,
        process_transitions: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
    ) -> Result<(), TurnRunExecutorError> {
        self.executor_for_profile(&claimed.resolved_run_profile.profile_id)
            .execute_claimed_run(claimed, process_transitions)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnusedExecutor;

    #[async_trait]
    impl TurnRunExecutor for UnusedExecutor {
        async fn execute_claimed_run(
            &self,
            _claimed: ClaimedTurnRun,
            _process_transitions: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
        ) -> Result<(), TurnRunExecutorError> {
            unreachable!("selection tests do not execute a turn")
        }
    }

    #[test]
    fn selected_profile_uses_registered_executor_and_other_profiles_use_default() {
        let default: Arc<dyn TurnRunExecutor> = Arc::new(UnusedExecutor);
        let alternate: Arc<dyn TurnRunExecutor> = Arc::new(UnusedExecutor);
        let router = ProfileRoutingTurnRunExecutor::new(Arc::clone(&default))
            .with_routes(
                ["assistant".to_string(), "coding".to_string()],
                Arc::clone(&alternate),
            )
            .expect("valid routes");

        assert!(Arc::ptr_eq(
            &router.executor_for_profile(&RunProfileId::new("assistant").expect("profile id")),
            &alternate
        ));
        assert!(Arc::ptr_eq(
            &router.executor_for_profile(&RunProfileId::new("automation").expect("profile id")),
            &default
        ));
    }

    #[test]
    fn duplicate_profile_routes_fail_closed() {
        let default: Arc<dyn TurnRunExecutor> = Arc::new(UnusedExecutor);
        let alternate: Arc<dyn TurnRunExecutor> = Arc::new(UnusedExecutor);
        let error = ProfileRoutingTurnRunExecutor::new(default)
            .with_routes(["coding".to_string(), "coding".to_string()], alternate)
            .err()
            .expect("duplicate route must fail");

        assert!(error.contains("duplicated"));
    }
}
