//! Scheduled memory upkeep — the dispatcher for `[memory].scheduled_ops`
//! (issues #7276, #7664).
//!
//! A memory provider declares its own recurring upkeep in its manifest: which
//! host trigger it rides, how often, and what to run. The host owns the clock,
//! the invocation envelope, and the authority; the declaration owns the work.
//! Nothing here knows which provider is bound or what its upkeep is about — it
//! runs whatever the RESOLVED declaration says.
//!
//! The native provider's declaration is a curation pass: every so often, after
//! an ordinary conversation turn finishes, the agent goes off on its own with
//! no user present, re-reads the user's standing memory document, and tidies it
//! — merges entries that say the same thing, resolves superseded facts, tightens
//! wording. Nothing is sent back to anyone; the output is the edits plus a
//! structured report. That exists because memory only ever grew: writes
//! accumulate, nothing prunes, the standing document has a byte budget, and no
//! human reads the file, so the decay is invisible.
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
//! ## Why a pass reads memory through a tool instead of being handed it
//!
//! The unbound run profile has no memory lane: nothing is injected into its
//! prompt automatically. That is the right shape here rather than a limitation —
//! an upkeep pass should not have the very document it is about to rewrite
//! placed in its context by machinery it does not control. It reads the document
//! explicitly, with the same mediated tools the model uses in conversation —
//! exactly the ones the declaration selected.
//!
//! ## Concurrency
//!
//! A pass rewrites the standing document while the user may be writing to it
//! from a live conversation. Memory writes are compare-and-swap
//! (`CasExpectation::Version`), so a concurrent write does not clobber: the
//! losing writer is rejected. The failure mode is therefore a LOST UPKEEP PASS,
//! never a lost memory — which is the direction this must fail, and is what
//! makes the pass safe without batch-atomic memory operations.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_extension_contracts::memory::{
    MIN_SCHEDULED_OP_INTERVAL_TURNS, MemoryScheduledOp, MemoryScheduledOpKind,
    MemoryScheduledTrigger,
};
use ironclaw_hooks::dispatch::{HookDispatcher, HookDispatcherBuilder};
use ironclaw_hooks::identity::{HookId, HookVersion};
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::AfterTurnHookContext;
use ironclaw_hooks::registry::HookRegistry;
use ironclaw_hooks::sink::PrivilegedAfterTurnHook;
use ironclaw_host_api::ids::{CapabilityId, TenantId, UserId};
use ironclaw_host_api::output::OutputContract;
use ironclaw_host_api::prepared_context::TurnLimits;
use ironclaw_product_contracts::surface::ProductSurfaceCaller;
use ironclaw_threads::agent_message::{AgentMessage, AgentMessageRole, ContentPart};
use tracing::debug;

use crate::unbound_turn::{UnboundTurnError, UnboundTurnService, UnboundTurnSubmission};

/// Opening message. The pass reads the document itself (see module docs), so
/// this only tells it to start. Host-owned: the declaration supplies the
/// instruction (the system prompt), the host supplies the envelope around it.
const SCHEDULED_PASS_KICKOFF: &str =
    "Perform a maintenance pass over the standing memory document now.";

/// Name of the structured report contract.
const SCHEDULED_PASS_OUTPUT_NAME: &str = "memory_curation_report_v1";

/// Capability calls allowed per declared tool: room for the call itself, a
/// retry after a malformed one, a refinement, and the result call. Host-owned
/// and derived from the declaration's own size, not declared: a manifest sizes
/// its MODEL-call budget (`pass.max_model_calls`, itself under a contract
/// ceiling), while how much tool traffic that budget may generate is the host's
/// to bound. Four per tool reproduces the ceiling of 12 the three-tool curation
/// pass shipped with, and scales honestly with a larger selection.
const CAPABILITY_CALLS_PER_DECLARED_TOOL: u32 = 4;

