//! Success evaluation for jobs.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context::{ActionRecord, JobContext};
use crate::error::EvaluationError;

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
#[cfg(test)]
pub struct RuleBasedEvaluator {
    /// Minimum success rate for actions.
    min_action_success_rate: f64,
    /// Maximum allowed failures.
    max_failures: u32,
}

#[cfg(test)]
impl RuleBasedEvaluator {
    /// Create a new rule-based evaluator.
    pub fn new() -> Self {
        Self {
            min_action_success_rate: 0.8,
            max_failures: 3,
        }
    }

    /// Set minimum action success rate.
    pub fn with_min_success_rate(mut self, rate: f64) -> Self {
        self.min_action_success_rate = rate;
        self
    }

    /// Set maximum failures.
    pub fn with_max_failures(mut self, max: u32) -> Self {
        self.max_failures = max;
        self
    }
}

#[cfg(test)]
impl Default for RuleBasedEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
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
        create_action_with_error(success, "Test error")
    }

    fn create_action_with_error(success: bool, error_msg: &str) -> ActionRecord {
        let mut action = ActionRecord::new(0, "test", serde_json::json!({}));
        if success {
            action = action.succeed(
                None,
                serde_json::json!({}),
                std::time::Duration::from_secs(1),
            );
        } else {
            action = action.fail(error_msg, std::time::Duration::from_secs(1));
        }
        action
    }

    fn completed_job(title: &str) -> JobContext {
        let mut job = JobContext::new(title, "test job");
        job.transition_to(crate::context::JobState::InProgress, None)
            .unwrap();
        job.transition_to(crate::context::JobState::Completed, None)
            .unwrap();
        job
    }

    // --- EvaluationResult construction ---

    #[test]
    fn test_evaluation_result_success_defaults() {
        let result = EvaluationResult::success("all good", 85);
        assert!(result.success);
        assert_eq!(result.confidence, 0.9);
        assert_eq!(result.reasoning, "all good");
        assert!(result.issues.is_empty());
        assert!(result.suggestions.is_empty());
        assert_eq!(result.quality_score, 85);
    }

    #[test]
    fn test_evaluation_result_failure_defaults() {
        let issues = vec!["bad thing".to_string(), "worse thing".to_string()];
        let result = EvaluationResult::failure("went wrong", issues.clone());
        assert!(!result.success);
        assert_eq!(result.confidence, 0.9);
        assert_eq!(result.reasoning, "went wrong");
        assert_eq!(result.issues, issues);
        assert_eq!(result.quality_score, 0);
    }

    #[test]
    fn test_evaluation_result_serde_roundtrip() {
        let result = EvaluationResult {
            success: true,
            confidence: 0.75,
            reasoning: "looks fine".to_string(),
            issues: vec!["minor".to_string()],
            suggestions: vec!["try harder".to_string()],
            quality_score: 60,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: EvaluationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.success, result.success);
        assert_eq!(deserialized.confidence, result.confidence);
        assert_eq!(deserialized.reasoning, result.reasoning);
        assert_eq!(deserialized.issues, result.issues);
        assert_eq!(deserialized.suggestions, result.suggestions);
        assert_eq!(deserialized.quality_score, result.quality_score);
    }

    // --- RuleBasedEvaluator builder ---

    #[test]
    fn test_rule_based_evaluator_default() {
        let eval = RuleBasedEvaluator::default();
        assert_eq!(eval.min_action_success_rate, 0.8);
        assert_eq!(eval.max_failures, 3);
    }

    #[test]
    fn test_rule_based_evaluator_builder_methods() {
        let eval = RuleBasedEvaluator::new()
            .with_min_success_rate(0.5)
            .with_max_failures(10);
        assert_eq!(eval.min_action_success_rate, 0.5);
        assert_eq!(eval.max_failures, 10);
    }

    // --- RuleBasedEvaluator::evaluate edge cases ---

    #[tokio::test]
    async fn test_empty_actions_fails() {
        let eval = RuleBasedEvaluator::new();
        let job = completed_job("empty");
        let result = eval.evaluate(&job, &[], None).await.unwrap();
        assert!(!result.success);
        assert!(result.issues.iter().any(|i| i.contains("No actions")));
    }

    #[tokio::test]
    async fn test_all_actions_succeed_completed_job_gets_100() {
        let eval = RuleBasedEvaluator::new();
        let job = completed_job("perfect");
        let actions = vec![
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
        ];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(result.success);
        // 100% success rate -> base 80, completion bonus 20 -> 100
        assert_eq!(result.quality_score, 100);
    }

    #[tokio::test]
    async fn test_quality_score_no_completion_bonus_for_pending_job() {
        // Even if all actions succeed, a non-completed job gets flagged
        let eval = RuleBasedEvaluator::new();
        let job = JobContext::new("pending", "still pending");
        let actions = vec![create_action(true)];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        // Job not in completed state => issues present
        assert!(!result.success);
        assert!(
            result
                .issues
                .iter()
                .any(|i| i.contains("not in completed state"))
        );
    }

    #[tokio::test]
    async fn test_submitted_state_counts_as_completed() {
        let eval = RuleBasedEvaluator::new();
        let mut job = JobContext::new("submitted", "test");
        job.transition_to(crate::context::JobState::InProgress, None)
            .unwrap();
        job.transition_to(crate::context::JobState::Completed, None)
            .unwrap();
        job.transition_to(crate::context::JobState::Submitted, None)
            .unwrap();
        let actions = vec![create_action(true)];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        // Submitted is treated like completed for state check (no issue),
        // but completion bonus only applies for Completed state
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_success_rate_below_threshold_fails() {
        let eval = RuleBasedEvaluator::new().with_min_success_rate(0.9);
        let job = completed_job("threshold");
        // 4 out of 5 = 80%, below 90% threshold
        let actions = vec![
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(false),
        ];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(!result.success);
        assert!(
            result
                .issues
                .iter()
                .any(|i| i.contains("success rate") && i.contains("below threshold"))
        );
    }

    #[tokio::test]
    async fn test_too_many_failures_flagged() {
        let eval = RuleBasedEvaluator::new().with_max_failures(1);
        let job = completed_job("failures");
        // 8 successes, 2 failures: rate is 80% (passes default 0.8) but failures > max 1
        let actions = vec![
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(false),
            create_action(false),
        ];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(!result.success);
        assert!(
            result
                .issues
                .iter()
                .any(|i| i.contains("Too many failures"))
        );
    }

    #[tokio::test]
    async fn test_critical_error_detected() {
        let eval = RuleBasedEvaluator::new().with_max_failures(10);
        let job = completed_job("critical");
        let actions = vec![
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action_with_error(false, "A CRITICAL system failure occurred"),
        ];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(!result.success);
        assert!(result.issues.iter().any(|i| i.contains("Critical error")));
    }

    #[tokio::test]
    async fn test_fatal_error_detected() {
        let eval = RuleBasedEvaluator::new().with_max_failures(10);
        let job = completed_job("fatal");
        let actions = vec![
            create_action(true),
            create_action(true),
            create_action(true),
            create_action(true),
            create_action_with_error(false, "Fatal: disk full"),
        ];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(result.issues.iter().any(|i| i.contains("Critical error")));
    }

    #[tokio::test]
    async fn test_quality_score_capped_at_50_with_issues() {
        let eval = RuleBasedEvaluator::new()
            .with_min_success_rate(0.0)
            .with_max_failures(100);
        // Job not completed => issues present, quality capped
        let job = JobContext::new("capped", "test");
        let actions = vec![create_action(true)];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(!result.success);
        assert!(result.quality_score <= 50);
    }

    #[tokio::test]
    async fn test_failed_result_includes_suggestions() {
        let eval = RuleBasedEvaluator::new().with_max_failures(0);
        let job = completed_job("suggestions");
        let actions = vec![create_action(false)];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(!result.success);
        assert!(!result.suggestions.is_empty());
        assert_eq!(result.confidence, 0.85);
    }

    #[tokio::test]
    async fn test_single_successful_action_completed_job() {
        let eval = RuleBasedEvaluator::new();
        let job = completed_job("single");
        let actions = vec![create_action(true)];
        let result = eval.evaluate(&job, &actions, None).await.unwrap();
        assert!(result.success);
        // 100% rate -> base 80, + 20 completion = 100
        assert_eq!(result.quality_score, 100);
        assert!(result.reasoning.contains("1/1"));
    }
}
