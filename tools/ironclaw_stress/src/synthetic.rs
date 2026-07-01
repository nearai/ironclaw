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
    pub(crate) thread_owner_user_id: UserId,
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
        let actor_user_index = partitioned_worker_index(
            self.users.len(),
            args.concurrency,
            worker_index,
            operation_index,
        );
        let thread_user_index =
            self.thread_user_index(args, actor_user_index, worker_index, operation_index);
        // Spread one owner's concurrent load across `threads_per_owner` distinct
        // threads that all share that owner's single `/turns/state.json`. This
        // is what makes the filesystem turn-state CAS actually contend
        // cross-thread (the production shape); with the default 1, behavior is
        // unchanged (one thread per owner).
        let thread_slot = if args.threads_per_owner > 1 {
            Some(
                worker_index.wrapping_mul(31).wrapping_add(operation_index)
                    % args.threads_per_owner,
            )
        } else {
            None
        };
        self.user_turn_context_for_indexes(actor_user_index, thread_user_index, thread_slot)
    }

    pub(crate) fn user_turn_context_for_user_index(
        &self,
        user_index: usize,
    ) -> Result<UserTurnContext, String> {
        self.user_turn_context_for_indexes(user_index, user_index, None)
    }

    fn user_turn_context_for_indexes(
        &self,
        actor_user_index: usize,
        thread_user_index: usize,
        thread_slot: Option<usize>,
    ) -> Result<UserTurnContext, String> {
        if actor_user_index >= self.users.len() {
            return Err(format!(
                "synthetic actor user index {actor_user_index} out of range for {} users",
                self.users.len()
            ));
        }
        if thread_user_index >= self.users.len() {
            return Err(format!(
                "synthetic thread user index {thread_user_index} out of range for {} users",
                self.users.len()
            ));
        }
        let tenant_index = thread_user_index % self.tenants.len();
        let tenant_id = self.tenants[tenant_index].clone();
        let user_id = self.users[actor_user_index].clone();
        let thread_owner_user_id = self.users[thread_user_index].clone();
        // The owner (and thus the per-user `/turns/state.json`) is keyed by
        // `thread_user_index`; the optional slot adds a distinct thread *under*
        // that same owner so multiple threads share — and contend on — one
        // turn-state document.
        let thread_id = match thread_slot {
            Some(slot) => ThreadId::new(format!(
                "thread-{tenant_index:04}-{thread_user_index:06}-{slot:04}"
            )),
            None => ThreadId::new(format!("thread-{tenant_index:04}-{thread_user_index:06}")),
        }
        .map_err(|error| error.to_string())?;
        let thread_scope = ThreadScope {
            tenant_id: tenant_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: Some(self.project_id.clone()),
            owner_user_id: Some(thread_owner_user_id.clone()),
            mission_id: None,
        };
        let turn_scope = TurnScope::new_with_owner(
            tenant_id,
            Some(self.agent_id.clone()),
            Some(self.project_id.clone()),
            thread_id.clone(),
            Some(thread_owner_user_id.clone()),
        );
        Ok(UserTurnContext {
            user_id,
            thread_owner_user_id,
            thread_id,
            thread_scope,
            turn_scope,
        })
    }

    fn thread_user_index(
        &self,
        args: &Args,
        user_index: usize,
        worker_index: usize,
        operation_index: usize,
    ) -> usize {
        if args.active_thread_count == 0 {
            user_index
        } else {
            partitioned_worker_index(
                args.active_thread_count,
                args.concurrency,
                worker_index,
                operation_index,
            )
        }
    }

    fn synthetic_indexes(
        &self,
        args: &Args,
        worker_index: usize,
        operation_index: usize,
    ) -> (usize, usize, usize) {
        let global_index = operation_index
            .saturating_mul(args.concurrency)
            .saturating_add(worker_index);
        let user_index = global_index % self.users.len();
        let tenant_index = user_index % self.tenants.len();
        (tenant_index, user_index, global_index)
    }
}

fn partitioned_worker_index(
    pool_size: usize,
    worker_count: usize,
    worker_index: usize,
    operation_index: usize,
) -> usize {
    let worker_count = worker_count.max(1);
    if pool_size < worker_count {
        return worker_index % pool_size;
    }

    let base_len = pool_size / worker_count;
    let remainder = pool_size % worker_count;
    let partition_len = base_len + usize::from(worker_index < remainder);
    let partition_start = worker_index
        .saturating_mul(base_len)
        .saturating_add(worker_index.min(remainder));
    partition_start + operation_index % partition_len
}
