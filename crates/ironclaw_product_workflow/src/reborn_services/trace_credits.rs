//! Contributor-local Trace Commons credit projection for WebChat v2.
//!
//! The WebUI surface is read-only: it reports the caller-scoped local
//! view of Trace Commons credit as of the last credit sync. The
//! authoritative ledger lives server-side; nothing here mutates trace
//! state or accepts a scope from request input — the trace scope is
//! always derived from the authenticated caller's tenant + user id (see
//! [`ironclaw_reborn_traces::contribution::trace_scope_key`]).

use chrono::{DateTime, Utc};
use ironclaw_reborn_traces::contribution::{
    authorize_manual_review_hold_for_scope, read_trace_policy_for_scope, scoped_credit_view,
};
use serde::{Deserialize, Serialize};

/// Server-authoritative framing returned with every credits response.
/// Mirrors the note `builtin.trace_commons.credits` reports.
pub(super) const TRACE_CREDITS_NOTE: &str = "Local view as of last sync; final credit can change \
     after privacy review, replay/eval, duplicate checks, and downstream utility scoring. \
     The authoritative ledger is server-side.";

/// Read-only Trace Commons credit summary scoped to one user.
///
/// All aggregates are the contributor-local view as of the last credit
/// sync (see [`TRACE_CREDITS_NOTE`]). A user with no local Trace
/// Commons state gets the unenrolled zero-state, never an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornTraceCreditsResponse {
    /// Whether the caller's standing trace-contribution policy is enabled.
    pub enrolled: bool,
    pub pending_credit: f32,
    pub final_credit: f32,
    pub delayed_credit_delta: f32,
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_accepted: u32,
    pub submissions_revoked: u32,
    pub submissions_expired: u32,
    pub credit_events_total: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submission_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_explanations: Vec<String>,
    /// Count of traces held awaiting the caller's manual-review authorization
    /// (e.g. High residual-PII-risk). These are retained, not submitted.
    #[serde(default)]
    pub manual_review_hold_count: u32,
    /// The held traces awaiting authorization. Sanitized: submission id and a
    /// safe hold reason only — never raw trace content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<RebornTraceHold>,
    /// Server-authoritative framing — always [`TRACE_CREDITS_NOTE`].
    pub note: String,
}

/// One trace held awaiting the caller's manual-review authorization. Carries
/// only the submission id (to authorize against) and a sanitized hold reason;
/// no raw trace payload is ever exposed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornTraceHold {
    pub submission_id: String,
    pub reason: String,
}

/// Result of authorizing a held trace for submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornTraceHoldAuthorizeResponse {
    /// True when a held trace matching the submission id was found and
    /// authorized for submission; false when there was no such held trace
    /// (already authorized, already submitted, or never held).
    pub authorized: bool,
}

/// Authorize the caller-scoped held manual-review trace for submission.
///
/// The trace `scope` is the caller's tenant-scoped key (see
/// [`ironclaw_reborn_traces::contribution::trace_scope_key`]); the submission id
/// is never an authority to cross scopes. Returns whether a matching
/// `ManualReview` hold was found and promoted.
pub(super) fn authorize_trace_hold_for_user(
    scope: &str,
    submission_id: uuid::Uuid,
) -> Result<bool, String> {
    authorize_manual_review_hold_for_scope(Some(scope), submission_id)
        .map_err(|error| error.to_string())
}

