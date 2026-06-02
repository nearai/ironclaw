use crate::{
    CreateSummaryArtifactRequest, SessionThreadError, SummaryArtifact, SummaryKind,
    SummaryModelContextPolicy,
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

pub(crate) fn find_overlapping_summary<'a>(
    summaries: &'a [SummaryArtifact],
    request: &CreateSummaryArtifactRequest,
    content: &str,
) -> Result<Option<&'a SummaryArtifact>, SessionThreadError> {
    if request.model_context_policy != Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected) {
        return Ok(None);
    }

    let Some(overlapping) = summaries.iter().find(|summary| {
        summary.model_context_policy == Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected)
            && ranges_overlap(
                request.start_sequence,
                request.end_sequence,
                summary.start_sequence,
                summary.end_sequence,
            )
    }) else {
        return Ok(None);
    };

    if is_exact_compaction_summary_replay(overlapping, request, content) {
        return Ok(Some(overlapping));
    }

    Err(SessionThreadError::OverlappingSummaryRange {
        start_sequence: request.start_sequence,
        end_sequence: request.end_sequence,
    })
}

fn ranges_overlap(left_start: u64, left_end: u64, right_start: u64, right_end: u64) -> bool {
    left_start <= right_end && right_start <= left_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};

    fn scope() -> crate::ThreadScope {
        crate::ThreadScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            agent_id: AgentId::new("agent-test").unwrap(),
            project_id: Some(ProjectId::new("project-test").unwrap()),
            owner_user_id: Some(UserId::new("user-test").unwrap()),
            mission_id: None,
        }
    }

    fn request() -> CreateSummaryArtifactRequest {
        CreateSummaryArtifactRequest {
            scope: scope(),
            thread_id: ThreadId::new("thread-test").unwrap(),
            start_sequence: 2,
            end_sequence: 4,
            summary_kind: SummaryKind::Compaction,
            content: crate::MessageContent::text("summary content"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        }
    }

    fn summary() -> SummaryArtifact {
        SummaryArtifact {
            summary_id: crate::SummaryArtifactId::new(),
            thread_id: ThreadId::new("thread-test").unwrap(),
            start_sequence: 2,
            end_sequence: 4,
            summary_kind: SummaryKind::Compaction,
            content: "summary content".to_string(),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        }
    }

    #[test]
    fn exact_compaction_summary_replay_matches_on_all_fields() {
        let request = request();
        let summary = summary();

        assert!(is_exact_compaction_summary_replay(
            &summary,
            &request,
            "summary content"
        ));
    }

    #[test]
    fn exact_compaction_summary_replay_rejects_content_mismatch() {
        let request = request();
        let summary = summary();

        assert!(!is_exact_compaction_summary_replay(
            &summary,
            &request,
            "different content"
        ));
    }

    #[test]
    fn exact_compaction_summary_replay_rejects_start_sequence_mismatch() {
        let request = request();
        let mut summary = summary();
        summary.start_sequence = 1;

        assert!(!is_exact_compaction_summary_replay(
            &summary,
            &request,
            "summary content"
        ));
    }

    #[test]
    fn exact_compaction_summary_replay_rejects_end_sequence_mismatch() {
        let request = request();
        let mut summary = summary();
        summary.end_sequence = 5;

        assert!(!is_exact_compaction_summary_replay(
            &summary,
            &request,
            "summary content"
        ));
    }

    #[test]
    fn exact_compaction_summary_replay_rejects_thread_id_mismatch() {
        let request = request();
        let mut summary = summary();
        summary.thread_id = ThreadId::new("other-thread").unwrap();

        assert!(!is_exact_compaction_summary_replay(
            &summary,
            &request,
            "summary content"
        ));
    }

    #[test]
    fn exact_compaction_summary_replay_rejects_policy_mismatch() {
        let mut request = request();
        request.model_context_policy = None;

        assert!(!is_exact_compaction_summary_replay(
            &summary(),
            &request,
            "summary content"
        ));
    }

    #[test]
    fn exact_compaction_summary_replay_rejects_summary_policy_mismatch() {
        let request = request();
        let mut summary = summary();
        summary.model_context_policy = None;

        assert!(!is_exact_compaction_summary_replay(
            &summary,
            &request,
            "summary content"
        ));
    }

    #[test]
    fn find_overlapping_summary_allows_policy_none() {
        let mut request = request();
        request.model_context_policy = None;

        let summaries = vec![summary()];
        assert!(matches!(
            find_overlapping_summary(&summaries, &request, "summary content"),
            Ok(None)
        ));
    }

    #[test]
    fn find_overlapping_summary_replays_exact_match() {
        let request = request();
        let summaries = vec![summary()];

        let overlapping = find_overlapping_summary(&summaries, &request, "summary content")
            .unwrap()
            .unwrap();

        assert_eq!(overlapping.summary_id, summaries[0].summary_id);
    }

    #[test]
    fn find_overlapping_summary_rejects_non_idempotent_overlap() {
        let request = request();
        let mut summary = summary();
        summary.content = "different content".to_string();

        let error = find_overlapping_summary(&[summary], &request, "summary content")
            .expect_err("overlap should be rejected");

        assert!(matches!(
            error,
            SessionThreadError::OverlappingSummaryRange {
                start_sequence: 2,
                end_sequence: 4
            }
        ));
    }
}
