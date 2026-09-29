//! Parsing one slice's answer into probabilities, rejecting anything short of
//! one valid probability per tool asked about.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The decisions response: `{model, answers: {<id>: {type, noul}},
/// usage: {input_tokens, output_tokens}}`.
#[derive(Debug, Deserialize)]
struct DecisionsResponse {
    #[serde(default)]
    model: Option<String>,
    answers: BTreeMap<String, Answer>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    noul: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: Option<u64>,
}

/// Longest model id kept from a response; anything longer is dropped.
const MAX_SERVED_MODEL_LEN: usize = 64;

/// One slice's probabilities, in the order the tools were asked, the input
/// tokens the service reported (zero when it reported none), and the model
/// it reported answering with (for the logs).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SliceAnswer {
    pub(crate) probabilities: Vec<f32>,
    pub(crate) input_tokens: u64,
    pub(crate) served_model: Option<String>,
}

/// The reported model id when it looks like one: short, and only letters,
/// digits and `-._:/`. It is logged, so arbitrary server text is refused.
fn served_model(model: Option<String>) -> Option<String> {
    model.filter(|model| {
        !model.is_empty()
            && model.len() <= MAX_SERVED_MODEL_LEN
            && model
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._:/".contains(c))
    })
}

/// Why a response body could not be used. Carries no response text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnswerError {
    /// Not JSON of the documented shape.
    Malformed,
    /// A tool that was asked about has no answer.
    Missing,
    /// An answer is not a `noul` probability in `[0, 1]`.
    InvalidProbability,
}

impl AnswerError {
    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Malformed => {
                "the classification response was not valid JSON of the expected shape"
            }
            Self::Missing => "the classification response left a tool unanswered",
            Self::InvalidProbability => {
                "the classification response carried an invalid probability"
            }
        }
    }
}

/// Read the probability of every name in `asked`, in that order. Answers
/// under ids that were not asked are ignored.
pub(crate) fn parse_answer(body: &[u8], asked: &[&str]) -> Result<SliceAnswer, AnswerError> {
    let response: DecisionsResponse =
        serde_json::from_slice(body).map_err(|_| AnswerError::Malformed)?;
    let mut probabilities = Vec::with_capacity(asked.len());
    for name in asked {
        let answer = response.answers.get(*name).ok_or(AnswerError::Missing)?;
        let probability = answer
            .noul
            .filter(|_| answer.kind == "noul")
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            .ok_or(AnswerError::InvalidProbability)?;
        probabilities.push(probability as f32);
    }
    Ok(SliceAnswer {
        probabilities,
        input_tokens: response
            .usage
            .and_then(|usage| usage.input_tokens)
            .unwrap_or(0),
        served_model: served_model(response.model),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_asked_probabilities_in_order_and_ignores_extra_ids() {
        let body = br#"{
            "model": "jev-1.13.0",
            "answers": {
                "b": {"type": "noul", "noul": 0.25},
                "a": {"type": "noul", "noul": 0.75},
                "unasked": {"type": "noul", "noul": 1.0}
            },
            "usage": {"input_tokens": 120, "output_tokens": 4}
        }"#;
        assert_eq!(
            parse_answer(body, &["a", "b"]),
            Ok(SliceAnswer {
                probabilities: vec![0.75, 0.25],
                input_tokens: 120,
                served_model: Some("jev-1.13.0".to_string()),
            })
        );
    }

    #[test]
    fn keeps_only_a_plausible_served_model_id() {
        let parsed = |model: &str| {
            let body = format!(
                r#"{{"model": {}, "answers": {{"a": {{"type": "noul", "noul": 0.5}}}}}}"#,
                serde_json::Value::from(model)
            );
            parse_answer(body.as_bytes(), &["a"]).map(|answer| answer.served_model)
        };
        assert_eq!(parsed("jev-latest"), Ok(Some("jev-latest".to_string())));
        assert_eq!(parsed(""), Ok(None));
        assert_eq!(parsed("jev latest\nforged=1"), Ok(None));
        assert_eq!(parsed(&"j".repeat(65)), Ok(None));
        // No `model` and no `usage` is still a usable answer.
        assert_eq!(
            parse_answer(
                br#"{"answers": {"a": {"type": "noul", "noul": 0.5}}}"#,
                &["a"]
            ),
            Ok(SliceAnswer {
                probabilities: vec![0.5],
                input_tokens: 0,
                served_model: None,
            })
        );
    }

    #[test]
    fn rejects_malformed_partial_and_out_of_range_answers() {
        assert_eq!(
            parse_answer(b"not json", &["a"]),
            Err(AnswerError::Malformed)
        );
        assert_eq!(
            parse_answer(br#"{"answers": []}"#, &["a"]),
            Err(AnswerError::Malformed)
        );
        assert_eq!(
            parse_answer(br#"{"answers": {}}"#, &["a"]),
            Err(AnswerError::Missing)
        );
        for answer in [
            r#"{"type": "noul", "noul": 1.5}"#,
            r#"{"type": "noul", "noul": -0.1}"#,
            r#"{"type": "noul"}"#,
            r#"{"type": "score", "noul": 0.5}"#,
        ] {
            let body = format!(r#"{{"answers": {{"a": {answer}}}}}"#);
            assert_eq!(
                parse_answer(body.as_bytes(), &["a"]),
                Err(AnswerError::InvalidProbability),
                "{answer}"
            );
        }
    }
}
