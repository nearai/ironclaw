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
    Failed {
        /// User-safe error message. Rendered into conversation replies.
        error: String,
        /// Low-level diagnostic detail preserved from the original typed
        /// error (e.g. Monty interpreter trace, Python traceback, upstream
        /// HTTP body). Never user-facing; only surfaced through gateway
        /// debug mode. `None` when the original error did not carry extra
        /// detail beyond `error`.
        debug_detail: Option<String>,
    },
    /// A unified execution gate paused the thread.
    GatePaused {
        gate_name: String,
        action_name: String,
        call_id: String,
        parameters: serde_json::Value,
        resume_kind: crate::gate::ResumeKind,
        /// Completed action output that should be injected on resume instead
        /// of re-running the action.
        resume_output: Option<serde_json::Value>,
        /// Lease snapshot captured when the gate paused the action.
        /// Boxed to keep `ThreadOutcome::GatePaused` under clippy's
        /// `large_enum_variant` threshold — `CapabilityLease` is ~360
        /// bytes and would otherwise dominate the whole enum's size.
        paused_lease: Option<Box<crate::types::capability::CapabilityLease>>,
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
