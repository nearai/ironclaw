//! LLM-as-judge scoring for benchmark turns.
//!
//! Provides functions to format a judge prompt, parse the score from the
//! judge's response, and run the full judge evaluation for a single turn.

use std::sync::Arc;

use regex::Regex;

use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};

/// Format a prompt asking the judge LLM to evaluate the agent's performance
/// on a 1-10 scale.
///
/// The prompt includes the user message, agent response, tool calls summary,
/// and the evaluation criteria. The judge is instructed to respond with
/// "SCORE: N" format.
pub fn format_judge_prompt(
    user_message: &str,
    agent_response: &str,
    tool_calls: &[(String, bool)],
    criteria: &str,
) -> String {
    let tool_calls_summary = if tool_calls.is_empty() {
        "No tools were called.".to_string()
    } else {
        tool_calls
            .iter()
            .map(|(name, success)| {
                let status = if *success { "succeeded" } else { "failed" };
                format!("- {name}: {status}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"You are an expert evaluator assessing an AI agent's performance.

## User Message
{user_message}

## Agent Response
{agent_response}

## Tool Calls
{tool_calls_summary}

## Evaluation Criteria
{criteria}

## Instructions
Rate the agent's performance on a scale of 1 to 10, where:
- 1-3: Poor (fails to address the task, incorrect, or harmful)
- 4-6: Adequate (partially addresses the task but with notable gaps)
- 7-9: Good (addresses the task well with minor issues)
- 10: Excellent (perfectly addresses the task)

Provide a brief justification, then end your response with the score in this exact format:
SCORE: N

where N is a single integer from 1 to 10."#
    )
}

/// Parse "SCORE: N" from the judge's response.
///
/// Returns `None` if the pattern is not found or the score is outside the
/// 1-10 range.
pub fn parse_judge_score(response: &str) -> Option<u8> {
    let re = Regex::new(r"SCORE:\s*(\d{1,2})").ok()?;
    let caps = re.captures(response)?;
    let score: u8 = caps[1].parse().ok()?;
    if (1..=10).contains(&score) {
        Some(score)
    } else {
        None
    }
}

/// Call the LLM with the judge prompt and parse the score.
///
/// Returns `None` if the LLM call fails or the response cannot be parsed.
pub async fn judge_turn(
    llm: &Arc<dyn LlmProvider>,
    user_message: &str,
    agent_response: &str,
    tool_calls: &[(String, bool)],
    criteria: &str,
) -> Option<u8> {
    let prompt = format_judge_prompt(user_message, agent_response, tool_calls, criteria);
    let messages = vec![ChatMessage::user(prompt)];
    let request = CompletionRequest::new(messages);

    match llm.complete(request).await {
        Ok(resp) => {
            let score = parse_judge_score(&resp.content);
            if score.is_none() {
                tracing::warn!(
                    response = %resp.content,
                    "Judge response did not contain a valid SCORE: N pattern"
                );
            }
            score
        }
        Err(e) => {
            tracing::warn!(error = %e, "Judge LLM call failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_judge_prompt() {
        let prompt = format_judge_prompt(
            "What time is it?",
            "The current time is 3:00 PM.",
            &[("time".to_string(), true)],
            "Response should include the current time",
        );

        assert!(prompt.contains("What time is it?"));
        assert!(prompt.contains("The current time is 3:00 PM."));
        assert!(prompt.contains("time: succeeded"));
        assert!(prompt.contains("Response should include the current time"));
        assert!(prompt.contains("SCORE: N"));
        assert!(prompt.contains("1 to 10"));
    }

    #[test]
    fn test_format_judge_prompt_no_tools() {
        let prompt = format_judge_prompt("Hello", "Hi there!", &[], "Be friendly");

        assert!(prompt.contains("No tools were called."));
    }

    #[test]
    fn test_parse_judge_score_valid() {
        assert_eq!(
            parse_judge_score("Good job overall.\nSCORE: 8\nNice work"),
            Some(8)
        );
    }

    #[test]
    fn test_parse_judge_score_boundary() {
        assert_eq!(parse_judge_score("SCORE: 1"), Some(1));
        assert_eq!(parse_judge_score("SCORE: 10"), Some(10));
    }

    #[test]
    fn test_parse_judge_score_out_of_range() {
        assert_eq!(parse_judge_score("SCORE: 0"), None);
        assert_eq!(parse_judge_score("SCORE: 11"), None);
    }

    #[test]
    fn test_parse_judge_score_missing() {
        assert_eq!(parse_judge_score("No score here"), None);
    }

    #[test]
    fn test_parse_judge_score_with_whitespace() {
        assert_eq!(parse_judge_score("SCORE:  7"), Some(7));
        assert_eq!(parse_judge_score("SCORE:   3"), Some(3));
    }
}
