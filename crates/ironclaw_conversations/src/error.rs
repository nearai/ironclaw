use ironclaw_turns::TurnError;

#[derive(Debug, thiserror::Error)]
pub enum InboundTurnError {
    #[error("{kind} is invalid: {reason}")]
    InvalidExternalRef { kind: &'static str, reason: String },
    #[error(
        "external actor {external_actor_id} on adapter {adapter_kind} requires pairing/binding"
    )]
    BindingRequired {
        adapter_kind: String,
        external_actor_id: String,
    },
    #[error("actor {actor_id} is not allowed to access thread {thread_id}")]
    AccessDenied { actor_id: String, thread_id: String },
    #[error("external conversation is already bound to thread {thread_id}")]
    BindingConflict { thread_id: String },
    #[error("thread {thread_id} was not found")]
    ThreadNotFound { thread_id: String },
    #[error("internal conversation state lock is poisoned")]
    StatePoisoned,
    #[error("failed to construct canonical reference: {reason}")]
    InvalidCanonicalRef { reason: String },
    #[error("turn submission failed: {error}")]
    TurnSubmissionFailed { error: TurnError },
}