/// Floor under that derivation, so a pass declaring no tools can still emit its
/// structured report rather than being denied its own result call.
const MIN_PASS_CAPABILITY_CALLS: u32 = 4;

/// Wall-clock ceiling for one pass. Host-owned, not declarable: a manifest may
/// size its model-call budget, but how long an unwatched background run may
/// occupy the scheduler is the deployment's concern.
const SCHEDULED_PASS_WALL_CLOCK_SECS: u32 = 90;

/// The report a pass must produce. Kept deliberately small: a person reading
/// operator output wants to know whether anything changed and what.
///
/// Host-owned in v0: the declaration says what work to do, and the host says
/// what a completed op must report back, so an operator reads the same shape
/// whichever provider is bound.
fn scheduled_pass_report_schema() -> serde_json::Value {
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

/// Submits one scheduled pass. Narrow on purpose: it is the whole seam between
/// the scheduling POLICY (when to run) and the turn machinery (how to run),
/// which is what lets the policy be tested without a coordinator or a thread
/// store.
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

/// Whose counted turns these are, on which trigger. Typed rather than a
/// formatted `"{tenant}/{user}"` string so two owners can never collide through
/// a separator that happens to appear inside an id; the trigger is part of the
/// key so ops riding different triggers never share a cadence.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScheduledOpOwner {
    tenant_id: TenantId,
    user_id: UserId,
    trigger: MemoryScheduledTrigger,
}

impl ScheduledOpOwner {
    fn from_context(ctx: &AfterTurnHookContext, trigger: MemoryScheduledTrigger) -> Self {
        Self {
            tenant_id: ctx.tenant_id.clone(),
            user_id: ctx.user_id.clone(),
            trigger,
        }
    }
}

/// A declared pass op with its prompt asset resolved — everything needed to
/// build a submission, and nothing that still has to be looked up.
#[derive(Debug, Clone)]
struct ResolvedPass {
    prompt: String,
    tools: Vec<CapabilityId>,
    max_model_calls: NonZeroU32,
}

/// A declared op with its assets resolved.
#[derive(Debug, Clone)]
enum ResolvedOp {
    Pass(ResolvedPass),
}

/// Runs one provider-declared scheduled op: counts qualifying turns per owner
/// and dispatches the declared work every Nth one.
///
/// Everything it dispatches comes from the declaration — cadence, instruction,
/// tool selection, model-call budget. The host contributes only what a
/// declaration must not be able to name for itself: who the pass acts as, the
/// wall clock, the tool-traffic ceiling, and the report contract.
pub struct MemoryScheduledOpRunner {
    submitter: Arc<dyn CurationPassSubmitter>,
    trigger: MemoryScheduledTrigger,
    interval_turns: NonZeroU32,
    op: ResolvedOp,
    /// Qualifying turns since each owner's last dispatch on this trigger.
    ///
    /// In-memory on purpose, with a bounded consequence: a restart resets the
    /// counts, so the next dispatch for an active user happens later than it
    /// otherwise would. Upkeep is a periodic chore with no deadline, so a late
    /// pass is not a defect — and paying for durable per-user counters to avoid
    /// it would buy nothing a user could perceive. Made durable only if the
    /// interval ever becomes something a user configures and expects to hold.
    counters: Mutex<HashMap<ScheduledOpOwner, u32>>,
}

