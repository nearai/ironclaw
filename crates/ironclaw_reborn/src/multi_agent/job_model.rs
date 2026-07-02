use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    Pending,
    Claimed,
    Running,
    WaitingForChildren,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Master,
    SubAgent,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::SubAgent => "sub_agent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimLease {
    pub worker_id: String,
    pub claimed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentJob {
    pub id: String,
    pub root_id: String,
    pub parent_id: Option<String>,
    pub agent_kind: AgentKind,
    pub task: String,
    pub status: AgentStatus,
    pub depth: u32,
    pub max_depth: u32,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub claim_lease: Option<ClaimLease>,
    pub max_retries: u32,
    pub retry_count: u32,
}

impl AgentJob {
    pub fn new_root(
        id: String,
        task: impl Into<String>,
        max_depth: u32,
        max_retries: u32,
    ) -> Self {
        let now = Utc::now();
        let task = task.into();
        Self {
            id: id.clone(),
            root_id: id,
            parent_id: None,
            agent_kind: AgentKind::Master,
            task,
            status: AgentStatus::Pending,
            depth: 0,
            max_depth,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            claim_lease: None,
            max_retries,
            retry_count: 0,
        }
    }

    pub fn new_child(
        id: String,
        parent: &AgentJob,
        task: impl Into<String>,
        max_retries: u32,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.clone(),
            root_id: parent.root_id.clone(),
            parent_id: Some(parent.id.clone()),
            agent_kind: AgentKind::SubAgent,
            task: task.into(),
            status: AgentStatus::Pending,
            depth: parent.depth.saturating_add(1),
            max_depth: parent.max_depth,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            claim_lease: None,
            max_retries,
            retry_count: 0,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            AgentStatus::Complete | AgentStatus::Failed | AgentStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub id: String,
    pub job_id: String,
    pub root_id: String,
    pub agent_kind: AgentKind,
    pub status: AgentStatus,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

impl AgentEvent {
    pub fn new(
        id: String,
        job: &AgentJob,
        status: AgentStatus,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id,
            job_id: job.id.clone(),
            root_id: job.root_id.clone(),
            agent_kind: job.agent_kind,
            status,
            message: message.into(),
            timestamp: Utc::now(),
        }
    }
}
