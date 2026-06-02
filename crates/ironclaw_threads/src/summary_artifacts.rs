use crate::{
    CreateSummaryArtifactRequest, SummaryArtifact, SummaryKind, SummaryModelContextPolicy,
};

pub(crate) fn is_exact_compaction_summary_replay(
    summary: &SummaryArtifact,
    request: &CreateSummaryArtifactRequest,
    content: &str,
) -> bool {
    request.summary_kind == SummaryKind::Compaction
        && request.model_context_policy == Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected)
        && summary.thread_id == request.thread_id
        && summary.start_sequence == request.start_sequence
        && summary.end_sequence == request.end_sequence
        && summary.summary_kind == SummaryKind::Compaction
        && summary.model_context_policy == Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected)
        && summary.content == content
}