/// Build the caller-scoped local Trace Commons credit view.
///
/// A MISSING local state is the normal "not enrolled / nothing submitted yet"
/// state and is softened to the default/empty value by the underlying
/// `read_*_for_scope` helpers (they return `Ok` when the file is absent). A
/// genuine read/parse failure (unreadable or corrupt local state) is NOT masked
/// as the zero-state response — it propagates so the caller can surface it
/// instead of telling an enrolled user they have nothing.
pub(super) fn local_trace_credits_for_user(
    scope: &str,
) -> Result<RebornTraceCreditsResponse, String> {
    // `{:#}` preserves the underlying error chain so the caller's sanitized 500
    // still leaves a useful server-side trail.
    let enrolled = read_trace_policy_for_scope(Some(scope))
        .map_err(|e| format!("{e:#}"))?
        .enabled;
    // The aggregate report + manual-review holds come from `scoped_credit_view`,
    // which memoizes the full-history read/aggregate by the on-disk input
    // signature — so steady polling on an unchanged history is a couple of
    // `stat`s, not an O(total submissions) rebuild per request.
    let view = scoped_credit_view(scope).map_err(|e| format!("{e:#}"))?;
    let report = view.report;
    let holds: Vec<RebornTraceHold> = view
        .manual_review_holds
        .into_iter()
        .map(|hold| RebornTraceHold {
            submission_id: hold.submission_id.to_string(),
            reason: hold.reason,
        })
        .collect();
    Ok(RebornTraceCreditsResponse {
        enrolled,
        manual_review_hold_count: holds.len() as u32,
        holds,
        pending_credit: report.pending_credit,
        final_credit: report.final_credit,
        delayed_credit_delta: report.delayed_credit_delta,
        submissions_total: report.submissions_total,
        submissions_submitted: report.submissions_submitted,
        submissions_accepted: report.submissions_accepted,
        submissions_revoked: report.submissions_revoked,
        submissions_expired: report.submissions_expired,
        credit_events_total: report.credit_events_total,
        last_submission_at: report.last_submission_at,
        last_credit_sync_at: report.last_credit_sync_at,
        recent_explanations: report.explanation_lines,
        note: TRACE_CREDITS_NOTE.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_reborn_traces::contribution::{
        StandingTraceContributionPolicy, trace_contribution_dir_for_scope,
        write_trace_policy_for_scope,
    };

    /// Removes the per-test trace scope directory on drop so failed
    /// assertions don't leak state into the shared base dir.
    struct ScopeCleanup(String);

    impl Drop for ScopeCleanup {
        fn drop(&mut self) {
            let dir = trace_contribution_dir_for_scope(Some(self.0.as_str()));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn unique_scope(label: &str) -> String {
        format!("{label}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn fresh_scope_yields_unenrolled_zero_state() {
        let scope = unique_scope("pw-trace-credits-fresh");
        let response = local_trace_credits_for_user(&scope).expect("local credits read");
        assert!(!response.enrolled);
        assert_eq!(response.submissions_total, 0);
        assert_eq!(response.submissions_submitted, 0);
        assert_eq!(response.credit_events_total, 0);
        assert_eq!(response.pending_credit, 0.0);
        assert_eq!(response.final_credit, 0.0);
        assert_eq!(response.delayed_credit_delta, 0.0);
        assert!(response.last_submission_at.is_none());
        assert!(response.last_credit_sync_at.is_none());
        assert_eq!(response.manual_review_hold_count, 0);
        assert!(response.holds.is_empty());
        // `trace_credit_report` always emits its summary lines, even
        // for an empty record set.
        assert!(
            response
                .recent_explanations
                .iter()
                .any(|line| line.contains("0 submitted trace(s)")),
            "zero-state explanations should describe the empty record set: {:?}",
            response.recent_explanations
        );
        assert_eq!(response.note, TRACE_CREDITS_NOTE);
    }

    #[test]
    fn enabled_policy_reports_enrolled_with_zero_aggregates() {
        let scope = unique_scope("pw-trace-credits-enrolled");
        let _cleanup = ScopeCleanup(scope.clone());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ..StandingTraceContributionPolicy::default()
        };
        write_trace_policy_for_scope(Some(scope.as_str()), &policy).expect("write policy");

        let response = local_trace_credits_for_user(&scope).expect("local credits read");
        assert!(response.enrolled);
        assert_eq!(response.submissions_total, 0);
        assert_eq!(response.pending_credit, 0.0);
        assert_eq!(response.note, TRACE_CREDITS_NOTE);
    }
}
