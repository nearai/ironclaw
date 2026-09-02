use super::*;

#[test]
fn prefixed_capability_summary_does_not_underflow_when_prefix_is_too_long() {
    let prefix = "x".repeat(MAX_SAFE_SUMMARY_BYTES + 1);
    let summary = prefixed_capability_summary(prefix, "detail".to_string());

    // An oversized combination degrades to the fixed fallback instead of
    // becoming a terminal PlannerContract error.
    assert_eq!(summary.as_str(), "the tool failure details were redacted");
}

#[test]
fn prefixed_capability_summary_degrades_marker_bearing_prefix_without_borking() {
    // Regression: `Failed(Authorization)` builds the prefix "capability
    // failed with authorization: ", whose "authorization:" substring is a
    // banned marker — this used to return a terminal PlannerContract error
    // before the model ever saw the tool failure.
    let summary = capability_failed_summary(
        FailureKind::Authorization,
        "the provider token has expired".to_string(),
    );

    assert_eq!(summary.as_str(), "the tool failure details were redacted");
}

#[test]
fn prefixed_capability_summary_rephrases_fixed_input_encode_summary() {
    let summary = prefixed_capability_summary(
        "capability failed with invalid_input: ".to_string(),
        INPUT_ENCODE_HUMAN_SUMMARY.to_string(),
    );

    assert_eq!(
        summary.as_str(),
        "capability failed with invalid_input: input could not be encoded"
    );
}
