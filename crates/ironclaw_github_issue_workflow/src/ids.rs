use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{GithubIssueRef, GithubIssueWorkflowError};

const MAX_WORKFLOW_ID_BYTES: usize = 512;

fn validate_bounded_string_id(
    kind: &'static str,
    value: &str,
) -> Result<(), GithubIssueWorkflowError> {
    if value.is_empty() {
        return Err(GithubIssueWorkflowError::InvalidId {
            kind,
            value: value.to_string(),
            reason: "must not be empty",
        });
    }
    if value.len() > MAX_WORKFLOW_ID_BYTES {
        return Err(GithubIssueWorkflowError::InvalidId {
            kind,
            value: value.to_string(),
            reason: "must be at most 512 bytes",
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(GithubIssueWorkflowError::InvalidId {
            kind,
            value: value.to_string(),
            reason: "NUL/control characters are not allowed",
        });
    }
    Ok(())
}

macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn from_trusted(value: String) -> Result<Self, GithubIssueWorkflowError> {
                validate_bounded_string_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::from_trusted(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! uuid_string_id {
    ($name:ident, $kind:literal) => {
        string_id!($name, $kind);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().to_string())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

uuid_string_id!(GithubIssueWorkflowRunId, "github_issue_workflow_run");
uuid_string_id!(GithubIssueWorkflowEventId, "github_issue_workflow_event");
uuid_string_id!(GithubIssueStageRunId, "github_issue_stage_run");
uuid_string_id!(GithubIssueProviderActionId, "github_issue_provider_action");
uuid_string_id!(
    GithubIssueProviderBindingId,
    "github_issue_provider_binding"
);
uuid_string_id!(
    GithubIssueWorkspaceSessionId,
    "github_issue_workspace_session"
);
uuid_string_id!(WorkflowStepRunId, "workflow_step_run");
uuid_string_id!(WorkflowWorkerId, "workflow_worker");

string_id!(GithubIssueWorkflowRunKey, "github_issue_workflow_run_key");
string_id!(WorkflowIdempotencyKey, "workflow_idempotency_key");

impl GithubIssueWorkflowRunKey {
    pub fn for_issue(issue_ref: &GithubIssueRef) -> Result<Self, GithubIssueWorkflowError> {
        Self::from_trusted(format!(
            "github-issue:v1:{}/{}#{}",
            issue_ref.owner, issue_ref.repo, issue_ref.number
        ))
    }
}

impl WorkflowIdempotencyKey {
    pub(crate) fn from_generated(value: String) -> Self {
        if validate_bounded_string_id("workflow_idempotency_key", &value).is_ok() {
            return Self(value);
        }

        let mut hasher = Sha256::new();
        hasher.update(value.as_bytes());
        Self(format!("workflow-key-sha256:{:x}", hasher.finalize()))
    }
}
