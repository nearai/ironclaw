//! Success evaluation for jobs.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context::{ActionRecord, JobContext};
use crate::error::EvaluationError;
use crate::llm::LlmProvider;

/// Result of evaluating job success.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// Whether the job was successful.
    pub success: bool,
    /// Confidence in the evaluation (0-1).
    pub confidence: f64,
    /// Detailed reasoning.
    pub reasoning: String,
    /// Specific issues found.
    pub issues: Vec<String>,
    /// Suggestions for improvement.
    pub suggestions: Vec<String>,
    /// Quality score (0-100).
    pub quality_score: u32,
}

impl EvaluationResult {
    /// Create a successful evaluation.
    pub fn success(reasoning: impl Into<String>, quality_score: u32) -> Self {
        Self {
            success: true,
            confidence: 0.9,
            reasoning: reasoning.into(),
            issues: vec![],
            suggestions: vec![],
            quality_score,
        }
    }

    /// Create a failed evaluation.
    pub fn failure(reasoning: impl Into<String>, issues: Vec<String>) -> Self {
        Self {
            success: false,
            confidence: 0.9,
            reasoning: reasoning.into(),
            issues,
            suggestions: vec![],
            quality_score: 0,
        }
    }
}

/// Trait for success evaluators.
#[async_trait]
pub trait SuccessEvaluator: Send + Sync {
    /// Evaluate whether a job was completed successfully.
    async fn evaluate(
        &self,
        job: &JobContext,
        actions: &[ActionRecord],
        output: Option<&str>,
    ) -> Result<EvaluationResult, EvaluationError>;
}

/// Rule-based success evaluator.
pub struct RuleBasedEvaluator {
    /// Minimum success rate for actions.
    min_action_success_rate: f64,
    /// Maximum allowed failures.
    max_failures: u32,
}

impl RuleBasedEvaluator {
    /// Create a new rule-based evaluator.
    pub fn new() -> Self {
        Self {
            min_action_success_rate: 0.8,
            max_failures: 3,
        }
    }

    /// Set minimum action success rate.
    #[allow(dead_code)] // Public API for configuring evaluation threshold
    pub fn with_min_success_rate(mut self, rate: f64) -> Self {
        self.min_action_success_rate = rate;
        self
    }

    /// Set maximum failures.
    #[allow(dead_code)] // Public API for configuring failure tolerance
    pub fn with_max_failures(mut self, max: u32) -> Self {
        self.max_failures = max;
        self
    }
}

impl Default for RuleBasedEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SuccessEvaluator for RuleBasedEvaluator {
    async fn evaluate(
        &self,
        job: &JobContext,
        actions: &[ActionRecord],
        _output: Option<&str>,
    ) -> Result<EvaluationResult, EvaluationError> {
        let mut issues = Vec::new();

        // Check if there were any actions
        if actions.is_empty() {
            return Ok(EvaluationResult::failure(
                "No actions were taken",
                vec!["No actions recorded".to_string()],
            ));
        }

        // Calculate action success rate
        let successful = actions.iter().filter(|a| a.success).count();
        let total = actions.len();
        let success_rate = successful as f64 / total as f64;

        if success_rate < self.min_action_success_rate {
            issues.push(format!(
                "Action success rate {:.1}% below threshold {:.1}%",
                success_rate * 100.0,
                self.min_action_success_rate * 100.0
            ));
        }

        // Count failures
        let failures = actions.iter().filter(|a| !a.success).count() as u32;
        if failures > self.max_failures {
            issues.push(format!(
                "Too many failures: {} (max {})",
                failures, self.max_failures
            ));
        }

        // Check for critical errors
        for action in actions.iter().filter(|a| !a.success) {
            if let Some(ref error) = action.error
                && (error.to_lowercase().contains("critical")
                    || error.to_lowercase().contains("fatal"))
            {
                issues.push(format!("Critical error in {}: {}", action.tool_name, error));
            }
        }

        // Check job state
        if job.state != crate::context::JobState::Completed
            && job.state != crate::context::JobState::Submitted
        {
            issues.push(format!("Job not in completed state: {:?}", job.state));
        }

        // Calculate quality score
        let quality_score = if issues.is_empty() {
            let base_score = (success_rate * 80.0) as u32;
            let completion_bonus = if job.state == crate::context::JobState::Completed {
                20
            } else {
                0
            };
            (base_score + completion_bonus).min(100)
        } else {
            ((success_rate * 50.0) as u32).min(50)
        };

        if issues.is_empty() {
            Ok(EvaluationResult::success(
                format!(
                    "Job completed successfully with {}/{} actions succeeding ({:.1}%)",
                    successful,
                    total,
                    success_rate * 100.0
                ),
                quality_score,
            ))
        } else {
            Ok(EvaluationResult {
                success: false,
                confidence: 0.85,
                reasoning: format!("Job had {} issues", issues.len()),
                issues,
                suggestions: vec![
                    "Review failed actions for common patterns".to_string(),
                    "Consider adjusting retry logic".to_string(),
                ],
                quality_score,
            })
        }
    }
}

