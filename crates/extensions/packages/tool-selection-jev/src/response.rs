//! Parsing one slice's answer into probabilities, rejecting anything short of
//! one valid probability per tool asked about.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The decisions response: `{answers: {<id>: {type, noul}}, ...}`. Fields
/// this crate does not use (`model`, `usage`) are ignored.
#[derive(Deserialize)]
struct DecisionsResponse {
    answers: BTreeMap<String, Answer>,
}

#[derive(Deserialize)]
struct Answer {
    /// Defaulted so an entry of another shape, under an id that was not
    /// asked, does not fail the whole response.
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    noul: Option<f64>,
}

/// Why a response body could not be used. Carries no response text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnswerError {
    /// Not JSON of the documented shape. The parser's error category is kept
    /// for the logs; its message is not, since it can quote the body.
    Malformed(serde_json::error::Category),
    /// A tool that was asked about has no answer.
    Missing,
    /// An answer is not a `noul` probability in `[0, 1]`.
    InvalidProbability,
}

impl AnswerError {
    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Malformed(_) => {
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
pub(crate) fn parse_answer(body: &[u8], asked: &[&str]) -> Result<Vec<f32>, AnswerError> {
    let response: DecisionsResponse =
        serde_json::from_slice(body).map_err(|error| AnswerError::Malformed(error.classify()))?;
    asked
        .iter()
        .map(|name| {
            let answer = response.answers.get(*name).ok_or(AnswerError::Missing)?;
            answer
                .noul
                .filter(|value| answer.kind == "noul" && (0.0..=1.0).contains(value))
                // Adding zero turns -0.0 into 0.0, which would otherwise sort
                // below the other zero scores.
                .map(|value| (value + 0.0) as f32)
                .ok_or(AnswerError::InvalidProbability)
        })
        .collect()
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
                "unasked": {"type": "noul", "noul": 1.0},
                "meta": {}
            },
            "usage": {"input_tokens": 120, "output_tokens": 4}
        }"#;
        assert_eq!(parse_answer(body, &["a", "b"]), Ok(vec![0.75, 0.25]));

        let negative_zero = br#"{"answers": {"a": {"type": "noul", "noul": -0.0}}}"#;
        let zero = parse_answer(negative_zero, &["a"]).expect("zero is a valid probability");
        assert!(zero[0].is_sign_positive(), "-0.0 is read as 0.0");
    }

    #[test]
    fn rejects_malformed_partial_and_out_of_range_answers() {
        use serde_json::error::Category;
        for (body, error) in [
            ("not json", AnswerError::Malformed(Category::Syntax)),
            (r#"{"answers": []}"#, AnswerError::Malformed(Category::Data)),
            (r#"{"answers": {}}"#, AnswerError::Missing),
            (
                r#"{"answers": {"a": {"type": "noul", "noul": 1.5}}}"#,
                AnswerError::InvalidProbability,
            ),
            (
                r#"{"answers": {"a": {"type": "noul", "noul": -0.1}}}"#,
                AnswerError::InvalidProbability,
            ),
            (
                r#"{"answers": {"a": {"type": "noul"}}}"#,
                AnswerError::InvalidProbability,
            ),
            (
                r#"{"answers": {"a": {"type": "score", "noul": 0.5}}}"#,
                AnswerError::InvalidProbability,
            ),
            (
                r#"{"answers": {"a": {"noul": 0.5}}}"#,
                AnswerError::InvalidProbability,
            ),
        ] {
            assert_eq!(parse_answer(body.as_bytes(), &["a"]), Err(error), "{body}");
        }
    }
}
