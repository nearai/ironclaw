use ironclaw_turns::run_profile::{LoopContextCompactionKind, LoopContextCompactionMetadata};

/// Builds one compaction index entry for prompt-bundle fixtures.
pub fn compaction_metadata(
    sequence: u64,
    kind: LoopContextCompactionKind,
    estimated_tokens: u64,
) -> LoopContextCompactionMetadata {
    LoopContextCompactionMetadata {
        sequence,
        kind,
        estimated_tokens,
    }
}

/// Builds a compactable prompt index that preserves the active user tail.
pub fn active_task_preserving_compaction_index() -> Vec<LoopContextCompactionMetadata> {
    vec![
        compaction_metadata(1, LoopContextCompactionKind::User, 10),
        compaction_metadata(2, LoopContextCompactionKind::Assistant, 10),
        compaction_metadata(3, LoopContextCompactionKind::User, 10),
        compaction_metadata(4, LoopContextCompactionKind::Assistant, 10),
        compaction_metadata(5, LoopContextCompactionKind::User, 10),
        compaction_metadata(6, LoopContextCompactionKind::Assistant, 3_000),
        compaction_metadata(7, LoopContextCompactionKind::User, 10),
        compaction_metadata(8, LoopContextCompactionKind::Assistant, 6_000),
    ]
}
