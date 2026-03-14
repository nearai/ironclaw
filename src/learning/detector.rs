//! Detects interactions worth synthesizing into skills.

use crate::learning::candidate::DetectionReason;

/// Configuration for pattern detection thresholds.
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    /// Minimum tool calls for a "complex tool chain" detection.
    pub min_tool_calls: usize,
    /// Minimum unique tools for a "novel combination" detection.
    pub min_unique_tools: usize,
    /// Minimum quality score for "high quality completion" detection.
    pub min_quality_score: u32,
    /// Minimum turn count for any detection (except user-requested).
    pub min_turns: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            min_tool_calls: 3,
            min_unique_tools: 2,
            min_quality_score: 75,
            min_turns: 2,
        }
    }
}

impl DetectorConfig {
    /// Create from `LearningConfig`.
    pub fn from_learning_config(config: &crate::config::LearningConfig) -> Self {
        Self {
            min_tool_calls: config.min_tool_calls,
            min_unique_tools: config.min_unique_tools,
            min_quality_score: config.min_quality_score,
            min_turns: config.min_turns,
        }
    }
}

/// Evaluates whether a completed interaction is worth synthesizing.
pub struct PatternDetector {
    config: DetectorConfig,
}

impl PatternDetector {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }

    /// Evaluate an interaction. Returns `Some(reason)` if synthesis-worthy.
    #[must_use]
    pub fn evaluate(
        &self,
        turn_count: usize,
        tools_used: &[String],
        quality_score: u32,
        user_requested: bool,
    ) -> Option<DetectionReason> {
        // User-requested always passes
        if user_requested {
            return Some(DetectionReason::UserRequested);
        }

        // Must meet minimum turn threshold
        if turn_count < self.config.min_turns {
            return None;
        }

        // BTreeSet for deterministic ordering in detection results
        let unique_tools: std::collections::BTreeSet<&String> = tools_used.iter().collect();

        // Check for novel tool combination FIRST (narrower match — would be
        // swallowed by ComplexToolChain if checked later)
        if unique_tools.len() >= self.config.min_unique_tools
            && tools_used.len() < self.config.min_tool_calls
            && quality_score >= self.config.min_quality_score
        {
            return Some(DetectionReason::NovelToolCombination {
                tools: unique_tools.into_iter().cloned().collect(),
            });
        }

        // Check for complex tool chain (many tool calls)
        if tools_used.len() >= self.config.min_tool_calls
            && quality_score >= self.config.min_quality_score
        {
            return Some(DetectionReason::ComplexToolChain {
                step_count: tools_used.len(),
            });
        }

        // Check for high quality completion on non-trivial task
        if quality_score >= 90 && tools_used.len() >= 2 {
            return Some(DetectionReason::HighQualityCompletion {
                score: quality_score,
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complex_tool_chain_detected() {
        let detector = PatternDetector::new(DetectorConfig::default());
        let tools = vec![
            "shell".into(),
            "http".into(),
            "write_file".into(),
            "shell".into(),
        ];
        let result = detector.evaluate(4, &tools, 80, false);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            DetectionReason::ComplexToolChain { step_count: 4 }
        ));
    }

    #[test]
    fn test_simple_interaction_not_detected() {
        let detector = PatternDetector::new(DetectorConfig::default());
        let tools = vec!["echo".into()];
        let result = detector.evaluate(1, &tools, 50, false);
        assert!(result.is_none());
    }

    #[test]
    fn test_user_requested_always_detected() {
        let detector = PatternDetector::new(DetectorConfig::default());
        let tools = vec!["echo".into()];
        let result = detector.evaluate(1, &tools, 50, true);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), DetectionReason::UserRequested));
    }

    #[test]
    fn test_below_quality_threshold_not_detected() {
        let detector = PatternDetector::new(DetectorConfig::default());
        let tools = vec!["shell".into(), "http".into(), "write_file".into()];
        let result = detector.evaluate(3, &tools, 50, false); // score 50 < 75
        assert!(result.is_none());
    }

    #[test]
    fn test_below_turn_threshold_not_detected() {
        let detector = PatternDetector::new(DetectorConfig::default());
        let tools = vec!["shell".into(), "http".into(), "write_file".into()];
        let result = detector.evaluate(1, &tools, 80, false); // 1 turn < 2 min
        assert!(result.is_none());
    }

    #[test]
    fn test_novel_tool_combination() {
        let detector = PatternDetector::new(DetectorConfig::default());
        let tools = vec!["shell".into(), "http".into()]; // 2 unique, meets threshold
        let result = detector.evaluate(3, &tools, 80, false);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            DetectionReason::NovelToolCombination { .. }
        ));
    }

    #[test]
    fn test_high_quality_completion() {
        let detector = PatternDetector::new(DetectorConfig::default());
        // 2 tools < min_tool_calls(3), but 2 unique >= min_unique_tools(2)
        // → NovelToolCombination fires first (narrower match)
        let tools = vec!["shell".into(), "http".into()];
        let result = detector.evaluate(3, &tools, 95, false);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            DetectionReason::NovelToolCombination { .. }
        ));
    }

    #[test]
    fn test_high_quality_completion_single_unique_tool() {
        let detector = PatternDetector::new(DetectorConfig::default());
        // 1 unique tool < min_unique_tools(2), not enough for NovelToolCombination
        // 2 calls < min_tool_calls(3), not enough for ComplexToolChain
        // But quality >= 90 and tools.len() >= 2 → HighQualityCompletion
        let tools = vec!["shell".into(), "shell".into()];
        let result = detector.evaluate(3, &tools, 95, false);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            DetectionReason::HighQualityCompletion { score: 95 }
        ));
    }

    #[test]
    fn test_custom_config() {
        let config = DetectorConfig {
            min_tool_calls: 5,
            min_unique_tools: 3,
            min_quality_score: 90,
            min_turns: 3,
        };
        let detector = PatternDetector::new(config);
        let tools = vec!["shell".into(), "http".into(), "write_file".into()];
        // 3 tool calls < 5 min, score 80 < 90
        let result = detector.evaluate(3, &tools, 80, false);
        assert!(result.is_none());
    }
}
