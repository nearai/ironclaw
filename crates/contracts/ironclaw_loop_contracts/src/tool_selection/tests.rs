use ironclaw_host_api::ids::CapabilityId;
use serde_json::json;

use super::*;

fn candidate(name: &str, parameters: serde_json::Value) -> ToolSelectionCandidate {
    ToolSelectionCandidate {
        definition: ProviderToolDefinition::from_parts(
            CapabilityId::new(format!("demo.{name}")).expect("capability id"),
            format!("demo__{name}"),
            "Secret description text.",
            parameters,
        )
        .expect("definition"),
        est_schema_tokens: 40,
    }
}

#[test]
fn conversation_context_carries_the_messages_and_never_prints_them() {
    let context = ConversationContext::new(vec!["older".to_string(), "private words".to_string()]);
    assert_eq!(context.user_messages(), ["older", "private words"]);
    assert!(!context.is_empty());
    assert!(ConversationContext::new(Vec::new()).is_empty());
    let printed = format!("{context:?}");
    assert!(!printed.contains("private"), "{printed}");
    assert!(printed.contains("messages: 2"), "{printed}");
    assert!(printed.contains("bytes: 18"), "{printed}");
}

#[test]
fn candidates_expose_parameter_names_and_hide_descriptions_from_debug() {
    let with_parameters = candidate(
        "search",
        json!({"type": "object", "properties": {"query": {}, "limit": {}}}),
    );
    let mut names = with_parameters.parameter_names();
    names.sort();
    assert_eq!(names, vec!["limit", "query"]);
    assert!(
        candidate("ping", json!({"type": "object"}))
            .parameter_names()
            .is_empty()
    );
    assert!(!format!("{with_parameters:?}").contains("Secret"));
}

#[test]
fn request_reports_the_capacity_left_for_the_classifier() {
    let request = ToolSelectionRequest {
        context: ConversationContext::new(vec!["private words".to_string()]),
        called_tools: Vec::new(),
        candidates: vec![candidate("search", json!({}))],
        pinned: vec!["demo__search".to_string()],
        max_tools: 10,
        token_budget: 1_000,
        reserved_tools: 4,
        reserved_tokens: 1_200,
    };
    assert_eq!(request.selectable_tools(), 6);
    assert_eq!(request.selectable_tokens(), 0);
    assert!(request.is_pinned("demo__search"));
    assert!(!request.is_pinned("demo__other"));
    let printed = format!("{request:?}");
    assert!(!printed.contains("private") && !printed.contains("Secret"));
}

#[test]
fn error_labels_are_stable_snake_case_and_carry_no_detail() {
    let reason = LoopSafeSummary::new("backend offline").expect("summary");
    let errors = [
        (
            ToolSelectionError::Unavailable {
                reason: reason.clone(),
            },
            "unavailable",
        ),
        (
            ToolSelectionError::Timeout {
                elapsed: Duration::from_millis(5),
            },
            "timeout",
        ),
        (ToolSelectionError::Unauthorized, "unauthorized"),
        (ToolSelectionError::PaymentRequired, "payment_required"),
        (ToolSelectionError::Rejected { status: 422 }, "rejected"),
        (ToolSelectionError::RateLimited, "rate_limited"),
        (
            ToolSelectionError::InvalidOutput { reason },
            "invalid_output",
        ),
        (
            ToolSelectionError::Ranking(ToolRetrievalError::Timeout {
                elapsed: Duration::from_millis(5),
            }),
            "ranking_failed",
        ),
    ];
    for (error, label) in errors {
        assert_eq!(error.kind_label(), label);
    }
}
