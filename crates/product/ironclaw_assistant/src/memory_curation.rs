//! Periodic memory curation — the "dreaming" pass (issue #7276).
//!
//! Every so often, after an ordinary conversation turn finishes, the agent goes
//! off on its own with no user present, re-reads the user's standing memory
//! document, and tidies it: merges entries that say the same thing, resolves
//! superseded facts, tightens wording. Nothing is sent back to anyone — the
//! output is the edits plus a structured report.
//!
//! This exists because memory only ever grew. Writes accumulate, nothing prunes,
//! and the standing document has a byte budget, so redundancy crowds out what
//! matters. No human reads the file, so the decay is invisible.
//!
//! ## Shape
//!
//! The loop tier reports "a turn's run reached a terminal state" through the
//! `after_turn` hook point ([`ironclaw_hooks::sink::PrivilegedAfterTurnHook`]);
//! every policy decision lives here, including which of those turns count. A
//! pass is one unbound turn — no conversation, no reply target — submitted
//! through the same [`UnboundTurnService`] door OpenAI-compat and subagent
//! spawn use.
//!
//! ## Why the pass reads memory through a tool instead of being handed it
//!
//! The unbound run profile has no memory lane: nothing is injected into its
//! prompt automatically. That is the right shape here rather than a limitation —
//! a curation pass should not have the very document it is about to rewrite
//! placed in its context by machinery it does not control. It reads the document
//! explicitly, with the same mediated tool the model uses in conversation.
//!
//! ## Concurrency
//!
//! A pass rewrites `MEMORY.md` while the user may be writing to it from a live
//! conversation. Memory writes are compare-and-swap
//! (`CasExpectation::Version`), so a concurrent write does not clobber: the
//! losing writer is rejected. The failure mode is therefore a LOST CURATION
//! PASS, never a lost memory — which is the direction this must fail, and is
//! what makes the pass safe without batch-atomic memory operations.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_hooks::dispatch::{HookDispatcher, HookDispatcherBuilder};
use ironclaw_hooks::identity::{HookId, HookVersion};
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::AfterTurnHookContext;
use ironclaw_hooks::registry::HookRegistry;
use ironclaw_hooks::sink::PrivilegedAfterTurnHook;
use ironclaw_hooks::trust::HookTrustClass;
use ironclaw_host_api::ids::{CapabilityId, TenantId, UserId};
use ironclaw_host_api::output::OutputContract;
use ironclaw_host_api::prepared_context::TurnLimits;
use ironclaw_memory::{
    MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
};
use ironclaw_product_contracts::surface::ProductSurfaceCaller;
use ironclaw_threads::agent_message::{AgentMessage, AgentMessageRole, ContentPart};
use tracing::debug;

use crate::unbound_turn::{UnboundTurnError, UnboundTurnService, UnboundTurnSubmission};

/// The curation instruction. A prompt asset, not a Rust string: multi-line
/// prompts live in `prompts/*.md` in the crate that owns the behavior.
const MEMORY_CURATION_PROMPT: &str = include_str!("../prompts/memory_curation.md");

/// Opening message. The pass reads the document itself (see module docs), so
/// this only tells it to start.
const MEMORY_CURATION_KICKOFF: &str =
    "Perform a maintenance pass over the standing memory document now.";

/// Name of the structured report contract.
const MEMORY_CURATION_OUTPUT_NAME: &str = "memory_curation_report_v1";

/// How many completed user turns pass between curation runs, by default.
///
/// Ten matches the interval the Hermes agent uses for its own memory-review
/// fork. It is a rate, not a deadline: too frequent burns tokens re-reading a
/// document that has not changed, too rare lets redundancy accumulate past the
/// point where a single pass can fix it.
// Built without a panic-shaped branch: the production panic baseline scans
// syntactically, so even a compile-time-unreachable `unreachable!()` counts.
// `MIN` is 1; saturating_add is const and cannot fail.
pub const DEFAULT_CURATION_INTERVAL_TURNS: NonZeroU32 = NonZeroU32::MIN.saturating_add(9);

/// Iteration ceiling for one pass: read, decide, write, report, plus room for
/// one retry. A pass that cannot finish in this many steps is not converging,
/// and letting it spin costs tokens for a chore nobody requested.
const MEMORY_CURATION_MAX_ITERATIONS: u32 = 6;

/// Capability-call ceiling: read, maybe a search or two, one write, and the
/// result tool, plus headroom. A pass calling far more than this is thrashing.
const MEMORY_CURATION_MAX_CAPABILITY_CALLS: u32 = 12;

