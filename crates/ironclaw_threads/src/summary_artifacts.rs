use crate::{
    CreateSummaryArtifactRequest, SessionThreadError, SummaryArtifact, SummaryKind,
    SummaryModelContextPolicy,
};

pub(crate) fn is_exact_compaction_summary_replay(
    summary: &SummaryArtifact,
    request: &CreateSummaryArtifactRequest,
    content: &str,
) -> bool {
    summary.thread_id == request.thread_id
        && summary.start_sequence == request.start_sequence
        && summary.end_sequence == request.end_sequence
        && summary.content == content
}

/// Callers with `model_context_policy != ReplaceRangeWhenSelected` skip overlap
/// checks by design.
pub(crate) fn find_overlapping_summary<'a>(
    summaries: &'a [SummaryArtifact],
    request: &CreateSummaryArtifactRequest,
    content: &str,
) -> Result<Option<&'a SummaryArtifact>, SessionThreadError> {
    if request.model_context_policy != Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected) {
        return Ok(None);
    }

    let overlapping: Vec<_> = summaries
        .iter()
        .filter(|summary| {
            summary.model_context_policy
                == Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected)
                && ranges_overlap(
                    request.start_sequence,
                    request.end_sequence,
                    summary.start_sequence,
                    summary.end_sequence,
                )
        })
        .collect();

    match overlapping.as_slice() {
        [] => Ok(None),
        [overlapping] => {
            if request.summary_kind == SummaryKind::Compaction
                && overlapping.summary_kind == request.summary_kind
                && is_exact_compaction_summary_replay(overlapping, request, content)
            {
                Ok(Some(overlapping))
            } else {
                Err(SessionThreadError::OverlappingSummaryRange {
                    start_sequence: request.start_sequence,
                    end_sequence: request.end_sequence,
                })
            }
        }
        _ => Err(SessionThreadError::OverlappingSummaryRange {
            start_sequence: request.start_sequence,
            end_sequence: request.end_sequence,
        }),
    }
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

    fn summary_with(start_sequence: u64, end_sequence: u64, content: &str) -> SummaryArtifact {
        SummaryArtifact {
            summary_id: crate::SummaryArtifactId::new(),
            thread_id: ThreadId::new("thread-test").unwrap(),
            start_sequence,
            end_sequence,
            summary_kind: SummaryKind::Compaction,
            content: content.to_string(),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        }
    }

    #[test]
    fn exact_compaction_summary_replay_matches_on_coordinates_and_content() {
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
    fn find_overlapping_summary_returns_none_when_no_overlaps_exist() {
        let request = request();
        let summaries = vec![summary_with(10, 12, "summary content")];

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
        let summary = summary_with(2, 4, "different content");

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

    #[test]
    fn find_overlapping_summary_rejects_later_exact_match_after_earlier_nonmatching_overlap() {
        let request = request();
        let summaries = vec![summary_with(2, 4, "different content"), summary()];

        let error = find_overlapping_summary(&summaries, &request, "summary content")
            .expect_err("multiple overlaps should be rejected");

        assert!(matches!(
            error,
            SessionThreadError::OverlappingSummaryRange {
                start_sequence: 2,
                end_sequence: 4
            }
        ));
    }

    #[test]
    fn find_overlapping_summary_rejects_multiple_overlaps_with_exact_match_present() {
        let request = request();
        let summaries = vec![summary(), summary_with(3, 5, "different content")];

        let error = find_overlapping_summary(&summaries, &request, "summary content")
            .expect_err("multiple overlaps should be rejected");

        assert!(matches!(
            error,
            SessionThreadError::OverlappingSummaryRange {
                start_sequence: 2,
                end_sequence: 4
            }
        ));
    }
}
