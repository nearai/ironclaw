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
}
