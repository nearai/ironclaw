//! C-TRACECAP enabler (c): production trace-capture sink on the int-tier lane.
//!
//! Wires the REAL `TraceCaptureTurnEventSink` (via composition's
//! `test_support::trace_capture_turn_event_sink_for_test`, mirroring
//! `build_ironclaw_runtime`'s recipe) into the group's ONE planned runtime with
//! `.with_trace_capture()`, and proves the capture path end-to-end: policy
//! read → transcript capture → redact → score → queue
//! (`ironclaw_traces::contribution`, previously 0% on the lane).
//!
//! Enrollment divergence from the plan (verified infeasible as written): a
//! scripted `builtin.trace_commons.onboard` with `confirmed=true` can NEVER
//! enroll against a canned recording-egress body — the onboarding client
//! cross-checks the server-echoed `device_key_id` against a locally generated
//! ephemeral key (`onboarding/mod.rs:241`), which only a live echo-mock
//! issuer can satisfy (see `trace_commons_dispatch_e2e.rs`). The scenario
//! therefore (1) scripts onboard with `confirmed=false` through real
//! capability dispatch — the consent gate, which doubles as the
//! inert-until-enrolled control — and (2) enrolls by writing the SAME policy
//! state onboard writes (`write_trace_policy_for_scope`, the plan's original
//! fallback). Live-issuer onboarding stays a named follow-on with the other
//! network paths.
//!
//! This binary owns `IRONCLAW_BASE_DIR`: trace policy/queue paths resolve
//! through `ironclaw_common`'s process-wide `LazyLock`, so the tempdir env
//! var is set as the FIRST action, before any read (same pattern as
//! `crates/ironclaw_host_runtime/tests/trace_commons_dispatch_e2e.rs`).
//! Keep this suite a single sequenced `#[tokio::test]`: enrollment state is
//! process-global (per scope), so concurrent tests in this binary would race.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use ironclaw_support::group::IronClawIntegrationGroup;
use ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_traces::contribution::{
    StandingTraceContributionPolicy, queued_trace_envelope_paths_for_scope, trace_scope_key,
    write_trace_policy_for_scope,
};
use tempfile::TempDir;

const ONBOARD_CAPABILITY_ID: &str = "builtin.trace_commons.onboard";

/// Distinct actor for the inert (never-enrolled) consent-gate control thread —
/// see the scope-isolation note on `completed_turn_queues_trace_contribution_for_enrolled_scope`.
const CONSENT_CONTROL_ACTOR_ID: &str = "host-user-trace-consent-control";

static BASE_DIR: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();

/// Point `IRONCLAW_BASE_DIR` at a process-lifetime tempdir before any code
/// reads it through `ironclaw_common`'s `LazyLock`. Must be the first call
/// in every test in this binary.
fn setup_base_dir() -> &'static TempDir {
    BASE_DIR.get_or_init(|| {
        let dir = tempfile::tempdir().expect("tempdir for IRONCLAW_BASE_DIR");
        // Serialize against every other env-mutating test in this binary (same
        // guard `apply_hermetic_env` takes) before touching the process env.
        let _env_guard = ironclaw_common::env_helpers::lock_env();
        // SAFETY: the `lock_env()` guard above serializes against all other
        // env-mutating tests in this binary; `OnceLock::get_or_init` additionally
        // guarantees this closure body runs exactly once, before any base-dir
        // read (this fn is called as the first action of every test here).
        unsafe {
            std::env::set_var("IRONCLAW_BASE_DIR", dir.path());
        }
        dir
    })
}

/// The enrolled-state policy the onboard flow writes
/// (`onboarding/mod.rs::write_policy_at_dir`), minus the device-key/issuer
/// fields only a live handshake can produce. `min_submission_score: 0.0` keeps
/// a short scripted turn Submit-eligible (the 0.35 default would Hold-and-drop
/// it as low-value); flush still fails fast before any network because the
/// default `IRONCLAW_TRACE_SUBMIT_TOKEN` env is unset, so the queued envelope
/// is retained for the assertion.
fn enrolled_policy() -> StandingTraceContributionPolicy {
    StandingTraceContributionPolicy {
        enabled: true,
        ingestion_endpoint: Some("https://traces.invalid/v1/ingest".to_string()),
        include_message_text: true,
        include_tool_payloads: true,
        min_submission_score: 0.0,
        ..StandingTraceContributionPolicy::default()
    }
}

