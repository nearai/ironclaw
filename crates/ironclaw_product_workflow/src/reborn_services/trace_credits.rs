//! Contributor-local Trace Commons credit projection for WebChat v2.
//!
//! The WebUI surface is read-only: it reports the caller-scoped local
//! view of Trace Commons credit as of the last credit sync. The
//! authoritative ledger lives server-side; nothing here mutates trace
//! state or accepts a scope from request input — the trace scope is
//! always derived from the authenticated caller's tenant + user id (see
//! [`ironclaw_reborn_traces::contribution::trace_scope_key`]).

use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_reborn_traces::contribution::{
    AccountLoginLinkError, authorize_manual_review_hold_for_scope, fetch_account_traces,
    mint_account_login_link, read_trace_policy_for_scope, resolve_trace_credentials,
    scoped_credit_view,
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

/// One submitted trace record as returned by the Trace Commons server.
/// Carries only the fields the UI needs; unknown server fields are ignored.
#[derive(Debug, Clone, Serialize)]
pub struct RebornAccountTrace {
    pub submission_id: String,
    pub status: String,
    pub pending_credit: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_credit: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_at: Option<String>,
}

/// Read-only list of the caller's submitted Trace Commons traces.
///
/// `enrolled` mirrors the caller's contribution-policy enrollment status
/// (same semantics as [`RebornTraceCreditsResponse::enrolled`]).
/// `traces` is the server-returned list in reverse-chronological order;
/// an empty list is normal for an enrolled user who has not yet submitted
/// any traces.
#[derive(Debug, Clone, Serialize)]
pub struct RebornAccountTracesResponse {
    pub enrolled: bool,
    pub traces: Vec<RebornAccountTrace>,
}

/// Result of authorizing a held trace for submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornTraceHoldAuthorizeResponse {
    /// True when a held trace matching the submission id was found and
    /// authorized for submission; false when there was no such held trace
    /// (already authorized, already submitted, or never held).
    pub authorized: bool,
}

/// One-time Trace Commons browser login link, minted for the authenticated
/// caller. SECURITY: the `url` is a code-bearing account-access credential.
/// It is delivered ONLY over the authenticated WebUI response to the caller's
/// own browser — it must never be logged, persisted, or placed on any
/// model-visible surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornAccountLoginLinkResponse {
    /// Whether a link was minted. `false` with `enrolled: false` is the
    /// unenrolled zero-state, not an error.
    pub minted: bool,
    pub enrolled: bool,
    /// The one-time login URL (present iff `minted`). Expires shortly and is
    /// single-use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Typed failure for [`account_login_link_for_user`]. Mirrors
/// [`AccountTracesError`]: the operation is named and the cause chain is
/// preserved for a diagnosable (but sanitized at the wire) 500.
#[derive(Debug, thiserror::Error)]
pub(super) enum AccountLoginLinkMintError {
    /// Minting the link failed (policy read, claim mint, transport, issuer).
    #[error("mint account login link: {0}")]
    Mint(String),
}

/// Mint a one-time Trace Commons browser login link for the caller.
///
/// Uses the crate-local pinned direct client (no host-egress sink) — the same
/// network lane as [`account_traces_for_user`]. An unenrolled caller gets the
/// zero-state (`minted: false, enrolled: false`), never an error; hosted
/// multi-tenant users have no host-file access, so the authenticated response
/// is the only delivery channel for the link.
pub(super) async fn account_login_link_for_user(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> Result<RebornAccountLoginLinkResponse, AccountLoginLinkMintError> {
    match mint_account_login_link(tenant_id, user_id).await {
        Ok(link) => Ok(RebornAccountLoginLinkResponse {
            minted: true,
            enrolled: true,
            url: Some(link.url),
        }),
        Err(AccountLoginLinkError::NotEnrolled) => Ok(RebornAccountLoginLinkResponse {
            minted: false,
            enrolled: false,
            url: None,
        }),
        Err(error) => Err(AccountLoginLinkMintError::Mint(format!("{error:#}"))),
    }
}

/// Typed failure for [`account_traces_for_user`]. Each variant names the backend
/// operation that failed and carries the full cause chain (`{:#}`) so the WebUI
/// boundary keeps a diagnosable cause instead of an undiscriminated `String`.
#[derive(Debug, thiserror::Error)]
pub(super) enum AccountTracesError {
    /// `resolve_trace_credentials` failed to read local enrollment state.
    #[error("resolve trace credentials: {0}")]
    ResolveCredentials(String),
    /// `fetch_account_traces` failed (transport / server / decode).
    #[error("fetch account traces: {0}")]
    Fetch(String),
}

/// Fetch the caller-scoped submitted traces from the Trace Commons server.
///
/// Uses the crate-local hardened reqwest path (no host-egress sink) — the same
/// network lane as the rest of the WebUI / facade surface. The `enrolled` flag
/// is set from `resolve_trace_credentials`; a user who is not enrolled gets the
/// unenrolled zero-state (`enrolled: false`, empty list) rather than an error.
/// Transport failures surface as a typed `Err` (the operation is named and the
/// cause chain preserved) so the caller can return a sanitized, diagnosable 500.
pub(super) async fn account_traces_for_user(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> Result<RebornAccountTracesResponse, AccountTracesError> {
    // Identity stays typed inside this crate; only cross to `&str` at the
    // `ironclaw_reborn_traces` boundary, which is stringly-typed.
    let enrolled = resolve_trace_credentials(tenant_id, user_id)
        .map_err(|e| AccountTracesError::ResolveCredentials(format!("{e:#}")))?
        .is_some();
    if !enrolled {
        return Ok(RebornAccountTracesResponse {
            enrolled: false,
            traces: vec![],
        });
    }
    // `None` is not unbounded: `fetch_account_traces` defaults a missing limit to
    // ACCOUNT_TRACES_DEFAULT_LIMIT (200), clamps any explicit value to
    // [1, ACCOUNT_TRACES_MAX_LIMIT], and caps the buffered response bytes — so
    // the initial settings-page slice can never scale with total account age.
    let items = fetch_account_traces(tenant_id.as_str(), user_id.as_str(), None)
        .await
        .map_err(|e| AccountTracesError::Fetch(format!("{e:#}")))?;
    let traces = items
        .into_iter()
        .map(|item| RebornAccountTrace {
            submission_id: item.submission_id,
            status: item.status,
            pending_credit: item.credit_points_pending,
            final_credit: item.credit_points_final,
            received_at: item.received_at,
        })
        .collect();
    Ok(RebornAccountTracesResponse {
        enrolled: true,
        traces,
    })
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
