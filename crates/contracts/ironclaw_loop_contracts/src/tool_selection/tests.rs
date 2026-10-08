use super::*;

#[test]
fn debug_output_never_carries_conversation_text() {
    let request = ToolSelectionRequest {
        context: ConversationContext::new(vec!["a private request".to_string()]),
        candidates: Vec::new(),
        max_tools: 8,
        token_budget: 4_000,
    };
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("private"), "{rendered}");
    assert!(rendered.contains("bytes: 17"), "{rendered}");
}

#[test]
fn every_error_has_a_distinct_snake_case_label() {
    let summary = LoopSafeSummary::new("backend down").expect("valid summary");
    let errors = [
        ToolSelectionError::Unavailable {
            reason: summary.clone(),
        },
        ToolSelectionError::Timeout {
            elapsed: Duration::from_secs(1),
        },
        ToolSelectionError::Unauthorized,
        ToolSelectionError::PaymentRequired,
        ToolSelectionError::Rejected { status: 422 },
        ToolSelectionError::RateLimited,
        ToolSelectionError::InvalidOutput { reason: summary },
    ];
    let labels: std::collections::BTreeSet<&str> =
        errors.iter().map(ToolSelectionError::kind_label).collect();
    assert_eq!(labels.len(), errors.len());
    assert!(
        labels
            .iter()
            .all(|label| { label.chars().all(|c| c.is_ascii_lowercase() || c == '_') })
    );
}
