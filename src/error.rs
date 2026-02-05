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

    #[error("PostgreSQL error: {0}")]
    Postgres(#[from] tokio_postgres::Error),

    #[error("Pool build error: {0}")]
    PoolBuild(#[from] deadpool_postgres::BuildError),

    #[error("Pool runtime error: {0}")]
    PoolRuntime(#[from] deadpool_postgres::PoolError),
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

/// LLM provider errors.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Provider {provider} request failed: {reason}")]
    RequestFailed { provider: String, reason: String },

    #[error("Provider {provider} rate limited, retry after {retry_after:?}")]
    RateLimited {
        provider: String,
        retry_after: Option<Duration>,
    },

    #[error("Invalid response from {provider}: {reason}")]
    InvalidResponse { provider: String, reason: String },

    #[error("Context length exceeded: {used} tokens used, {limit} allowed")]
    ContextLengthExceeded { used: usize, limit: usize },

    #[error("Model {model} not available on provider {provider}")]
    ModelNotAvailable { provider: String, model: String },

    #[error("Authentication failed for provider {provider}")]
    AuthFailed { provider: String },

    #[error("Session expired for provider {provider}")]
    SessionExpired { provider: String },

    #[error("Session renewal failed for provider {provider}: {reason}")]
    SessionRenewalFailed { provider: String, reason: String },

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

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
}

/// Result type alias for the agent.
pub type Result<T> = std::result::Result<T, Error>;
