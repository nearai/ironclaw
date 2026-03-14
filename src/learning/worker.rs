//! Background learning worker.
//!
//! Receives `LearningEvent`s via a bounded mpsc channel, evaluates them
//! through `PatternDetector`, synthesizes skills via LLM, validates,
//! and records in the audit log.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::config::LearningConfig;
use crate::db::LearningStore;
use crate::learning::LearningEvent;
use crate::learning::candidate::SynthesisCandidate;
use crate::learning::detector::{DetectorConfig, PatternDetector};
use crate::learning::synthesizer::SkillSynthesizer;
use crate::learning::validator::SkillValidator;

/// Compute a heuristic quality score from turn metrics.
///
/// Uses a simple formula: base score from tool success indicators,
/// adjusted by turn count and unique tool diversity.
/// Range: 0-100.
pub fn heuristic_quality_score(tools_used: &[String], turn_count: usize, had_errors: bool) -> u32 {
    let unique_tools: std::collections::HashSet<&String> = tools_used.iter().collect();
    let unique_count = unique_tools.len();

    // Base: 50 if no errors, 20 if errors
    let base = if had_errors { 20u32 } else { 50 };

    // Bonus for tool diversity (up to +20)
    let diversity_bonus = (unique_count as u32).min(4) * 5;

    // Bonus for multi-turn interactions (up to +20)
    let turn_bonus = (turn_count as u32).min(4) * 5;

    // Bonus for tool usage volume (up to +10)
    let volume_bonus = (tools_used.len() as u32).min(5) * 2;

    (base + diversity_bonus + turn_bonus + volume_bonus).min(100)
}

/// Spawn the background learning worker as a tokio task.
///
/// Returns the `Sender` half — dispatch `LearningEvent`s into it.
/// The worker runs until the sender is dropped.
pub fn spawn_learning_worker(
    config: LearningConfig,
    synthesizer: Arc<dyn SkillSynthesizer>,
    store: Arc<dyn LearningStore>,
) -> mpsc::Sender<LearningEvent> {
    let (tx, mut rx) = mpsc::channel::<LearningEvent>(32);

    let detector = PatternDetector::new(DetectorConfig::from_learning_config(&config));
    let validator = SkillValidator::new().with_max_size(config.max_skill_size);

    tokio::spawn(async move {
        tracing::info!("Learning background worker started");

        while let Some(event) = rx.recv().await {
            // Evaluate whether this interaction is synthesis-worthy
            let detection = detector.evaluate(
                event.turn_count,
                &event.tools_used,
                event.quality_score,
                event.user_requested_synthesis,
            );

            let Some(reason) = detection else {
                continue;
            };

            // Check skill count limit before synthesizing
            let existing_count = store
                .list_synthesized_skills(&event.user_id, &event.agent_id, None)
                .await
                .map(|rows| rows.len())
                .unwrap_or(0);
            if existing_count >= config.max_skills_per_user {
                tracing::debug!(
                    user_id = %event.user_id,
                    count = existing_count,
                    limit = config.max_skills_per_user,
                    "Learning: skill limit reached, skipping synthesis"
                );
                continue;
            }

            tracing::info!(
                user_id = %event.user_id,
                reason = ?reason,
                "Learning: synthesis candidate detected"
            );

            // Build candidate
            let candidate = SynthesisCandidate {
                conversation_id: event.conversation_id,
                user_id: event.user_id.clone(),
                task_summary: format!(
                    "Interaction with {} tool calls across {} turns",
                    event.tools_used.len(),
                    event.turn_count
                ),
                tools_used: event.tools_used.clone(),
                tool_call_count: event.tools_used.len(),
                turn_count: event.turn_count,
                quality_score: event.quality_score,
                detection_reason: reason,
                completed_at: chrono::Utc::now(),
            };

            // Synthesize via LLM
            let context: Vec<String> = event
                .user_messages
                .iter()
                .map(|m| format!("User message: {}", m))
                .collect();

            let skill_content = match synthesizer.synthesize(&candidate, &context).await {
                Ok(content) => content,
                Err(e) => {
                    tracing::warn!("Learning: synthesis failed: {e}");
                    continue;
                }
            };

            // Validate — discard skills that fail safety checks
            if let Err(e) = validator.validate(&skill_content) {
                tracing::warn!(
                    user_id = %event.user_id,
                    "Learning: skill failed safety validation, discarding: {e}"
                );
                continue;
            }

            let content_hash = format!("{:x}", content_hash(skill_content.as_bytes()));

            // Record in audit log (pending status — user must approve)
            if let Err(e) = store
                .record_synthesized_skill(
                    &event.user_id,
                    &event.agent_id,
                    &format!("auto-{}", &content_hash[..8]),
                    Some(&skill_content),
                    &content_hash,
                    Some(event.conversation_id),
                    "pending",
                    true, // safety_passed — only reached if validation succeeded
                    event.quality_score as i32,
                )
                .await
            {
                tracing::error!("Learning: failed to record skill: {e}");
            } else {
                tracing::info!(
                    user_id = %event.user_id,
                    "Learning: skill synthesized and recorded (pending approval)"
                );
            }
        }

        tracing::info!("Learning background worker stopped");
    });

    tx
}

/// FNV-1a 64-bit hash for content deduplication (not cryptographic).
fn content_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heuristic_quality_score_no_errors() {
        let tools = vec!["shell".into(), "http".into(), "write_file".into()];
        let score = heuristic_quality_score(&tools, 4, false);
        // base 50 + diversity 15 (3 unique * 5) + turn 20 (4 * 5) + volume 6 (3 * 2) = 91
        assert_eq!(score, 91);
    }

    #[test]
    fn test_heuristic_quality_score_with_errors() {
        let tools = vec!["shell".into()];
        let score = heuristic_quality_score(&tools, 1, true);
        // base 20 + diversity 5 (1 * 5) + turn 5 (1 * 5) + volume 2 (1 * 2) = 32
        assert_eq!(score, 32);
    }

    #[test]
    fn test_heuristic_quality_score_capped_at_100() {
        let tools: Vec<String> = (0..10).map(|i| format!("tool_{i}")).collect();
        let score = heuristic_quality_score(&tools, 10, false);
        assert_eq!(score, 100); // would be 50+20+20+20=110, capped at 100
    }

    #[test]
    fn test_heuristic_quality_score_empty() {
        let score = heuristic_quality_score(&[], 0, false);
        // base 50 + diversity 0 + turn 0 + volume 0 = 50
        assert_eq!(score, 50);
    }
}
