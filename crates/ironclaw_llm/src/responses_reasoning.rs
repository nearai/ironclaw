use serde_json::{Value, json};

pub(crate) fn summary_request() -> Value {
    json!({ "effort": "medium", "summary": "auto" })
}

pub(crate) fn apply_summary_event(reasoning: &mut String, event_type: &str, data: &Value) -> bool {
    match event_type {
        "response.reasoning_summary_text.delta" => {
            if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
                reasoning.push_str(delta);
            }
            true
        }
        "response.reasoning_summary_text.done" => {
            if reasoning.trim().is_empty()
                && let Some(text) = data.get("text").and_then(|t| t.as_str())
            {
                reasoning.push_str(text);
            }
            true
        }
        "response.reasoning_summary_part.done" => {
            if reasoning.trim().is_empty()
                && let Some(text) = data
                    .get("part")
                    .and_then(|part| part.get("text"))
                    .and_then(|text| text.as_str())
            {
                reasoning.push_str(text);
            }
            true
        }
        _ => false,
    }
}

pub(crate) fn finish_summary(reasoning: String) -> Option<String> {
    (!reasoning.trim().is_empty()).then_some(reasoning)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_request_enables_reasoning_effort() {
        assert_eq!(
            summary_request(),
            json!({ "effort": "medium", "summary": "auto" })
        );
    }

    #[test]
    fn captures_summary_part_done_when_text_events_are_absent() {
        let mut reasoning = String::new();
        let data = json!({
            "part": {
                "type": "summary_text",
                "text": "Checked the user request and selected a concise answer."
            }
        });

        assert!(apply_summary_event(
            &mut reasoning,
            "response.reasoning_summary_part.done",
            &data
        ));
        assert_eq!(
            reasoning,
            "Checked the user request and selected a concise answer."
        );
    }

    #[test]
    fn captures_summary_text_done_when_accumulator_empty() {
        let mut reasoning = String::new();
        let data = json!({
            "text": "Fallback reasoning from done event"
        });

        assert!(apply_summary_event(
            &mut reasoning,
            "response.reasoning_summary_text.done",
            &data
        ));
        assert_eq!(reasoning, "Fallback reasoning from done event");
    }

    #[test]
    fn done_event_does_not_overwrite_accumulated_deltas() {
        let mut reasoning = "Existing delta content".to_string();
        let data = json!({
            "text": "This should not overwrite"
        });

        assert!(apply_summary_event(
            &mut reasoning,
            "response.reasoning_summary_text.done",
            &data
        ));
        // Done event should not overwrite when accumulator has content
        assert_eq!(reasoning, "Existing delta content");
    }

    #[test]
    fn delta_event_appends_to_accumulator() {
        let mut reasoning = "Initial ".to_string();
        let data = json!({
            "delta": "continuation"
        });

        assert!(apply_summary_event(
            &mut reasoning,
            "response.reasoning_summary_text.delta",
            &data
        ));
        assert_eq!(reasoning, "Initial continuation");
    }

    #[test]
    fn delta_event_ignores_non_string_delta() {
        let mut reasoning = String::new();
        let data = json!({
            "delta": 123  // Invalid type
        });

        assert!(apply_summary_event(
            &mut reasoning,
            "response.reasoning_summary_text.delta",
            &data
        ));
        assert_eq!(reasoning, "");
    }

    #[test]
    fn unknown_event_type_returns_false() {
        let mut reasoning = String::new();
        let data = json!({});

        assert!(!apply_summary_event(
            &mut reasoning,
            "response.unknown_event",
            &data
        ));
    }

    #[test]
    fn finish_summary_returns_none_for_empty_string() {
        assert_eq!(finish_summary(String::new()), None);
    }

    #[test]
    fn finish_summary_returns_none_for_whitespace_only() {
        assert_eq!(finish_summary("   \n\t  ".to_string()), None);
    }

    #[test]
    fn finish_summary_returns_some_for_valid_content() {
        assert_eq!(
            finish_summary("Valid reasoning".to_string()),
            Some("Valid reasoning".to_string())
        );
    }
}
