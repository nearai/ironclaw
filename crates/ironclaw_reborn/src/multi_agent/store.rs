use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;

use super::error::MultiAgentError;
use super::job_model::{AgentEvent, AgentJob, AgentStatus, ClaimLease};

pub trait AgentJobStore: Send + Sync {
    fn insert_job(&self, job: AgentJob) -> Result<(), MultiAgentError>;
    fn get_job(&self, id: &str) -> Result<Option<AgentJob>, MultiAgentError>;
    fn update_job(&self, job: AgentJob) -> Result<(), MultiAgentError>;
    fn list_jobs_for_root(&self, root_id: &str) -> Result<Vec<AgentJob>, MultiAgentError>;
    fn list_children(&self, parent_id: &str) -> Result<Vec<AgentJob>, MultiAgentError>;
    fn append_event(&self, event: AgentEvent) -> Result<(), MultiAgentError>;
    fn list_events_for_root(&self, root_id: &str) -> Result<Vec<AgentEvent>, MultiAgentError>;
    fn next_job_id(&self) -> String;
    fn next_event_id(&self) -> String;
    fn requeue_expired_claims(&self, now: DateTime<Utc>) -> Result<(), MultiAgentError>;
    fn claim_next_pending(
        &self,
        worker_id: &str,
        lease: Duration,
        now: DateTime<Utc>,
    ) -> Result<Option<AgentJob>, MultiAgentError>;
    fn jobs_waiting_for_children(&self) -> Result<Vec<AgentJob>, MultiAgentError>;
    fn cancel_subtree(&self, root_id: &str, job_id: &str) -> Result<(), MultiAgentError>;
}

#[derive(Default)]
struct StoreState {
    jobs: Vec<AgentJob>,
    events: Vec<AgentEvent>,
}

pub struct InMemoryAgentJobStore {
    state: RwLock<StoreState>,
    job_seq: AtomicU64,
    event_seq: AtomicU64,
    max_iterations: u32,
    iterations_used: AtomicU64,
}

impl InMemoryAgentJobStore {
    pub fn new(max_iterations: u32) -> Self {
        Self {
            state: RwLock::new(StoreState::default()),
            job_seq: AtomicU64::new(0),
            event_seq: AtomicU64::new(0),
            max_iterations,
            iterations_used: AtomicU64::new(0),
        }
    }

    fn reserve_iteration(&self) -> Result<(), MultiAgentError> {
        let next = self.iterations_used.fetch_add(1, Ordering::SeqCst) + 1;
        if next > u64::from(self.max_iterations) {
            return Err(MultiAgentError::MaxIterationsExceeded {
                max_iterations: self.max_iterations,
            });
        }
        Ok(())
    }
}

impl AgentJobStore for InMemoryAgentJobStore {
    fn insert_job(&self, job: AgentJob) -> Result<(), MultiAgentError> {
        self.reserve_iteration()?;
        let mut state = self.state.write();
        if state.jobs.iter().any(|existing| existing.id == job.id) {
            return Err(MultiAgentError::OrchestrationFailed {
                reason: format!("duplicate job id {}", job.id),
            });
        }
        state.jobs.push(job);
        Ok(())
    }

    fn get_job(&self, id: &str) -> Result<Option<AgentJob>, MultiAgentError> {
        let state = self.state.read();
        Ok(state.jobs.iter().find(|job| job.id == id).cloned())
    }

    fn update_job(&self, job: AgentJob) -> Result<(), MultiAgentError> {
        let mut state = self.state.write();
        let slot = state
            .jobs
            .iter_mut()
            .find(|existing| existing.id == job.id)
            .ok_or_else(|| MultiAgentError::JobNotFound {
                job_id: job.id.clone(),
            })?;
        *slot = job;
        Ok(())
    }

    fn list_jobs_for_root(&self, root_id: &str) -> Result<Vec<AgentJob>, MultiAgentError> {
        let state = self.state.read();
        Ok(state
            .jobs
            .iter()
            .filter(|job| job.root_id == root_id)
            .cloned()
            .collect())
    }

    fn list_children(&self, parent_id: &str) -> Result<Vec<AgentJob>, MultiAgentError> {
        let state = self.state.read();
        Ok(state
            .jobs
            .iter()
            .filter(|job| job.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect())
    }

    fn append_event(&self, event: AgentEvent) -> Result<(), MultiAgentError> {
        self.state.write().events.push(event);
        Ok(())
    }

    fn list_events_for_root(&self, root_id: &str) -> Result<Vec<AgentEvent>, MultiAgentError> {
        let state = self.state.read();
        Ok(state
            .events
            .iter()
            .filter(|event| event.root_id == root_id)
            .cloned()
            .collect())
    }

    fn next_job_id(&self) -> String {
        format!("job-{}", self.job_seq.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn next_event_id(&self) -> String {
        format!("event-{}", self.event_seq.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn requeue_expired_claims(&self, now: DateTime<Utc>) -> Result<(), MultiAgentError> {
        let mut state = self.state.write();
        for job in &mut state.jobs {
            if job.status != AgentStatus::Claimed {
                continue;
            }
            let Some(lease) = &job.claim_lease else {
                job.status = AgentStatus::Pending;
                job.updated_at = now;
                continue;
            };
            if lease.expires_at <= now {
                job.status = AgentStatus::Pending;
                job.claim_lease = None;
                job.updated_at = now;
            }
        }
        Ok(())
    }

    fn claim_next_pending(
        &self,
        worker_id: &str,
        lease: Duration,
        now: DateTime<Utc>,
    ) -> Result<Option<AgentJob>, MultiAgentError> {
        let mut state = self.state.write();
        let index = state
            .jobs
            .iter()
            .position(|job| job.status == AgentStatus::Pending);
        let Some(index) = index else {
            return Ok(None);
        };
        let expires_at = now + chrono::Duration::from_std(lease).unwrap_or_else(|_| chrono::Duration::seconds(30));
        state.jobs[index].status = AgentStatus::Claimed;
        state.jobs[index].claim_lease = Some(ClaimLease {
            worker_id: worker_id.to_string(),
            claimed_at: now,
            expires_at,
        });
        state.jobs[index].updated_at = now;
        Ok(Some(state.jobs[index].clone()))
    }

    fn jobs_waiting_for_children(&self) -> Result<Vec<AgentJob>, MultiAgentError> {
        let state = self.state.read();
        Ok(state
            .jobs
            .iter()
            .filter(|job| job.status == AgentStatus::WaitingForChildren)
            .cloned()
            .collect())
    }

    fn cancel_subtree(&self, root_id: &str, job_id: &str) -> Result<(), MultiAgentError> {
        let mut state = self.state.write();
        let mut to_cancel = std::collections::HashSet::from([job_id.to_string()]);
        loop {
            let mut added = false;
            for job in &state.jobs {
                if job.root_id != root_id {
                    continue;
                }
                if let Some(parent_id) = &job.parent_id {
                    if to_cancel.contains(parent_id) && to_cancel.insert(job.id.clone()) {
                        added = true;
                    }
                }
            }
            if !added {
                break;
            }
        }
        let now = Utc::now();
        for job in &mut state.jobs {
            if job.root_id == root_id && to_cancel.contains(&job.id) {
                job.status = AgentStatus::Cancelled;
                job.updated_at = now;
                job.claim_lease = None;
            }
        }
        Ok(())
    }
}