/// Poll for the spawned capture task (`capture_turn_trace` runs off the sink's
/// `publish` via `tokio::spawn`) to queue the envelope.
async fn wait_for_queued_envelopes(scope: &str) -> Vec<std::path::PathBuf> {
    for _ in 0..100 {
        let paths = queued_trace_envelope_paths_for_scope(Some(scope)).expect("read queue");
        if !paths.is_empty() {
            return paths;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no trace envelope queued for enrolled scope {scope:?} within 10s");
}

#[tokio::test]
async fn completed_turn_queues_trace_contribution_for_enrolled_scope() {
    setup_base_dir();

    let group = IronClawIntegrationGroup::builder()
        .with_trace_capture()
        .trace_commons_tools()
        .await
        .expect("trace-commons group builds");
    let scope = group
        .trace_capture_scope()
        .expect("with_trace_capture records the runtime owner scope")
        .to_string();

    // Turn 1 — NOT enrolled. Drives the real `builtin.trace_commons.onboard`
    // capability dispatch at the consent gate (`confirmed=false` returns
    // consent_required without enrolling), and doubles as the
    // inert-until-enrolled control: the sink runs (policy read per turn) but
    // must queue nothing.
    //
    // Scope isolation (regression guard for a prior race): this thread runs
    // under a DISTINCT actor (`CONSENT_CONTROL_ACTOR_ID`), so its capture
    // scope (`control_scope` below) is entirely disjoint from `scope` — the
    // canonical scope the SECOND thread enrolls and captures under. The
    // sink's per-event capture task is detached (`tokio::spawn`), so without
    // this isolation a slow scheduler could delay the first turn's capture
    // task until AFTER `write_trace_policy_for_scope(&scope, ...)` below
    // enrolls it, making the task observe the now-enrolled policy and queue
    // an envelope into `scope` — inflating the final count to 2. With a
    // disjoint scope, the control's own scope is NEVER enrolled by this test,
    // so its capture stays inert regardless of task-scheduling timing; no
    // sleep is required to make this deterministic.
    let first = group
        .thread("conv-trace-capture-consent")
        .with_actor_id(CONSENT_CONTROL_ACTOR_ID)
        .script([
            IronClawScriptedReply::tool_call(
                ONBOARD_CAPABILITY_ID,
                serde_json::json!({
                    "invite_url": "https://tc.example.test/onboard#REBORN-INT-CODE",
                    "confirmed": false
                }),
            ),
            IronClawScriptedReply::text("not enrolled"),
        ])
        .build()
        .await
        .expect("first thread builds");
    let control_scope = trace_scope_key(
        first.binding.tenant_id.as_str(),
        first
            .binding
            .subject_user_id
            .as_ref()
            .expect("resolved binding has a subject user id")
            .as_str(),
    );
    assert_ne!(
        control_scope, scope,
        "the control actor must resolve a DIFFERENT trace scope than the \
         canonical enrolled scope, or scope isolation below proves nothing"
    );
    first
        .submit_turn("contribute my traces?")
        .await
        .expect("consent-gate turn completes");
    first
        .assert_tool_invoked(ONBOARD_CAPABILITY_ID)
        .await
        .expect("onboard capability dispatched through the group runtime");

    // Enroll the scope: the same standing-policy state a completed onboard
    // handshake persists.
    write_trace_policy_for_scope(Some(&scope), &enrolled_policy()).expect("enroll scope");

    // Turn 2 — enrolled. A plain completed turn must now flow policy read →
    // capture → redact → score → queue.
    let second = group
        .thread("conv-trace-capture-enrolled")
        .script([IronClawScriptedReply::text("done")])
        .build()
        .await
        .expect("second thread builds");
    second
        .submit_turn("hello after enrollment")
        .await
        .expect("enrolled turn completes");

    let queued = wait_for_queued_envelopes(&scope).await;
    assert_eq!(
        queued.len(),
        1,
        "exactly the enrolled turn's envelope is queued (a same-scope late \
         capture from the pre-enrollment turn would make this 2): {queued:?}"
    );
    // The disjoint control scope must stay empty throughout — by now the test
    // has already waited (up to 10s) for the enrolled envelope above, so any
    // detached capture task the control turn spawned has long since run.
    assert_eq!(
        queued_trace_envelope_paths_for_scope(Some(&control_scope)).expect("read control queue"),
        Vec::<std::path::PathBuf>::new(),
        "the never-enrolled control scope must never accumulate a queued envelope"
    );
}
