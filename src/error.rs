//! Error types for IronClaw.

use std::time::Duration;

use uuid::Uuid;

/// Top-level error type for the agent.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("Channel error: {0}")]
    Channel(#[from] ChannelError),

    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("Safety error: {0}")]
    Safety(#[from] SafetyError),

    #[error("Job error: {0}")]
    Job(#[from] JobError),

    #[error("Estimation error: {0}")]
    Estimation(#[from] EstimationError),

    #[error("Evaluation error: {0}")]
    Evaluation(#[from] EvaluationError),

    #[error("Repair error: {0}")]
    Repair(#[from] RepairError),

    #[error("Workspace error: {0}")]
    Workspace(#[from] WorkspaceError),

    #[error("Hook error: {0}")]
    Hook(#[from] crate::hooks::HookError),

    #[error("Orchestrator error: {0}")]
    Orchestrator(#[from] OrchestratorError),

    #[error("Worker error: {0}")]
    Worker(#[from] WorkerError),

    #[error("Routine error: {0}")]
    Routine(#[from] RoutineError),
}

/// Configuration-related errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Missing required environment variable: {0}")]
    MissingEnvVar(String),

    #[error("Missing required configuration: {key}. {hint}")]
    MissingRequired { key: String, hint: String },

    #[error("Invalid configuration value for {key}: {message}")]
    InvalidValue { key: String, message: String },

    #[error("Failed to parse configuration: {0}")]
    ParseError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Database-related errors.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Connection pool error: {0}")]
    Pool(String),

    #[error("Query failed: {0}")]
    Query(String),

    #[error("Entity not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    #[error("Constraint violation: {0}")]
    Constraint(String),

    #[error("Migration failed: {0}")]
    Migration(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[cfg(feature = "postgres")]
    #[error("PostgreSQL error: {0}")]
    Postgres(#[from] tokio_postgres::Error),

    #[cfg(feature = "postgres")]
    #[error("Pool build error: {0}")]
    PoolBuild(#[from] deadpool_postgres::BuildError),

    #[cfg(feature = "postgres")]
    #[error("Pool runtime error: {0}")]
    PoolRuntime(#[from] deadpool_postgres::PoolError),

    #[cfg(feature = "libsql")]
    #[error("LibSQL error: {0}")]
    LibSql(#[from] libsql::Error),
}

/// Channel-related errors.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("Channel {name} failed to start: {reason}")]
    StartupFailed { name: String, reason: String },

    #[error("Channel {name} disconnected: {reason}")]
    Disconnected { name: String, reason: String },

    #[error("Failed to send response on channel {name}: {reason}")]
    SendFailed { name: String, reason: String },

    #[error("Channel {name} is missing a routing target: {reason}")]
    MissingRoutingTarget { name: String, reason: String },

    #[error("Invalid message format: {0}")]
    InvalidMessage(String),

    #[error("Authentication failed for channel {name}: {reason}")]
    AuthFailed { name: String, reason: String },

    #[error("Rate limited on channel {name}")]
    RateLimited { name: String },

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Channel health check failed: {name}")]
    HealthCheckFailed { name: String },
}

// LlmError lives in src/llm/error.rs; re-exported here for backward compatibility.
pub use crate::llm::error::LlmError;

/// Tool execution errors.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Tool {name} not found")]
    NotFound { name: String },

    #[error("Tool {name} execution failed: {reason}")]
    ExecutionFailed { name: String, reason: String },

    #[error("Tool {name} timed out after {timeout:?}")]
    Timeout { name: String, timeout: Duration },

    #[error("Invalid parameters for tool {name}: {reason}")]
    InvalidParameters { name: String, reason: String },

    #[error("Tool {name} is disabled: {reason}")]
    Disabled { name: String, reason: String },

    #[error("Sandbox error for tool {name}: {reason}")]
    Sandbox { name: String, reason: String },

    #[error("Tool {name} requires authentication")]
    AuthRequired { name: String },

    #[error("Tool {name} is not available for autonomous execution: {reason}")]
    AutonomousUnavailable { name: String, reason: String },

    #[error("Tool {name} is rate limited, retry after {retry_after:?}")]
    RateLimited {
        name: String,
        retry_after: Option<Duration>,
    },

    #[error("Tool builder failed: {0}")]
    BuilderFailed(String),
}

/// Safety/sanitization errors.
#[derive(Debug, thiserror::Error)]
pub enum SafetyError {
    #[error("Potential prompt injection detected: {pattern}")]
    InjectionDetected { pattern: String },

    #[error("Output exceeded maximum length: {length} > {max}")]
    OutputTooLarge { length: usize, max: usize },

    #[error("Blocked content pattern detected: {pattern}")]
    BlockedContent { pattern: String },

    #[error("Validation failed: {reason}")]
    ValidationFailed { reason: String },

    #[error("Policy violation: {rule}")]
    PolicyViolation { rule: String },
}

/// Job-related errors.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("Job {id} not found")]
    NotFound { id: Uuid },

    #[error("Job {id} already in state {state}, cannot transition to {target}")]
    InvalidTransition {
        id: Uuid,
        state: String,
        target: String,
    },

    #[error("Job {id} failed: {reason}")]
    Failed { id: Uuid, reason: String },

    #[error("Job {id} stuck for {duration:?}")]
    Stuck { id: Uuid, duration: Duration },

    #[error("Maximum parallel jobs ({max}) exceeded")]
    MaxJobsExceeded { max: usize },

    #[error("Job {id} context error: {reason}")]
    ContextError { id: Uuid, reason: String },
}

/// Estimation errors.
#[derive(Debug, thiserror::Error)]
pub enum EstimationError {
    #[error("Insufficient data for estimation: need {needed} samples, have {have}")]
    InsufficientData { needed: usize, have: usize },

    #[error("Estimation calculation failed: {reason}")]
    CalculationFailed { reason: String },

    #[error("Invalid estimation parameters: {reason}")]
    InvalidParameters { reason: String },
}

/// Evaluation errors.
#[derive(Debug, thiserror::Error)]
pub enum EvaluationError {
    #[error("Evaluation failed for job {job_id}: {reason}")]
    Failed { job_id: Uuid, reason: String },

    #[error("Missing required evaluation data: {field}")]
    MissingData { field: String },

    #[error("Invalid evaluation criteria: {reason}")]
    InvalidCriteria { reason: String },
}

/// Self-repair errors.
#[derive(Debug, thiserror::Error)]
pub enum RepairError {
    #[error("Repair failed for {target_type} {target_id}: {reason}")]
    Failed {
        target_type: String,
        target_id: Uuid,
        reason: String,
    },

    #[error("Maximum repair attempts ({max}) exceeded for {target_type} {target_id}")]
    MaxAttemptsExceeded {
        target_type: String,
        target_id: Uuid,
        max: u32,
    },

    #[error("Cannot diagnose issue for {target_type} {target_id}: {reason}")]
    DiagnosisFailed {
        target_type: String,
        target_id: Uuid,
        reason: String,
    },
}

/// Workspace/memory errors.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Document not found: {doc_type} for user {user_id}")]
    DocumentNotFound { doc_type: String, user_id: String },

    // TODO: SearchFailed is used as a catch-all for metadata, versioning, and
    // connection errors across both backends. A cleanup pass should introduce
    // more specific variants (e.g. MetadataError, VersioningError).
    #[error("Search failed: {reason}")]
    SearchFailed { reason: String },

    #[error("Embedding generation failed: {reason}")]
    EmbeddingFailed { reason: String },

    #[error("Document chunking failed: {reason}")]
    ChunkingFailed { reason: String },

    #[error("Invalid document type: {doc_type}")]
    InvalidDocType { doc_type: String },

    #[error("Workspace not initialized for user {user_id}")]
    NotInitialized { user_id: String },

    #[error("Heartbeat error: {reason}")]
    HeartbeatError { reason: String },

    #[error("I/O error: {reason}")]
    IoError { reason: String },

    #[error("Layer not found: {name}")]
    LayerNotFound { name: String },

    #[error("Layer '{name}' is read-only")]
    LayerReadOnly { name: String },

    #[error("Cannot write sensitive content: no private layer available for redirect")]
    PrivacyRedirectFailed,

    #[error("Write rejected for '{path}': prompt injection detected ({reason})")]
    InjectionRejected { path: String, reason: String },

    #[error("Version not found: document {document_id} version {version}")]
    VersionNotFound { document_id: Uuid, version: i32 },

    #[error("Patch failed for '{path}': {reason}")]
    PatchFailed { path: String, reason: String },

    #[error("Schema validation failed for '{path}': {}", errors.join("; "))]
    SchemaValidation { path: String, errors: Vec<String> },

    /// A user-supplied path or key was rejected by structural validation
    /// (path-traversal, character set, length). Distinct from
    /// `SchemaValidation` so callers can tell "your *content* is wrong"
    /// from "your *key/path* is wrong" without string-matching error
    /// messages.
    #[error("Invalid path '{path}': {reason}")]
    InvalidPath { path: String, reason: String },
}

/// Orchestrator errors (internal API, container management).
#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("Container creation failed for job {job_id}: {reason}")]
    ContainerCreationFailed { job_id: Uuid, reason: String },

    #[error("Container not found for job {job_id}")]
    ContainerNotFound { job_id: Uuid },

    #[error("Container for job {job_id} is in unexpected state: {state}")]
    InvalidContainerState { job_id: Uuid, state: String },

    #[error("Internal API error: {reason}")]
    ApiError { reason: String },

    #[error("Docker error: {reason}")]
    Docker { reason: String },

    #[error("{mode} mode is not enabled")]
    ModeDisabled { mode: String },
}

impl OrchestratorError {
    pub fn capability_contract_reason(&self) -> Option<&str> {
        match self {
            Self::ContainerCreationFailed { reason, .. }
                if crate::sandbox::is_capability_contract_violation(reason) =>
            {
                Some(reason.as_str())
            }
            _ => None,
        }
    }

    pub fn user_facing_message(&self) -> String {
        self.capability_contract_reason()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.to_string())
    }
}

/// Worker errors (container-side execution).
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("Failed to connect to orchestrator at {url}: {reason}")]
    ConnectionFailed { url: String, reason: String },

    #[error("LLM proxy request failed: {reason}")]
    LlmProxyFailed { reason: String },

    #[error("Secret resolution failed for {secret_name}: {reason}")]
    SecretResolveFailed { secret_name: String, reason: String },

    #[error("Orchestrator returned error for job {job_id}: {reason}")]
    OrchestratorRejected { job_id: Uuid, reason: String },

    #[error("Worker execution failed: {reason}")]
    ExecutionFailed { reason: String },

    #[error("Missing worker token (IRONCLAW_WORKER_TOKEN not set)")]
    MissingToken,
}

/// Routine-related errors.
#[derive(Debug, thiserror::Error)]
pub enum RoutineError {
    #[error("Unknown trigger type: {trigger_type}")]
    UnknownTriggerType { trigger_type: String },

    #[error("Unknown action type: {action_type}")]
    UnknownActionType { action_type: String },

    #[error("Missing field in {context}: {field}")]
    MissingField { context: String, field: String },

    #[error("Invalid cron expression: {reason}")]
    InvalidCron { reason: String },

    #[error("Unknown run status: {status}")]
    UnknownRunStatus { status: String },

    #[error("Routine {name} is disabled")]
    Disabled { name: String },

    #[error("Routine not found: {id}")]
    NotFound { id: Uuid },

    #[error("Not authorized to trigger routine {id}")]
    NotAuthorized { id: Uuid },

    #[error("Routine {name} is in cooldown period")]
    Cooldown { name: String },

    #[error("Routine {name} at max concurrent runs")]
    MaxConcurrent { name: String },

    #[error("Database error: {reason}")]
    Database { reason: String },

    #[error("LLM call failed: {reason}")]
    LlmFailed {
        reason: String,
        /// Partial token count consumed before the failure (if any).
        /// Used to accumulate usage across retry attempts.
        partial_tokens: Option<i32>,
        /// Whether the underlying LLM error was classified as retryable.
        /// Set at the `LlmError` → `RoutineError` conversion site using
        /// `crate::llm::retry::is_retryable()`, avoiding fragile substring
        /// matching on the stringified reason.
        retryable: bool,
    },

    #[error("Failed to dispatch full job: {reason}")]
    JobDispatchFailed { reason: String },

    #[error("LLM returned empty content")]
    EmptyResponse {
        /// Tokens consumed by the call that produced the empty response.
        partial_tokens: Option<i32>,
    },

    #[error("LLM response truncated (finish_reason=length) with no content")]
    TruncatedResponse {
        /// Tokens consumed by the call that produced the truncated response.
        partial_tokens: Option<i32>,
    },
}

impl RoutineError {
    /// Whether this error is transient and worth retrying with backoff.
    ///
    /// Retryable: LLM failures where the underlying `LlmError` was classified
    /// as retryable by `crate::llm::retry::is_retryable()`, empty responses,
    /// and truncated responses.
    /// Non-retryable: configuration errors, authorization, resource limits,
    /// DB errors, and LLM failures caused by auth/content-policy/context-length.
    pub fn is_retryable(&self) -> bool {
        match self {
            RoutineError::LlmFailed { retryable, .. } => *retryable,
            RoutineError::EmptyResponse { .. } | RoutineError::TruncatedResponse { .. } => true,
            _ => false,
        }
    }
}

/// Result type alias for the agent.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_error_display() {
        let err = ConfigError::MissingEnvVar("DATABASE_URL".to_string());
        let msg = err.to_string();
        assert!(
            msg.contains("DATABASE_URL"),
            "Should mention the variable name: {msg}"
        );

        let err = ConfigError::MissingRequired {
            key: "llm.model".to_string(),
            hint: "Set LLM_MODEL env var".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("llm.model"), "Should mention the key: {msg}");
        assert!(
            msg.contains("Set LLM_MODEL"),
            "Should include the hint: {msg}"
        );

        let err = ConfigError::InvalidValue {
            key: "port".to_string(),
            message: "must be a number".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("port"), "Should mention the key: {msg}");
    }

    #[test]
    fn database_error_display() {
        let err = DatabaseError::NotFound {
            entity: "conversation".to_string(),
            id: "abc-123".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("conversation"), "Should mention entity: {msg}");
        assert!(msg.contains("abc-123"), "Should mention id: {msg}");

        let err = DatabaseError::Query("syntax error near SELECT".to_string());
        assert!(err.to_string().contains("syntax error"));
    }

    #[test]
    fn channel_error_display() {
        let err = ChannelError::StartupFailed {
            name: "telegram".to_string(),
            reason: "invalid token".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("telegram"), "Should mention channel: {msg}");
        assert!(
            msg.contains("invalid token"),
            "Should mention reason: {msg}"
        );
    }

    #[test]
    fn job_error_display() {
        let err = JobError::MaxJobsExceeded { max: 5 };
        let msg = err.to_string();
        assert!(msg.contains("5"), "Should mention max: {msg}");

        let id = Uuid::new_v4();
        let err = JobError::NotFound { id };
        let msg = err.to_string();
        assert!(
            msg.contains(&id.to_string()),
            "Should mention job id: {msg}"
        );
    }

    #[test]
    fn safety_error_display() {
        let err = SafetyError::InjectionDetected {
            pattern: "SYSTEM:".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("SYSTEM:"), "Should mention pattern: {msg}");
    }

    #[test]
    fn workspace_error_display() {
        let err = WorkspaceError::DocumentNotFound {
            doc_type: "notes".to_string(),
            user_id: "user1".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("notes"), "Should mention doc_type: {msg}");
        assert!(msg.contains("user1"), "Should mention user_id: {msg}");
    }

    #[test]
    fn routine_error_display() {
        let err = RoutineError::InvalidCron {
            reason: "bad format".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("bad format"), "Should mention reason: {msg}");
    }

    #[test]
    fn routine_error_retryable_classification() {
        // Transient errors should be retryable
        assert!(
            RoutineError::LlmFailed {
                reason: "timeout".into(),
                partial_tokens: None,
                retryable: true,
            }
            .is_retryable()
        );
        // Non-retryable LLM error
        assert!(
            !RoutineError::LlmFailed {
                reason: "timeout".into(),
                partial_tokens: None,
                retryable: false,
            }
            .is_retryable()
        );
        assert!(
            RoutineError::EmptyResponse {
                partial_tokens: None
            }
            .is_retryable()
        );
        assert!(
            RoutineError::TruncatedResponse {
                partial_tokens: None
            }
            .is_retryable()
        );

        // Hard failures should NOT be retryable
        assert!(
            !RoutineError::Disabled {
                name: "test".into()
            }
            .is_retryable()
        );
        assert!(
            !RoutineError::JobDispatchFailed {
                reason: "no docker".into()
            }
            .is_retryable()
        );
        assert!(
            !RoutineError::Database {
                reason: "conn refused".into()
            }
            .is_retryable()
        );
        assert!(!RoutineError::NotFound { id: Uuid::new_v4() }.is_retryable());
        assert!(!RoutineError::NotAuthorized { id: Uuid::new_v4() }.is_retryable());
        assert!(
            !RoutineError::MaxConcurrent {
                name: "test".into()
            }
            .is_retryable()
        );
        assert!(
            !RoutineError::UnknownTriggerType {
                trigger_type: "x".into()
            }
            .is_retryable()
        );
        assert!(
            !RoutineError::UnknownActionType {
                action_type: "x".into()
            }
            .is_retryable()
        );
        assert!(
            !RoutineError::MissingField {
                context: "c".into(),
                field: "f".into()
            }
            .is_retryable()
        );
        assert!(
            !RoutineError::InvalidCron {
                reason: "bad".into()
            }
            .is_retryable()
        );
        assert!(
            !RoutineError::UnknownRunStatus {
                status: "bad".into()
            }
            .is_retryable()
        );
    }

    #[test]
    fn top_level_error_from_conversions() {
        let config_err = ConfigError::MissingEnvVar("TEST".to_string());
        let err: Error = config_err.into();
        assert!(matches!(err, Error::Config(_)));

        let db_err = DatabaseError::Query("test".to_string());
        let err: Error = db_err.into();
        assert!(matches!(err, Error::Database(_)));

        let job_err = JobError::MaxJobsExceeded { max: 1 };
        let err: Error = job_err.into();
        assert!(matches!(err, Error::Job(_)));

        let safety_err = SafetyError::ValidationFailed {
            reason: "test".to_string(),
        };
        let err: Error = safety_err.into();
        assert!(matches!(err, Error::Safety(_)));
    }
}