impl MemoryScheduledOpRunner {
    /// Build a runner from a provider's DECLARATION plus the prompt text the
    /// host resolved for it, with an optional deployment override of the
    /// declared cadence.
    ///
    /// The override is validated against the same floor the declaration is
    /// ([`MIN_SCHEDULED_OP_INTERVAL_TURNS`]): a manifest may not demand
    /// per-turn invocation on someone else's deployment, and an operator config
    /// must not be a way around that for the deployment's own users either.
    /// Absent override = the provider's declared cadence applies.
    pub fn from_declaration(
        submitter: Arc<dyn CurationPassSubmitter>,
        declared: &MemoryScheduledOp,
        prompt: &str,
        interval_override: Option<NonZeroU32>,
    ) -> Result<Self, String> {
        let interval_turns = interval_override.unwrap_or(declared.interval_turns);
        if interval_turns.get() < MIN_SCHEDULED_OP_INTERVAL_TURNS {
            return Err(format!(
                "a scheduled-op interval of {} turns is below the minimum of \
                 {MIN_SCHEDULED_OP_INTERVAL_TURNS}",
                interval_turns.get()
            ));
        }
        // One arm, and no catch-all: a `tool` op is unreachable by construction
        // in v0 — the manifest parser REJECTS `tool = "..."` with its own
        // message (#7664), so no such declaration can reach a runner. When that
        // variant lands, this match stops compiling, which is the point: a
        // wildcard would let a new op kind be scheduled and silently do nothing.
        let op = match &declared.op {
            MemoryScheduledOpKind::Pass(pass) => ResolvedOp::Pass(ResolvedPass {
                prompt: prompt.to_string(),
                tools: pass.tools.clone(),
                max_model_calls: pass.max_model_calls,
            }),
        };
        Ok(Self {
            submitter,
            trigger: declared.trigger,
            interval_turns,
            op,
            counters: Mutex::new(HashMap::new()),
        })
    }