/// Wall-clock ceiling for one pass.
const MEMORY_CURATION_WALL_CLOCK_SECS: u32 = 90;

/// The three memory capabilities a pass may use: read the document, search for
/// related entries, write the result. Nothing else — a maintenance chore has no
/// need of the network, the filesystem, or any other surface, and the declared
/// selection is what bounds it.
fn curation_tools() -> Result<Vec<CapabilityId>, String> {
    [
        MEMORY_READ_CAPABILITY_ID,
        MEMORY_SEARCH_CAPABILITY_ID,
        MEMORY_WRITE_CAPABILITY_ID,
    ]
    .into_iter()
    .map(|id| {
        CapabilityId::new(id).map_err(|error| format!("invalid memory capability id {id}: {error}"))
    })
    .collect()
}

/// The report a pass must produce. Kept deliberately small: a person reading
/// operator output wants to know whether anything changed and what.
fn curation_report_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["changed", "summary"],
        "properties": {
            "changed": {
                "type": "boolean",
                "description": "Whether the standing memory document was rewritten."
            },
            "summary": {
                "type": "string",
                "maxLength": 500,
                "description": "One sentence describing what changed, or why nothing did."
            },
            "entries_merged": { "type": "integer", "minimum": 0 },
            "entries_removed": { "type": "integer", "minimum": 0 },
            "conflicts": {
                "type": "array",
                "maxItems": 20,
                "items": { "type": "string", "maxLength": 200 },
                "description":
                    "Contradictions left in place because the current fact could not be determined."
            }
        }
    })
}

/// Submits one curation pass. Narrow on purpose: it is the whole seam between
/// the curation POLICY (when to run) and the turn machinery (how to run), which
/// is what lets the policy be tested without a coordinator or a thread store.
#[async_trait]
pub trait CurationPassSubmitter: Send + Sync {
    /// The door's own typed error, carried whole: a `String` here would flatten
    /// "the caller built an invalid submission" and "the services are down" into
    /// one indistinguishable line at the `debug!` boundary below.
    async fn submit_pass(&self, submission: UnboundTurnSubmission) -> Result<(), UnboundTurnError>;
}

#[async_trait]
impl CurationPassSubmitter for UnboundTurnService {
    async fn submit_pass(&self, submission: UnboundTurnSubmission) -> Result<(), UnboundTurnError> {
        // Fire-and-forget by design: the pass runs on the scheduler like any
        // other turn. Nothing waits for its result, and nothing reads it back —
        // its effect is the memory it rewrote.
        self.accept_and_submit(submission).await.map(|_| ())
    }
}

/// Who a set of counted turns belongs to. Typed rather than a formatted
/// `"{tenant}/{user}"` string so two owners can never collide through a
/// separator that happens to appear inside an id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CurationOwner {
    tenant_id: TenantId,
    user_id: UserId,
}

impl CurationOwner {
    fn from_context(ctx: &AfterTurnHookContext) -> Self {
        Self {
            tenant_id: ctx.tenant_id.clone(),
            user_id: ctx.user_id.clone(),
        }
    }
}

/// Policy: counts completed turns per owner and submits a pass every Nth one.
pub struct MemoryCurationService {
    submitter: Arc<dyn CurationPassSubmitter>,
    interval_turns: NonZeroU32,
    /// Completed turns since each owner's last pass.
    ///
    /// In-memory on purpose, with a bounded consequence: a restart resets the
    /// counts, so the next pass for an active user happens later than it
    /// otherwise would. Curation is a periodic chore with no deadline, so a late
    /// pass is not a defect — and paying for durable per-user counters to avoid
    /// it would buy nothing a user could perceive. Made durable only if the
    /// interval ever becomes something a user configures and expects to hold.
    counters: Mutex<HashMap<CurationOwner, u32>>,
}

