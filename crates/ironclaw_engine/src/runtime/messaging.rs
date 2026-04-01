//! Thread-to-thread messaging via channels.

use crate::types::message::ThreadMessage;
use crate::types::thread::ThreadId;

/// Signal sent to a running thread via its mailbox.
#[derive(Debug)]
pub enum ThreadSignal {
    /// Stop the thread gracefully.
    Stop,
    /// Pause execution (can be resumed later).
    Suspend,
    /// Resume a suspended thread.
    Resume,
    /// Inject a user message into the thread's context.
    InjectMessage(ThreadMessage),
    /// Notification that a child thread completed.
    ChildCompleted {
        child_id: ThreadId,
        outcome: ThreadOutcome,
    },
}

/// Final outcome of a thread's execution.
#[derive(Debug, Clone)]
pub enum ThreadOutcome {
    /// Completed with an optional text response.
    Completed { response: Option<String> },
    /// Thread was stopped by a signal.
    Stopped,
    /// Max iterations reached without completing.
    MaxIterations,
    /// Terminal failure.
    Failed { error: String },
    /// A capability action requires user approval before continuing.
    NeedApproval {
        action_name: String,
        call_id: String,
        parameters: serde_json::Value,
    },
    /// An action needs a credential that requires user authentication (e.g. OAuth).
    /// The thread pauses until the credential is available, then resumes.
    NeedAuthentication {
        credential_name: String,
        action_name: String,
        call_id: String,
        parameters: serde_json::Value,
    },
    /// A unified execution gate paused the thread. Replaces separate
    /// `NeedApproval` and `NeedAuthentication` for new code paths.
    GatePaused {
        gate_name: String,
        action_name: String,
        call_id: String,
        parameters: serde_json::Value,
        resume_kind: crate::gate::ResumeKind,
    },
}

/// A mailbox for sending signals to a running thread.
///
/// Each thread gets a `(sender, receiver)` pair. The `ThreadManager` holds
/// the sender; the `ExecutionLoop` holds the receiver.
pub type SignalSender = tokio::sync::mpsc::Sender<ThreadSignal>;
pub type SignalReceiver = tokio::sync::mpsc::Receiver<ThreadSignal>;

/// Create a new signal channel with the given buffer size.
pub fn signal_channel(buffer: usize) -> (SignalSender, SignalReceiver) {
    tokio::sync::mpsc::channel(buffer)
}