/// LLM-based success evaluator for more nuanced evaluation.
pub struct LlmEvaluator {
    llm: Arc<dyn LlmProvider>,
}

impl LlmEvaluator {
    /// Create a new LLM-based evaluator.
    #[allow(dead_code)] // Public API for LLM-based evaluation
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl SuccessEvaluator for LlmEvaluator {
    async fn evaluate(
        &self,
        job: &JobContext,
        actions: &[ActionRecord],
        output: Option<&str>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Build evaluation prompt
        let actions_summary: Vec<String> = actions
            .iter()
            .map(|a| {
                format!(
                    "- {}: {} ({})",
                    a.tool_name,
                    if a.success { "success" } else { "failed" },
                    a.error.as_deref().unwrap_or("ok")
                )
            })
            .collect();

        let prompt = format!(
            r#"Evaluate if this job was completed successfully.

Job: {}
Description: {}
State: {:?}

Actions taken:
{}

{}

Respond in JSON format:
{{
    "success": true/false,
    "confidence": 0.0-1.0,
    "reasoning": "...",
    "issues": ["..."],
    "suggestions": ["..."],
    "quality_score": 0-100
}}"#,
            job.title,
            job.description,
            job.state,
            actions_summary.join("\n"),
            output
                .map(|o| format!("Output:\n{}", o))
                .unwrap_or_default()
        );

        let request =
            crate::llm::CompletionRequest::new(vec![crate::llm::ChatMessage::user(prompt)])
                .with_max_tokens(1024)
                .with_temperature(0.1);

        let response = self
            .llm
            .complete(request)
            .await
            .map_err(|e| EvaluationError::Failed {
                job_id: job.job_id,
                reason: e.to_string(),
            })?;

        // Parse the response
        let result: EvaluationResult =
            serde_json::from_str(&response.content).map_err(|e| EvaluationError::Failed {
                job_id: job.job_id,
                reason: format!("Failed to parse LLM evaluation: {}", e),
            })?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::JobContext;

    #[tokio::test]
    async fn test_rule_based_evaluator_success() {
        let evaluator = RuleBasedEvaluator::new();

        let mut job = JobContext::new("Test", "Test job");
        job.transition_to(crate::context::JobState::InProgress, None)
            .unwrap();
        job.transition_to(crate::context::JobState::Completed, None)
            .unwrap();

        let actions = vec![
            create_action(true),
            create_action(true),
            create_action(true),
        ];

        let result = evaluator.evaluate(&job, &actions, None).await.unwrap();
        assert!(result.success);
        assert!(result.quality_score > 80);
    }

    #[tokio::test]
    async fn test_rule_based_evaluator_failure() {
        let evaluator = RuleBasedEvaluator::new().with_max_failures(1);

        let job = JobContext::new("Test", "Test job");

        let actions = vec![
            create_action(true),
            create_action(false),
            create_action(false),
        ];

        let result = evaluator.evaluate(&job, &actions, None).await.unwrap();
        assert!(!result.success);
        assert!(!result.issues.is_empty());
    }

    fn create_action(success: bool) -> ActionRecord {
        let mut action = ActionRecord::new(0, "test", serde_json::json!({}));
        if success {
            action = action.succeed(
                None,
                serde_json::json!({}),
                std::time::Duration::from_secs(1),
            );
        } else {
            action = action.fail("Test error", std::time::Duration::from_secs(1));
        }
        action
    }
}
