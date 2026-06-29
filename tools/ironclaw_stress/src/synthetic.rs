use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::TurnScope;

use crate::Args;

pub(crate) struct SyntheticIds {
    tenants: Vec<TenantId>,
    users: Vec<UserId>,
    agent_id: AgentId,
    project_id: ProjectId,
}

pub(crate) struct UserTurnContext {
    pub(crate) user_id: UserId,
    pub(crate) thread_id: ThreadId,
    pub(crate) thread_scope: ThreadScope,
    pub(crate) turn_scope: TurnScope,
}

impl SyntheticIds {
    pub(crate) fn new(args: &Args) -> Result<Self, String> {
        let tenants = (0..args.tenants)
            .map(|tenant_index| {
                TenantId::new(format!("tenant-{tenant_index:04}"))
                    .map_err(|error| format!("build synthetic tenant id: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let users = (0..args.users)
            .map(|user_index| {
                UserId::new(format!("user-{user_index:06}"))
                    .map_err(|error| format!("build synthetic user id: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let agent_id = AgentId::new("ironclaw-stress")
            .map_err(|error| format!("build synthetic agent id: {error}"))?;
        let project_id = ProjectId::new("ironclaw-stress")
            .map_err(|error| format!("build synthetic project id: {error}"))?;
        Ok(Self {
            tenants,
            users,
            agent_id,
            project_id,
        })
    }

    #[cfg(test)]
    pub(crate) fn tenant_count(&self) -> usize {
        self.tenants.len()
    }

    #[cfg(test)]
    pub(crate) fn user_count(&self) -> usize {
        self.users.len()
    }

    pub(crate) fn scope(
        &self,
        args: &Args,
        worker_index: usize,
        operation_index: usize,
    ) -> ResourceScope {
        let (tenant_index, user_index, _) =
            self.synthetic_indexes(args, worker_index, operation_index);
        ResourceScope {
            tenant_id: self.tenants[tenant_index].clone(),
            user_id: self.users[user_index].clone(),
            agent_id: Some(self.agent_id.clone()),
            project_id: Some(self.project_id.clone()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    pub(crate) fn user_turn_context(
        &self,
        args: &Args,
        worker_index: usize,
        operation_index: usize,
    ) -> Result<UserTurnContext, String> {
        let (tenant_index, user_index, _) =
            self.synthetic_indexes(args, worker_index, operation_index);
        let tenant_id = self.tenants[tenant_index].clone();
        let user_id = self.users[user_index].clone();
        let thread_id = ThreadId::new(format!("thread-{tenant_index:04}-{user_index:06}"))
            .map_err(|error| error.to_string())?;
        let thread_scope = ThreadScope {
            tenant_id: tenant_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: Some(self.project_id.clone()),
            owner_user_id: Some(user_id.clone()),
            mission_id: None,
        };
        let turn_scope = TurnScope::new_with_owner(
            tenant_id,
            Some(self.agent_id.clone()),
            Some(self.project_id.clone()),
            thread_id.clone(),
            Some(user_id.clone()),
        );
        Ok(UserTurnContext {
            user_id,
            thread_id,
            thread_scope,
            turn_scope,
        })
    }

    fn synthetic_indexes(
        &self,
        args: &Args,
        worker_index: usize,
        operation_index: usize,
    ) -> (usize, usize, usize) {
        let global_index = if args.uses_duration_mode() {
            operation_index
                .saturating_mul(args.concurrency)
                .saturating_add(worker_index)
        } else {
            worker_index
                .saturating_mul(args.operations)
                .saturating_add(operation_index)
        };
        let user_index = global_index % self.users.len();
        let tenant_index = user_index % self.tenants.len();
        (tenant_index, user_index, global_index)
    }
}
