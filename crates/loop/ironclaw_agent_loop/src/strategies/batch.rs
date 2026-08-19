//! Wire vocabulary for reporting capability-batch execution.

/// Batch-level execution mode. Wire-stable: serialized into checkpoints and
/// emitted on observability events, so the snake_case names are part of the
/// public contract.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BatchPolicy {
    Sequential,
    Parallel,
}
