//! Contribution value scoring and the credit estimate / summary / report
//! views derived from it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreditEstimate {
    pub submission_score: f32,
    pub credit_points_pending: f32,
    pub scorecard: TraceValueScorecard,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreditSummary {
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_revoked: u32,
    #[serde(default)]
    pub submissions_expired: u32,
    pub pending_credit: f32,
    pub final_credit: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub delayed_credit_delta: f32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub credit_events_total: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_explanations: Vec<String>,
}

pub fn trace_credit_notice_message(summary: &CreditSummary) -> String {
    let mut message = format!(
        "Trace contribution credit update: {} submitted, {} expired ({} total), pending +{:.2}, final confirmed +{:.2}, delayed ledger {:+.2}. Delayed credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.",
        summary.submissions_submitted,
        summary.submissions_expired,
        summary.submissions_total,
        summary.pending_credit,
        summary.final_credit,
        summary.delayed_credit_delta
    );
    if summary.credit_events_total > 0 {
        message.push_str(&format!(
            " {} credit event(s) recorded.",
            summary.credit_events_total
        ));
    }
    if !summary.recent_explanations.is_empty() {
        let explanations = summary
            .recent_explanations
            .iter()
            .take(2)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        message.push_str(" Recent factors: ");
        message.push_str(&explanations);
    }
    message
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditReport {
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_revoked: u32,
    #[serde(default)]
    pub submissions_expired: u32,
    #[serde(default)]
    pub submissions_accepted: u32,
    #[serde(default)]
    pub submissions_quarantined: u32,
    #[serde(default)]
    pub submissions_rejected: u32,
    pub pending_credit: f32,
    pub final_credit: f32,
    #[serde(default)]
    pub credit_events_total: u32,
    #[serde(default)]
    pub delayed_credit_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submission_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation_lines: Vec<String>,
}

pub fn estimate_initial_credit(envelope: &TraceContributionEnvelope) -> CreditEstimate {
    let scorecard = compute_value_scorecard(envelope);
    let submission_score = scorecard.online_score;
    let credit_points_pending = scorecard.credit_points_estimate;
    let explanation = scorecard.explanation.clone();

    CreditEstimate {
        submission_score,
        credit_points_pending,
        scorecard,
        explanation,
    }
}

pub fn compute_value_scorecard(envelope: &TraceContributionEnvelope) -> TraceValueScorecard {
    let schema_validity = if envelope.schema_version == TRACE_CONTRIBUTION_SCHEMA_VERSION {
        1.0
    } else {
        0.0
    };
    let privacy_risk = privacy_risk_score(envelope.privacy.residual_pii_risk);
    let gate = privacy_gate(envelope.privacy.residual_pii_risk);
    let event_count = envelope.events.len() as f32;
    let quality = (event_count / 8.0).clamp(0.15, 1.0);
    let replayability = if envelope.replay.replayable { 1.0 } else { 0.0 };
    let novelty = envelope
        .embedding_analysis
        .as_ref()
        .and_then(|analysis| analysis.novelty_score)
        .unwrap_or_else(|| (event_count / 12.0).clamp(0.15, 0.6))
        .min(0.85);
    let duplicate_penalty = envelope
        .embedding_analysis
        .as_ref()
        .and_then(|analysis| analysis.duplicate_score)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let coverage_bonus = (envelope.replay.required_tools.len() as f32 / 5.0).clamp(0.0, 1.0);
    let failed_or_partial = matches!(
        envelope.outcome.task_success,
        TaskSuccess::Failure | TaskSuccess::Partial
    );
    let difficulty = if failed_or_partial { 0.65 } else { 0.35 };
    let dependability = if envelope.events.is_empty() {
        0.0
    } else if envelope.privacy.redaction_hash.starts_with("sha256:") {
        1.0
    } else {
        0.5
    };
    let user_correction_value = if envelope.outcome.human_correction.is_some()
        || envelope.outcome.user_feedback == UserFeedback::Correction
    {
        1.0
    } else {
        0.0
    };
    let process_eval_value = envelope
        .process_evaluation
        .as_ref()
        .and_then(|labels| labels.overall_score)
        .map(|score| score.clamp(0.0, 1.0));

    let raw = gate
        * schema_validity
        * (0.25 * quality
            + 0.20 * replayability
            + 0.20 * novelty
            + 0.15 * coverage_bonus
            + 0.10 * difficulty
            + 0.10 * user_correction_value)
        - 0.40 * duplicate_penalty
        - 0.60 * privacy_risk;
    let online_score = raw.clamp(0.0, 1.0);
    let credit_points_estimate =
        if matches!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High) {
            0.0
        } else {
            (10.0 * online_score * 100.0).round() / 100.0
        };

    let mut explanation = Vec::new();
    if gate > 0.0 {
        explanation.push(format!(
            "Privacy gate passed with {:?} residual risk.",
            envelope.privacy.residual_pii_risk
        ));
    } else {
        explanation.push("Residual privacy risk is high; credit is held for review.".to_string());
    }
    if envelope.replay.replayable {
        explanation.push("Replay metadata is present.".to_string());
    }
    if !envelope.replay.required_tools.is_empty() {
        explanation.push(format!(
            "Covers {} tool(s).",
            envelope.replay.required_tools.len()
        ));
    }
    if user_correction_value > 0.0 {
        explanation.push("Includes a redacted user correction signal.".to_string());
    }
    if duplicate_penalty > 0.0 {
        explanation.push(format!(
            "Duplicate penalty applied at {:.2}.",
            duplicate_penalty
        ));
    }
    if !envelope.privacy.redaction_counts.is_empty() {
        explanation.push("Deterministic redactions were applied before submission.".to_string());
    }

    TraceValueScorecard {
        schema_validity,
        privacy_risk,
        quality,
        replayability,
        novelty,
        duplicate_penalty,
        coverage_bonus,
        difficulty,
        dependability,
        user_correction_value,
        process_eval_value,
        downstream_utility: None,
        online_score,
        credit_points_estimate,
        explanation,
    }
}

// Below-High residual PII risk is treated as clean for scoring: the
// deterministic redactor has already scrubbed detected PII, and the 0.35
// submission gate leaves no headroom for a partial Medium discount on a
// minimal trace (it would zero an otherwise-valuable trace). Only High risk
// is penalized — `privacy_gate` zeros its score and `trace_autonomous_eligibility`
// holds it for manual review.
pub(crate) fn privacy_gate(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low | ResidualPiiRisk::Medium => 1.0,
        ResidualPiiRisk::High => 0.0,
    }
}

pub(crate) fn privacy_risk_score(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low | ResidualPiiRisk::Medium => 0.0,
        ResidualPiiRisk::High => 1.0,
    }
}