    /// Count this turn and report whether it triggers a dispatch.
    ///
    /// A poisoned lock declines rather than panicking: this runs on a
    /// post-terminal background path where a panic would be far worse than a
    /// skipped chore.
    fn count_and_check(&self, owner: ScheduledOpOwner) -> bool {
        let Ok(mut counters) = self.counters.lock() else {
            debug!("memory scheduled op: counter lock poisoned; skipping this turn");
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
    ///
    /// The `memory-curation-` prefix is contract, not decoration — operator
    /// tooling and the integration scenario both key off it — so it stays put
    /// even though the dispatcher is no longer curation-specific.
    fn pass_id(ctx: &AfterTurnHookContext) -> String {
        format!(
            "memory-curation-{}-{}-{}",
            ctx.tenant_id.as_str(),
            ctx.user_id.as_str(),
            ctx.run_id
        )
    }

    /// Tool-traffic ceiling for a pass, derived from how many tools it
    /// declared. See [`CAPABILITY_CALLS_PER_DECLARED_TOOL`].
    fn max_capability_calls(tool_count: usize) -> u32 {
        u32::try_from(tool_count)
            .unwrap_or(u32::MAX)
            .saturating_mul(CAPABILITY_CALLS_PER_DECLARED_TOOL)
            .max(MIN_PASS_CAPABILITY_CALLS)
    }

    fn build_pass_submission(
        pass: &ResolvedPass,
        ctx: &AfterTurnHookContext,
    ) -> Result<UnboundTurnSubmission, String> {
        let output = OutputContract::try_json_schema(
            SCHEDULED_PASS_OUTPUT_NAME,
            scheduled_pass_report_schema(),
        )
        .map_err(|error| format!("scheduled pass report schema is invalid: {error}"))?;
        let public_id = Self::pass_id(ctx);
        Ok(UnboundTurnSubmission {
            // The pass acts AS the owner. Memory is per-user: a pass acting as
            // anything else would read and write the wrong scope. Never an
            // operator-config caller — upkeep touches one user's memory and has
            // no business holding deployment-wide authority.
            caller: ProductSurfaceCaller::new(
                ctx.tenant_id.clone(),
                ctx.user_id.clone(),
                ctx.agent_id.clone(),
                ctx.project_id.clone(),
            ),
            public_id: public_id.clone(),
            system_prompt: pass.prompt.clone(),
            messages: vec![AgentMessage {
                role: AgentMessageRole::User,
                content: vec![ContentPart::text(SCHEDULED_PASS_KICKOFF)],
            }],
            tools: pass.tools.clone(),
            // Deliberately NOT the unattended narrowing (#7812): the pass's
            // surface is already the manifest's declared tool list — narrowed
            // above via `tools` — and dropping an approval-gated declared tool
            // here would silently break the pass (its write is the whole
            // point) instead of surfacing the misconfiguration. If a declared
            // tool requires approval, parking visibly is the better failure.
            require_no_approval: false,
            output,
            // A background chore nobody is watching needs a ceiling. Without
            // one it inherits the unbound profile's 1024-iteration budget and
            // no wall clock — a pass that fails to converge would burn tokens
            // against a user's memory unobserved until it hit that limit.
            limits: TurnLimits {
                max_model_calls: Some(pass.max_model_calls.get()),
                max_capability_invocations: Some(Self::max_capability_calls(pass.tools.len())),
                max_wall_clock_seconds: Some(SCHEDULED_PASS_WALL_CLOCK_SECS),
            },
            requested_model: None,
            idempotency_key: public_id,
        })
    }
}

/// Canonical identity of the scheduled-op hook binding. `for_builtin` hashes a
/// stable path + symbol, so this string is the hook's durable identity across
/// restarts and must not be reworded casually.
const MEMORY_SCHEDULED_OP_HOOK_PATH: &str =
    "ironclaw_assistant::memory_scheduled_ops::MemoryScheduledOpRunner";

/// Mints the per-run `after_turn` dispatcher carrying the scheduled-op hook.
///
/// A FACTORY of dispatchers, not a dispatcher: the hook framework scopes
/// poison (a panicking or timed-out hook) to one turn run by contract, so the
/// executor mints a fresh dispatcher per terminal run. One process-lifetime
/// dispatcher would turn a single bad run into scheduled upkeep being off until
/// the process restarts, with nothing surfacing it.
pub type AfterTurnDispatcherFactory = Arc<dyn Fn() -> Arc<HookDispatcher> + Send + Sync>;

/// Forwards the point to the one long-lived runner.
///
/// Every per-run dispatcher installs a fresh box, all pointing at the SAME
/// [`MemoryScheduledOpRunner`]: the per-owner turn counters are the policy's
/// whole state and must accumulate across runs. A runner minted per run would
/// count every turn as the first one and never reach an interval.
struct ScheduledOpHookBinding(Arc<MemoryScheduledOpRunner>);

#[async_trait]
impl PrivilegedAfterTurnHook for ScheduledOpHookBinding {
    async fn on_turn(&self, ctx: &AfterTurnHookContext) {
        self.0.on_turn(ctx).await;
    }
}

/// Build one dispatcher over the shared runner.
fn scheduled_op_dispatcher(
    runner: &Arc<MemoryScheduledOpRunner>,
) -> Result<Arc<HookDispatcher>, String> {
    Ok(HookDispatcherBuilder::new(HookRegistry::new())
        .install_builtin_after_turn(
            HookId::for_builtin(MEMORY_SCHEDULED_OP_HOOK_PATH, HookVersion::ONE),
            // Telemetry, the last phase: the run this reacts to is already
            // terminal, so the hook enforces no contract and gates nothing —
            // it only reads the outcome and may start its own work.
            HookPhase::Telemetry,
            Box::new(ScheduledOpHookBinding(Arc::clone(runner))),
        )
        .map_err(|error| format!("could not install the memory scheduled-op hook: {error}"))?
        .build_arc())
}

/// Assemble the `after_turn` dispatcher factory for a provider's declared
/// after-turn op.
///
/// Composition calls this instead of assembling the parts itself: which hook,
/// at which phase, under which trust class is a decision belonging to the crate
/// that owns the behavior, not to the wiring root. A provider that declares no
/// after-turn op is never handed here at all — there is no "disabled"
/// dispatcher and no sentinel interval.
pub fn after_turn_scheduled_op_dispatcher_factory(
    submitter: Arc<dyn CurationPassSubmitter>,
    declared: &MemoryScheduledOp,
    prompt: &str,
    interval_override: Option<NonZeroU32>,
) -> Result<AfterTurnDispatcherFactory, String> {
    match declared.trigger {
        MemoryScheduledTrigger::AfterTurn => {}
    }
    let runner = Arc::new(MemoryScheduledOpRunner::from_declaration(
        submitter,
        declared,
        prompt,
        interval_override,
    )?);
    // Install once here, at build time, and throw the result away: an install
    // that cannot succeed must fail the deployment's startup rather than go
    // quiet at the first terminal run, where nothing would report it.
    scheduled_op_dispatcher(&runner)?;
    Ok(Arc::new(move || match scheduled_op_dispatcher(&runner) {
        Ok(dispatcher) => dispatcher,
        Err(reason) => {
            // Unreachable in practice: the identical install already succeeded
            // above, over the same hook id and phase. If it somehow fails now,
            // this run gets an EMPTY dispatcher rather than a shared one — a
            // skipped chore, never a dispatcher whose poison outlives the run.
            debug!("memory scheduled op: could not mint this run's dispatcher: {reason}");
            HookDispatcherBuilder::new(HookRegistry::new()).build_arc()
        }
    }))
}

#[async_trait]
impl PrivilegedAfterTurnHook for MemoryScheduledOpRunner {
    async fn on_turn(&self, ctx: &AfterTurnHookContext) {
        // Only successful turns count. A failed or cancelled turn says nothing
        // about whether memory needs tidying, and counting it would drift the
        // interval — the point fires for every terminal state, so this filter
        // has to live here.
        if !ctx.completed {
            return;
        }
        // Per-owner key: each user's turns count toward their OWN dispatch, so a
        // busy user cannot trigger upkeep over a quiet user's memory.
        if !self.count_and_check(ScheduledOpOwner::from_context(ctx, self.trigger)) {
            return;
        }
        let submission = match &self.op {
            ResolvedOp::Pass(pass) => match Self::build_pass_submission(pass, ctx) {
                Ok(submission) => submission,
                Err(reason) => {
                    debug!("memory scheduled op: could not build pass: {reason}");
                    return;
                }
            },
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
            debug!("memory scheduled op: pass submission failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use ironclaw_extension_contracts::memory::MemoryDescriptor;
    use ironclaw_host_api::turn::TurnRunId;

    use super::*;

    /// Non-zero intervals for the tests, built where a `NonZeroU32` is needed.
    fn interval(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("test interval is non-zero")
    }

    const PASS_PROMPT: &str = "Tidy the standing memory document. \
                               Never invent, infer, or extrapolate.";

    /// A declaration exactly as a manifest carries it — parsed through the same
    /// TOML path production uses, so a test can never assert over a shape the
    /// parser would reject.
    fn declaration(interval_turns: u32, tools: &[&str], max_model_calls: u32) -> MemoryScheduledOp {
        let tool_list = tools
            .iter()
            .map(|tool| format!("\"{tool}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let toml = format!(
            r#"
lifecycle = ["read_long_term"]

[[scheduled_ops]]
trigger = "after_turn"
interval_turns = {interval_turns}
pass = {{ prompt = "prompts/memory_curation.md", tools = [{tool_list}], max_model_calls = {max_model_calls} }}
"#
        );
        let descriptor: MemoryDescriptor = toml::from_str(&toml).expect("declaration parses");
        descriptor
            .scheduled_ops
            .into_iter()
            .next()
            .expect("one declared op")
    }

    fn curation_declaration(interval_turns: u32) -> MemoryScheduledOp {
        declaration(
            interval_turns,
            &[
                "ironclaw.memory.read",
                "ironclaw.memory.search",
                "ironclaw.memory.write",
            ],
            10,
        )
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

    fn runner(interval_turns: u32) -> (MemoryScheduledOpRunner, Arc<RecordingSubmitter>) {
        let submitter = Arc::new(RecordingSubmitter::default());
        let runner = MemoryScheduledOpRunner::from_declaration(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            &curation_declaration(interval_turns),
            PASS_PROMPT,
            None,
        )
        .expect("the declaration builds a runner");
        (runner, submitter)
    }

    /// Upkeep is a periodic chore, not a per-turn one: a pass every turn would
    /// burn tokens re-reading a document that has not changed. The cadence is
    /// the DECLARATION's — nothing here picks it.
    #[tokio::test]
    async fn a_pass_is_submitted_only_every_nth_declared_turn() {
        let (runner, submitter) = runner(3);

        for _ in 0..2 {
            runner.on_turn(&ctx_for("user-a")).await;
        }
        assert_eq!(
            submitter.count(),
            0,
            "no pass before the declared interval is reached"
        );

        runner.on_turn(&ctx_for("user-a")).await;
        assert_eq!(submitter.count(), 1, "the third turn triggers a pass");

        for _ in 0..2 {
            runner.on_turn(&ctx_for("user-a")).await;
        }
        assert_eq!(submitter.count(), 1, "the counter resets after a pass");

        runner.on_turn(&ctx_for("user-a")).await;
        assert_eq!(
            submitter.count(),
            2,
            "and triggers again one interval later"
        );
    }

    /// The deployment gets the last word on cadence: a manifest declares work
    /// that runs at the deployment's expense, so an operator who set an
    /// interval must be able to slow it down (or speed it up, within the floor)
    /// without editing the provider.
    #[tokio::test]
    async fn a_configured_interval_overrides_the_declared_one() {
        let submitter = Arc::new(RecordingSubmitter::default());
        let runner = MemoryScheduledOpRunner::from_declaration(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            &curation_declaration(10),
            PASS_PROMPT,
            Some(interval(2)),
        )
        .expect("an override within the floor builds");

        runner.on_turn(&ctx_for("user-a")).await;
        assert_eq!(submitter.count(), 0);
        runner.on_turn(&ctx_for("user-a")).await;
        assert_eq!(
            submitter.count(),
            1,
            "the configured interval, not the declared 10, decides"
        );
    }

    /// The floor exists because a per-turn pass amplifies cost and latency for
    /// every user of a deployment. The manifest parser enforces it on the
    /// declaration; an operator override must not be the way around it.
    #[test]
    fn an_override_below_the_floor_is_refused() {
        let submitter = Arc::new(RecordingSubmitter::default());
        let error = MemoryScheduledOpRunner::from_declaration(
            submitter as Arc<dyn CurationPassSubmitter>,
            &curation_declaration(10),
            PASS_PROMPT,
            Some(interval(1)),
        )
        .err()
        .expect("an interval below the floor must be refused");
        assert!(
            error.contains(&MIN_SCHEDULED_OP_INTERVAL_TURNS.to_string()),
            "{error}"
        );
    }

    /// Each user's turns count toward their OWN pass. A shared counter would let
    /// a busy user trigger upkeep over a quiet user's memory.
    #[tokio::test]
    async fn counters_are_per_owner() {
        let (runner, submitter) = runner(2);

        runner.on_turn(&ctx_for("user-a")).await;
        runner.on_turn(&ctx_for("user-b")).await;
        assert_eq!(submitter.count(), 0, "one turn each is below the interval");

        runner.on_turn(&ctx_for("user-a")).await;
        assert_eq!(submitter.count(), 1);
        assert_eq!(
            submitter.last().caller.user_id.as_str(),
            "user-a",
            "the pass belongs to the user whose turns triggered it"
        );
    }

    /// What the pass actually runs comes from the DECLARATION: its prompt, its
    /// tool selection, its model-call budget. The host contributes only what a
    /// manifest must not be able to name for itself — who the pass acts as, the
    /// wall clock, the tool-traffic ceiling, and the report contract.
    #[tokio::test]
    async fn the_submitted_pass_carries_the_declaration_and_the_hosts_envelope() {
        let (runner, submitter) = runner(2);
        runner.on_turn(&ctx_for("user-a")).await;
        runner.on_turn(&ctx_for("user-a")).await;

        let submission = submitter.last();
        assert_eq!(submission.caller.user_id.as_str(), "user-a");
        assert_eq!(submission.caller.tenant_id.as_str(), "tenant-a");
        assert!(
            !submission.caller.operator_config,
            "a per-user chore must never hold deployment-wide authority"
        );

        assert_eq!(
            submission.system_prompt, PASS_PROMPT,
            "the instruction is the provider's resolved prompt asset, not host text"
        );
        let tools: Vec<&str> = submission.tools.iter().map(|id| id.as_str()).collect();
        assert_eq!(
            tools,
            vec![
                "ironclaw.memory.read",
                "ironclaw.memory.search",
                "ironclaw.memory.write"
            ],
            "the pass gets exactly the tools the declaration selected"
        );
        assert_eq!(
            submission.limits.max_model_calls,
            Some(10),
            "the declared budget is what bounds the run"
        );

        assert!(
            matches!(submission.output, OutputContract::JsonSchema { .. }),
            "the pass must report what it changed, not free text"
        );
        // Without declared ceilings a pass inherits the unbound profile's
        // 1024-iteration budget and no wall clock. Nobody is watching this run,
        // so an unconverged pass would burn tokens against a user's memory
        // until it hit that limit — the failure nobody would notice.
        assert_eq!(
            submission.limits.max_capability_invocations,
            Some(12),
            "three declared tools at four calls each — the host's derivation, not a declaration"
        );
        assert_eq!(
            submission.limits.max_wall_clock_seconds,
            Some(SCHEDULED_PASS_WALL_CLOCK_SECS)
        );
        assert!(
            !submission.limits.is_unlimited(),
            "an unwatched background pass must never run unbounded"
        );
    }

    /// A smaller declaration gets a smaller envelope, and a declaration with no
    /// tools still gets room for its own result call rather than being denied
    /// the report the host demands of it.
    #[tokio::test]
    async fn the_capability_ceiling_scales_with_the_declared_tools() {
        let submitter = Arc::new(RecordingSubmitter::default());
        let runner = MemoryScheduledOpRunner::from_declaration(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            &declaration(2, &["ironclaw.memory.read"], 4),
            PASS_PROMPT,
            None,
        )
        .expect("a one-tool declaration builds");

        runner.on_turn(&ctx_for("user-a")).await;
        runner.on_turn(&ctx_for("user-a")).await;

        assert_eq!(
            submitter.last().limits.max_capability_invocations,
            Some(4),
            "one declared tool earns one tool's worth of traffic"
        );
        assert_eq!(
            MemoryScheduledOpRunner::max_capability_calls(0),
            MIN_PASS_CAPABILITY_CALLS,
            "a pass with no tools still gets to emit its structured report"
        );
    }

    /// A crash-retry of the triggering turn must converge on the same pass
    /// rather than starting a second one over the same document.
    #[tokio::test]
    async fn the_pass_id_is_the_idempotency_key() {
        let (runner, submitter) = runner(2);
        runner.on_turn(&ctx_for("user-a")).await;
        runner.on_turn(&ctx_for("user-a")).await;

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
        let runner = MemoryScheduledOpRunner::from_declaration(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            &curation_declaration(2),
            PASS_PROMPT,
            None,
        )
        .expect("the declaration builds a runner");

        runner.on_turn(&ctx_for("user-a")).await;
        runner.on_turn(&ctx_for("user-a")).await;

        assert_eq!(submitter.count(), 1, "it tried");
    }

    /// The point fires for every terminal state, so a failed or cancelled turn
    /// arrives here too. It must not count: a turn that never finished says
    /// nothing about whether memory needs tidying, and counting it would drift
    /// the interval away from "every Nth real turn".
    #[tokio::test]
    async fn an_unsuccessful_turn_never_counts_toward_the_interval() {
        let (runner, submitter) = runner(2);

        for _ in 0..5 {
            runner.on_turn(&ctx_for_status("user-a", false)).await;
        }
        assert_eq!(submitter.count(), 0, "unsuccessful turns do not accumulate");

        runner.on_turn(&ctx_for("user-a")).await;
        runner.on_turn(&ctx_for("user-a")).await;
        assert_eq!(
            submitter.count(),
            1,
            "the interval is counted purely in successful turns"
        );
    }

    /// The factory composition wires is only useful if a turn dispatched
    /// through the built dispatcher actually reaches the runner. Testing the
    /// runner alone would leave the install arguments (point, trust class,
    /// phase) unproven — and a wrong trust class is an install-time rejection
    /// that would silently leave upkeep un-wired in production.
    #[tokio::test]
    async fn the_built_dispatcher_delivers_a_turn_to_the_runner() {
        let submitter = Arc::new(RecordingSubmitter::default());
        let dispatchers = after_turn_scheduled_op_dispatcher_factory(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            &curation_declaration(2),
            PASS_PROMPT,
            None,
        )
        .expect("the scheduled-op hook installs");

        dispatchers().dispatch_after_turn(ctx_for("user-a")).await;
        dispatchers().dispatch_after_turn(ctx_for("user-a")).await;

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
        let (runner, submitter) = runner(2);

        for _ in 0..4 {
            runner.on_turn(&ctx_for("user-a")).await;
        }

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
        let (runner, submitter) = runner(2);
        let run_id = TurnRunId::new();

        for _ in 0..4 {
            runner.on_turn(&ctx_for_run("user-a", true, run_id)).await;
        }

        let submissions = submitter.all();
        assert_eq!(submissions.len(), 2, "both retries reached the submitter");
        assert_eq!(
            submissions[0].public_id, submissions[1].public_id,
            "a replayed trigger must produce the same pass, which the accept door dedupes"
        );
    }

    /// The counters are the policy's whole state, and they must survive the
    /// per-run dispatcher churn: the executor mints a FRESH dispatcher for
    /// every terminal run (so hook poison stays run-scoped), and each one
    /// installs a fresh binding. If the factory minted a fresh RUNNER too,
    /// every turn would look like the first and no interval would ever be
    /// reached — upkeep would be silently dead.
    #[tokio::test]
    async fn counters_survive_the_per_run_dispatcher() {
        let submitter = Arc::new(RecordingSubmitter::default());
        let dispatchers = after_turn_scheduled_op_dispatcher_factory(
            Arc::clone(&submitter) as Arc<dyn CurationPassSubmitter>,
            &curation_declaration(3),
            PASS_PROMPT,
            None,
        )
        .expect("the scheduled-op hook installs");

        // Three runs, three separate dispatchers — as production dispatches.
        for _ in 0..3 {
            dispatchers().dispatch_after_turn(ctx_for("user-a")).await;
        }

        assert_eq!(
            submitter.count(),
            1,
            "the third turn triggers a pass even though each turn had its own dispatcher"
        );
    }

    /// Each mint is a genuinely separate dispatcher: poison recorded in one
    /// run's registry cannot be carried into the next run's.
    #[test]
    fn each_mint_is_a_separate_dispatcher() {
        let submitter = Arc::new(RecordingSubmitter::default());
        let dispatchers = after_turn_scheduled_op_dispatcher_factory(
            submitter as Arc<dyn CurationPassSubmitter>,
            &curation_declaration(3),
            PASS_PROMPT,
            None,
        )
        .expect("the scheduled-op hook installs");

        assert!(
            !Arc::ptr_eq(&dispatchers(), &dispatchers()),
            "a shared dispatcher would let one run's poison disable upkeep until restart"
        );
    }
}
