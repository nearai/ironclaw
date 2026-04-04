//! Simple async job scheduler for Proof of Claw.
//!
//! IronClaw manages the full agent loop; this module handles lightweight
//! internal POC jobs like proof generation tasks.

use std::collections::HashMap;
use uuid::Uuid;

/// Status of a scheduled job.
#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// A tracked async job.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub status: JobStatus,
}

#[derive(Default)]
pub struct JobScheduler {
    jobs: HashMap<String, Job>,
}

impl JobScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedule a job and return its generated ID.
    pub fn schedule(&mut self, mut job: Job) -> String {
        let id = Uuid::new_v4().to_string();
        job.id = id.clone();
        self.jobs.insert(id.clone(), job);
        id
    }

    /// Get the current status of a job by ID.
    pub fn get_status(&self, id: &str) -> Option<JobStatus> {
        self.jobs.get(id).map(|j| j.status.clone())
    }

    /// Mark a job as completed.
    pub fn complete(&mut self, id: &str) {
        if let Some(job) = self.jobs.get_mut(id) {
            job.status = JobStatus::Completed;
        }
    }

    /// Mark a job as failed.
    pub fn fail(&mut self, id: &str) {
        if let Some(job) = self.jobs.get_mut(id) {
            job.status = JobStatus::Failed;
        }
    }
}