impl MemoryCurationService {
    /// The interval is `NonZeroU32` so "a pass after every single turn while
    /// reading as disabled to whoever wrote the config" is not a representable
    /// state here at all. Zero is rejected where an operator can still see why
    /// — `[memory].curation_interval_turns` validation in `ironclaw_config` —
    /// and curation is disabled by NOT WIRING the service, never by a sentinel.
    pub fn new(submitter: Arc<dyn CurationPassSubmitter>, interval_turns: NonZeroU32) -> Self {
        Self {
            submitter,
            interval_turns,
            counters: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_default_interval(submitter: Arc<dyn CurationPassSubmitter>) -> Self {
        Self::new(submitter, DEFAULT_CURATION_INTERVAL_TURNS)
    }

    /// Count this turn and report whether it triggers a pass.
    ///
    /// A poisoned lock declines rather than panicking: this runs on a
    /// post-terminal background path where a panic would be far worse than a
    /// skipped chore.
    fn count_and_check(&self, owner: CurationOwner) -> bool {
        let Ok(mut counters) = self.counters.lock() else {
            debug!("memory curation: counter lock poisoned; skipping this turn");
            return false;
        };
        let counter = counters.entry(owner).or_insert(0);
        *counter += 1;
        if *counter < self.interval_turns.get() {
            return false;
        }
        *counter = 0;
        true
    }

    /// Per-pass identity, taken from the run that triggered it.
    ///
    /// Two properties at once, which is why it is the triggering run and not a
    /// counter or a clock: a crash-retry of that same run replays the same id,
    /// so the accept door converges on ONE pass instead of minting a second
    /// over the same document; and every other interval is triggered by a
    /// different run, so it gets a different id instead of being replayed as
    /// the first pass forever.
    fn pass_id(ctx: &AfterTurnHookContext) -> String {
        format!(
            "memory-curation-{}-{}-{}",
            ctx.tenant_id.as_str(),
            ctx.user_id.as_str(),
            ctx.run_id
        )
    }

    fn build_submission(ctx: &AfterTurnHookContext) -> Result<UnboundTurnSubmission, String> {
        let output =
            OutputContract::try_json_schema(MEMORY_CURATION_OUTPUT_NAME, curation_report_schema())
                .map_err(|error| format!("curation report schema is invalid: {error}"))?;
        let public_id = Self::pass_id(ctx);
        Ok(UnboundTurnSubmission {
            // The pass acts AS the owner. Memory is per-user: a pass acting as
            // anything else would read and write the wrong scope. Never an
            // operator-config caller — curation touches one user's memory and
            // has no business holding deployment-wide authority.
            caller: ProductSurfaceCaller::new(
                ctx.tenant_id.clone(),
                ctx.user_id.clone(),
                ctx.agent_id.clone(),
                ctx.project_id.clone(),
            ),
            public_id: public_id.clone(),
            system_prompt: MEMORY_CURATION_PROMPT.to_string(),
            messages: vec![AgentMessage {
                role: AgentMessageRole::User,
                content: vec![ContentPart::text(MEMORY_CURATION_KICKOFF)],
            }],
            tools: curation_tools()?,
            output,
            // A background chore nobody is watching needs a ceiling. Without
            // one it inherits the unbound profile's 1024-iteration budget and
            // no wall clock — a pass that fails to converge would burn tokens
            // against a user's memory unobserved until it hit that limit.
            limits: TurnLimits {
                max_model_calls: Some(MEMORY_CURATION_MAX_ITERATIONS),
                max_capability_invocations: Some(MEMORY_CURATION_MAX_CAPABILITY_CALLS),
                max_wall_clock_seconds: Some(MEMORY_CURATION_WALL_CLOCK_SECS),
            },
            requested_model: None,
            idempotency_key: public_id,
        })
    }
}

/// Canonical identity of the curation hook binding. `for_builtin` hashes a
/// stable path + symbol, so this string is the hook's durable identity across
/// restarts and must not be reworded casually.
const MEMORY_CURATION_HOOK_PATH: &str =
    "ironclaw_assistant::memory_curation::MemoryCurationService";

/// Assemble the `after_turn` dispatcher that carries the curation hook.
///
/// Composition calls this instead of assembling the parts itself: which hook,
/// at which phase, under which trust class, is a decision belonging to the
/// crate that owns the behavior, not to the wiring root. A caller that does not
/// want curation does not call this at all — there is no "disabled" dispatcher
/// (see [`MemoryCurationService::new`] on why disabling is never a sentinel
/// interval).
///
/// The dispatcher is deliberately its OWN small dispatcher rather than the
/// per-run one the loop middleware builds: `after_turn` fires once per run from
/// the executor, which holds a single process-lifetime `Arc`, while the
/// middleware dispatchers are minted per run (and, for third-party bindings,
/// per tenant). Sharing one would mean rebuilding this binding on every run for
/// no gain.
pub fn after_turn_curation_dispatcher(
    submitter: Arc<dyn CurationPassSubmitter>,
    interval_turns: NonZeroU32,
) -> Result<Arc<HookDispatcher>, String> {
    let service = MemoryCurationService::new(submitter, interval_turns);
    Ok(HookDispatcherBuilder::new(HookRegistry::new())
        .install_after_turn(
            HookId::for_builtin(MEMORY_CURATION_HOOK_PATH, HookVersion::ONE),
            // Telemetry, the last phase: the run this reacts to is already
            // terminal, so the hook enforces no contract and gates nothing —
            // it only reads the outcome and may start its own work.
            HookPhase::Telemetry,
            HookTrustClass::Builtin,
            Box::new(service),
        )
        .map_err(|error| format!("could not install the memory curation hook: {error}"))?
        .build_arc())
}

#[async_trait]
impl PrivilegedAfterTurnHook for MemoryCurationService {
    async fn on_turn(&self, ctx: &AfterTurnHookContext) {
        // Only successful turns count. A failed or cancelled turn says nothing
        // about whether memory needs tidying, and counting it would drift the
        // interval — the point fires for every terminal state, so this filter
        // has to live here.
        if !ctx.completed {
            return;
        }
        // Per-owner key: each user's turns count toward their OWN pass, so a
        // busy user cannot trigger curation over a quiet user's memory.
        if !self.count_and_check(CurationOwner::from_context(ctx)) {
            return;
        }
        let submission = match Self::build_submission(ctx) {
            Ok(submission) => submission,
            Err(reason) => {
                debug!("memory curation: could not build pass: {reason}");
                return;
            }
        };
        // DECISION #7770 (gate posture): a pass runs unbound, and the unbound
        // loop family aborts on an approval gate with `gate_not_supported`
        // (`ironclaw_agent_loop::strategies::gate::GateNotSupportedStrategy`).
        // `ironclaw.memory.write` is auto-approved for a default user but NOT
        // exempt from the gate, so a user who turned auto-approve off would get
        // a failed pass instead of a skipped one. The epic's intent is
        // skip-and-note, and it is NOT implemented here on purpose: no
        // read-only "would this capability gate for this scope" query exists.
        // Deciding it needs the capability DESCRIPTOR (effects + origin gate
        // matrix), the run's `ApprovalPolicy`, the `TrustDecision`, grants, and
        // leases composed together — which happens only inside
        // `authorize_dispatch_with_trust` at dispatch time, and whose origin
        // input does not exist until the run is executing. Approximating it
        // from `ApprovalSettingsProvider::global_auto_approve` alone would
        // duplicate gate composition in a product service and drift from the
        // authorizer, which is the stage-collapsing this codebase forbids. The
        // honest fix belongs at the gate seam (a `GateOutcome` that skips the
        // capability for the model instead of aborting the run), not here.
        //
        // Every failure is swallowed at `debug!`. This runs after a terminal run
        // on a background path: `info!`/`warn!` would corrupt the REPL, and a
        // failed chore must never surface as a user-visible problem.
        if let Err(error) = self.submitter.submit_pass(submission).await {
            debug!("memory curation: pass submission failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use ironclaw_host_api::turn::TurnRunId;

    use super::*;

    /// Non-zero intervals for the tests, built where a `NonZeroU32` is needed.
    fn interval(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("test interval is non-zero")
    }

    #[derive(Default)]
    struct RecordingSubmitter {
        submissions: StdMutex<Vec<UnboundTurnSubmission>>,
        fail: bool,
    }

    #[async_trait]
    impl CurationPassSubmitter for RecordingSubmitter {
        async fn submit_pass(
            &self,
            submission: UnboundTurnSubmission,
        ) -> Result<(), UnboundTurnError> {
            self.submissions
                .lock()
                .expect("submissions lock")
                .push(submission);
            if self.fail {
                return Err(UnboundTurnError::Unavailable);
            }
            Ok(())
        }
    }

    impl RecordingSubmitter {
        fn count(&self) -> usize {
            self.submissions.lock().expect("submissions lock").len()
        }
        fn all(&self) -> Vec<UnboundTurnSubmission> {
            self.submissions.lock().expect("submissions lock").clone()
        }
        fn last(&self) -> UnboundTurnSubmission {
            self.submissions
                .lock()
                .expect("submissions lock")
                .last()
                .cloned()
                .expect("a submission")
        }
    }

    fn ctx_for(user: &str) -> AfterTurnHookContext {
        ctx_for_status(user, true)
    }

    fn ctx_for_status(user: &str, completed: bool) -> AfterTurnHookContext {
        ctx_for_run(user, completed, TurnRunId::new())
    }

    fn ctx_for_run(user: &str, completed: bool, run_id: TurnRunId) -> AfterTurnHookContext {
        AfterTurnHookContext::new(
            TenantId::new("tenant-a").expect("tenant id"),
            run_id,
            UserId::new(user).expect("user id"),
            None,
            None,
            completed,
        )
    }

    fn service(interval_turns: u32) -> (MemoryCurationService, Arc<RecordingSubmitter>) {
        let submitter = Arc::new(RecordingSubmitter::default());
        (
            MemoryCurationService::new(
                Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
                interval(interval_turns),
            ),
            submitter,
        )
    }

    /// Curation is a periodic chore, not a per-turn one: a pass every turn would
    /// burn tokens re-reading a document that has not changed.
    #[tokio::test]
    async fn a_pass_is_submitted_only_every_nth_turn() {
        let (service, submitter) = service(3);

        for _ in 0..2 {
            service.on_turn(&ctx_for("user-a")).await;
        }
        assert_eq!(
            submitter.count(),
            0,
            "no pass before the interval is reached"
        );

        service.on_turn(&ctx_for("user-a")).await;
        assert_eq!(submitter.count(), 1, "the third turn triggers a pass");

        for _ in 0..2 {
            service.on_turn(&ctx_for("user-a")).await;
        }
        assert_eq!(submitter.count(), 1, "the counter resets after a pass");

        service.on_turn(&ctx_for("user-a")).await;
        assert_eq!(
            submitter.count(),
            2,
            "and triggers again one interval later"
        );
    }

    /// Each user's turns count toward their OWN pass. A shared counter would let
    /// a busy user trigger curation over a quiet user's memory.
    #[tokio::test]
    async fn counters_are_per_owner() {
        let (service, submitter) = service(2);

        service.on_turn(&ctx_for("user-a")).await;
        service.on_turn(&ctx_for("user-b")).await;
        assert_eq!(submitter.count(), 0, "one turn each is below the interval");

        service.on_turn(&ctx_for("user-a")).await;
        assert_eq!(submitter.count(), 1);
        assert_eq!(
            submitter.last().caller.user_id.as_str(),
            "user-a",
            "the pass belongs to the user whose turns triggered it"
        );
    }

    /// The pass acts as the owner, over exactly the three memory capabilities,
    /// and must produce a structured report. Each of these is a security or
    /// correctness property, not a preference.
    #[tokio::test]
    async fn the_submitted_pass_is_scoped_and_bounded() {
        let (service, submitter) = service(1);
        service.on_turn(&ctx_for("user-a")).await;

        let submission = submitter.last();
        assert_eq!(submission.caller.user_id.as_str(), "user-a");
        assert_eq!(submission.caller.tenant_id.as_str(), "tenant-a");
        assert!(
            !submission.caller.operator_config,
            "a per-user chore must never hold deployment-wide authority"
        );

        let tools: Vec<&str> = submission.tools.iter().map(|id| id.as_str()).collect();
        assert_eq!(
            tools,
            vec![
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID
            ],
            "a maintenance chore gets memory and nothing else"
        );

        assert!(
            matches!(submission.output, OutputContract::JsonSchema { .. }),
            "the pass must report what it changed, not free text"
        );
        assert!(
            submission
                .system_prompt
                .contains("Never invent, infer, or extrapolate"),
            "the anti-fabrication rule must reach the model"
        );

        // Without declared ceilings a pass inherits the unbound profile's
        // 1024-iteration budget and no wall clock. Nobody is watching this run,
        // so an unconverged pass would burn tokens against a user's memory
        // until it hit that limit — the failure nobody would notice.
        assert_eq!(
            submission.limits.max_model_calls,
            Some(MEMORY_CURATION_MAX_ITERATIONS)
        );
        assert_eq!(
            submission.limits.max_capability_invocations,
            Some(MEMORY_CURATION_MAX_CAPABILITY_CALLS)
        );
        assert_eq!(
            submission.limits.max_wall_clock_seconds,
            Some(MEMORY_CURATION_WALL_CLOCK_SECS)
        );
        assert!(
            !submission.limits.is_unlimited(),
            "an unwatched background pass must never run unbounded"
        );
    }

    /// A crash-retry of the triggering turn must converge on the same pass
    /// rather than starting a second one over the same document.
    #[tokio::test]
    async fn the_pass_id_is_the_idempotency_key() {
        let (service, submitter) = service(1);
        service.on_turn(&ctx_for("user-a")).await;

        let submission = submitter.last();
        assert_eq!(
            submission.idempotency_key, submission.public_id,
            "the accept door replays on this key"
        );
        assert!(submission.public_id.starts_with("memory-curation-"));
        assert!(submission.public_id.contains("user-a"));
    }

    /// A failed submission is swallowed. This runs after a run is already
    /// terminal; a background chore must never surface as a user-visible
    /// problem, and must never panic on the scheduler worker.
    #[tokio::test]
    async fn a_failed_submission_is_swallowed() {
        let submitter = Arc::new(RecordingSubmitter {
            submissions: StdMutex::new(Vec::new()),
            fail: true,
        });
        let service = MemoryCurationService::new(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            interval(1),
        );

        service.on_turn(&ctx_for("user-a")).await;

        assert_eq!(submitter.count(), 1, "it tried");
    }

    /// The point fires for every terminal state, so a failed or cancelled turn
    /// arrives here too. It must not count: a turn that never finished says
    /// nothing about whether memory needs tidying, and counting it would drift
    /// the interval away from "every Nth real turn".
    #[tokio::test]
    async fn an_unsuccessful_turn_never_counts_toward_the_interval() {
        let (service, submitter) = service(2);

        for _ in 0..5 {
            service.on_turn(&ctx_for_status("user-a", false)).await;
        }
        assert_eq!(submitter.count(), 0, "unsuccessful turns do not accumulate");

        service.on_turn(&ctx_for("user-a")).await;
        service.on_turn(&ctx_for("user-a")).await;
        assert_eq!(
            submitter.count(),
            1,
            "the interval is counted purely in successful turns"
        );
    }

    /// The factory composition wires is only useful if a turn dispatched
    /// through the built dispatcher actually reaches the service. Testing the
    /// service alone would leave the install arguments (point, trust class,
    /// phase) unproven — and a wrong trust class is an install-time rejection
    /// that would silently leave curation un-wired in production.
    #[tokio::test]
    async fn the_built_dispatcher_delivers_a_turn_to_the_curation_service() {
        let submitter = Arc::new(RecordingSubmitter::default());
        let dispatcher = after_turn_curation_dispatcher(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            interval(1),
        )
        .expect("the curation hook installs");

        dispatcher.dispatch_after_turn(ctx_for("user-a")).await;

        assert_eq!(
            submitter.count(),
            1,
            "a completed turn dispatched at the after_turn point must reach the hook"
        );
        assert_eq!(submitter.last().caller.user_id.as_str(), "user-a");
    }

    /// THE identity bug this replaced: a pass id derived from anything that
    /// does not change per trigger (a per-owner counter, say) means every
    /// interval after the first reuses the first pass's idempotency key, and
    /// the accept door REPLAYS that first pass instead of running a new one —
    /// so the document is curated exactly once, ever, and nothing surfaces the
    /// fact.
    #[tokio::test]
    async fn each_triggering_run_yields_a_distinct_pass() {
        let (service, submitter) = service(1);

        service.on_turn(&ctx_for("user-a")).await;
        service.on_turn(&ctx_for("user-a")).await;

        let submissions = submitter.all();
        assert_eq!(submissions.len(), 2, "two triggers, two passes");
        assert_ne!(
            submissions[0].public_id, submissions[1].public_id,
            "a second interval must be a NEW pass, not a replay of the first"
        );
        assert_ne!(
            submissions[0].idempotency_key, submissions[1].idempotency_key,
            "the accept door replays on this key, so it must differ too"
        );
    }

    /// The other half of the same property: a crash-retry of the SAME
    /// triggering run must converge on one pass rather than starting a second
    /// one over the same document.
    #[tokio::test]
    async fn the_same_triggering_run_converges_on_one_pass_id() {
        let (service, submitter) = service(1);
        let run_id = TurnRunId::new();

        service.on_turn(&ctx_for_run("user-a", true, run_id)).await;
        service.on_turn(&ctx_for_run("user-a", true, run_id)).await;

        let submissions = submitter.all();
        assert_eq!(submissions.len(), 2, "both retries reached the submitter");
        assert_eq!(
            submissions[0].public_id, submissions[1].public_id,
            "a replayed trigger must produce the same pass, which the accept door dedupes"
        );
    }
}
