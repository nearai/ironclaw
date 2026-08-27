//! Memory-surface declaration vocabulary (`[memory]` in a v3 manifest).
//!
//! An extension declares `[memory]` to say "I am a backend for the host's
//! memory adapter." The bound provider's manifest is the single source of
//! truth for its memory surface: the manifest's `[[tools]]` array declares the
//! model-visible memory tools the provider serves, and `[memory].lifecycle`
//! declares which host-initiated lifecycle hooks the host may call on it. A
//! hook that is not declared is never called. Compose-time binding selects
//! exactly one memory provider (native by default), so the model's memory
//! interface stays stable while the backend swaps underneath — it is never
//! installed/removed or swapped at runtime. The concrete provider is the
//! manifest's `[runtime].service`; no connection or credential material lives
//! here (that is compose-time configuration). `[memory].scheduled_ops`
//! additionally lets a provider declare its own recurring upkeep against a
//! host-owned trigger vocabulary — the provider names the work and the cadence,
//! the host owns the clock and the authority.

use std::num::NonZeroU32;

use ironclaw_host_api::{capability_profile::CapabilityProfileSchemaRef, ids::CapabilityId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Reserved capability-id namespace for the always-on memory adapter tools.
///
/// A `[memory]`-declaring manifest may declare its tools under
/// `ironclaw.memory.*` even when its own extension id differs, so swapping the
/// bound backend never renames the model's memory tools. Trust-safe: `[memory]`
/// requires a first_party runtime, which requires a host-bundled manifest
/// source, so no installable extension can squat the namespace.
pub const MEMORY_TOOL_ID_NAMESPACE: &str = "ironclaw.memory";

/// A host-initiated memory lifecycle hook a provider participates in.
///
/// Declared in `[memory].lifecycle`. The host calls only declared hooks:
///
/// - `read_long_term` / `read_short_term` — the two retrieve-before-run
///   context lanes (general durable memory vs. the active thread's scratch),
///   each queried independently once per run.
/// - `record_interaction` — the after-turn transcript record seam, called
///   after every `Completed` run.
/// - `profile_read` — the loop-start user-profile document read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLifecycleHook {
    /// Retrieve-before-run: the user's general / durable memory lane.
    ReadLongTerm,
    /// Retrieve-before-run: the active thread's short-term scratch lane.
    ReadShortTerm,
    /// After-turn interaction recording.
    RecordInteraction,
    /// Loop-start user-profile document read.
    ProfileRead,
}

impl MemoryLifecycleHook {
    /// Stable wire token.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadLongTerm => "read_long_term",
            Self::ReadShortTerm => "read_short_term",
            Self::RecordInteraction => "record_interaction",
            Self::ProfileRead => "profile_read",
        }
    }

    /// Every hook in the vocabulary, in wire order.
    pub const ALL: [MemoryLifecycleHook; 4] = [
        Self::ReadLongTerm,
        Self::ReadShortTerm,
        Self::RecordInteraction,
        Self::ProfileRead,
    ];
}

/// Minimum turns between two invocations of one scheduled op.
///
/// A cost floor, not a tuning knob: a manifest declares work that runs on
/// someone else's deployment, at their expense. Without a floor an
/// `interval_turns = 1` declaration would demand invocation after every single
/// turn and amplify cost and latency for every user of the deployment that
/// bound the provider. Two is the smallest interval that is still
/// usage-proportional rather than per-turn.
pub const MIN_SCHEDULED_OP_INTERVAL_TURNS: u32 = 2;

/// Ceiling on `pass.max_model_calls` for one scheduled pass.
///
/// A scheduled pass is unwatched background spend — a manifest-authored prompt
/// running as every user on a schedule, with nobody reading the transcript — so
/// the manifest may size its own budget only up to a host-owned ceiling. The
/// #7770 phase-1 live test put the realistic need for a curation pass at 10;
/// 16 leaves headroom for a larger document without letting a declaration open
/// an unbounded run.
pub const MAX_SCHEDULED_PASS_MODEL_CALLS: u32 = 16;

/// The host-owned trigger vocabulary a scheduled op may bind to.
///
/// Closed on purpose: the trigger names host machinery, so a manifest selects
/// from what the host actually implements and can never name a clock the host
/// does not own. Unknown tokens fail the manifest parse rather than being
/// dropped — a silently ignored trigger would present as a provider whose
/// declared upkeep simply never runs. v0 has exactly one entry; new triggers
/// arrive only with a consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScheduledTrigger {
    /// After a conversation turn reaches a terminal state — the after-turn hook
    /// machinery, invoked once every `interval_turns` turns per owner.
    AfterTurn,
}

impl MemoryScheduledTrigger {
    /// Stable wire token.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AfterTurn => "after_turn",
        }
    }

    /// Every trigger in the vocabulary, in wire order.
    pub const ALL: [MemoryScheduledTrigger; 1] = [Self::AfterTurn];
}

/// A bounded, unbound model run built from the provider package's own assets.
///
/// The larger of the two op kinds: a manifest-authored prompt executing with
/// write tools, as every user, on a schedule. That is why pass ops are
/// trust-gated at the manifest layer (see
/// [`MemoryDescriptor::scheduled_ops`]) — the declaration selects the work,
/// the host supplies the authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryScheduledPass {
    /// Bundled asset holding the pass prompt, named exactly the way
    /// [`MemoryDescriptor::guidance_doc`] names its asset. Shape is validated
    /// here (relative, no escaping segments); RESOLUTION against the package's
    /// asset table stays host-side and fail-closed, so a declared-but-missing
    /// asset is a host-side refusal, not a parse-time one.
    pub prompt: CapabilityProfileSchemaRef,
    /// The tools the pass may call. Selection, never authority: every id must
    /// be one the SAME manifest declares in its `[[tools]]` array (enforced
    /// where the manifest-wide view exists), and each call still crosses
    /// normal capability authorization at invocation time.
    pub tools: Vec<CapabilityId>,
    /// Per-invocation model-call budget, bounded by
    /// [`MAX_SCHEDULED_PASS_MODEL_CALLS`].
    pub max_model_calls: NonZeroU32,
}

/// What a scheduled op actually does, tagged by which key the manifest entry
/// declares. Exactly one of `pass` or `tool` — never both, never neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScheduledOpKind {
    /// `pass = { … }` — a bounded model run over package assets.
    Pass(MemoryScheduledPass),
}

/// One upkeep operation a memory provider declares, bound to a host trigger.
///
/// The provider says WHAT should run and how often; the host owns the clock,
/// the invocation envelope, and the authority. Declared as a `[[memory
/// .scheduled_ops]]` array entry:
///
/// ```toml
/// [[memory.scheduled_ops]]
/// trigger = "after_turn"
/// interval_turns = 10
/// pass = { prompt = "prompts/memory_curation.md", tools = ["ironclaw.memory.read"], max_model_calls = 10 }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawMemoryScheduledOp", into = "RawMemoryScheduledOp")]
pub struct MemoryScheduledOp {
    /// The host trigger this op rides.
    pub trigger: MemoryScheduledTrigger,
    /// Turns between invocations, at least
    /// [`MIN_SCHEDULED_OP_INTERVAL_TURNS`]. `NonZeroU32` so "every 0 turns" is
    /// not even representable.
    pub interval_turns: NonZeroU32,
    /// The declared operation.
    pub op: MemoryScheduledOpKind,
}

/// The wire shape of one `[[memory.scheduled_ops]]` entry, before the op-kind
/// keys collapse into the tagged [`MemoryScheduledOpKind`].
///
/// A separate raw type so the parsed [`MemoryScheduledOp`] can carry its
/// invariants: the only way to build one from a manifest is through
/// [`TryFrom`], which rejects every entry that violates a per-op rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMemoryScheduledOp {
    trigger: MemoryScheduledTrigger,
    interval_turns: NonZeroU32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pass: Option<MemoryScheduledPass>,
    /// Recognized and rejected. Declaring the key keeps a future `tool` op from
    /// failing as an unknown field — a manifest written against the eventual
    /// schema fails with intent instead of confusion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
}

/// Per-op declaration errors, raised while parsing one
/// `[[memory.scheduled_ops]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryScheduledOpError {
    #[error(
        "scheduled tool ops are not yet supported (lands with the first wired external provider \
         — #7664)"
    )]
    ToolOpUnsupported,
    #[error("a scheduled op declares exactly one of `pass` or `tool`; this entry declares both")]
    BothOpKinds,
    #[error(
        "a scheduled op must declare exactly one of `pass` or `tool`; this entry declares neither"
    )]
    MissingOpKind,
    #[error(
        "interval_turns = {interval} is below the minimum scheduled-op interval of {floor} turns"
    )]
    IntervalBelowFloor { interval: u32, floor: u32 },
    #[error("pass.max_model_calls = {declared} exceeds the maximum of {maximum}")]
    MaxModelCallsAboveCeiling { declared: u32, maximum: u32 },
}

impl TryFrom<RawMemoryScheduledOp> for MemoryScheduledOp {
    type Error = MemoryScheduledOpError;

    fn try_from(raw: RawMemoryScheduledOp) -> Result<Self, Self::Error> {
        if raw.interval_turns.get() < MIN_SCHEDULED_OP_INTERVAL_TURNS {
            return Err(MemoryScheduledOpError::IntervalBelowFloor {
                interval: raw.interval_turns.get(),
                floor: MIN_SCHEDULED_OP_INTERVAL_TURNS,
            });
        }
        let op = match (raw.pass, raw.tool) {
            (Some(pass), None) => {
                if pass.max_model_calls.get() > MAX_SCHEDULED_PASS_MODEL_CALLS {
                    return Err(MemoryScheduledOpError::MaxModelCallsAboveCeiling {
                        declared: pass.max_model_calls.get(),
                        maximum: MAX_SCHEDULED_PASS_MODEL_CALLS,
                    });
                }
                MemoryScheduledOpKind::Pass(pass)
            }
            (None, Some(_)) => return Err(MemoryScheduledOpError::ToolOpUnsupported),
            (Some(_), Some(_)) => return Err(MemoryScheduledOpError::BothOpKinds),
            (None, None) => return Err(MemoryScheduledOpError::MissingOpKind),
        };
        Ok(Self {
            trigger: raw.trigger,
            interval_turns: raw.interval_turns,
            op,
        })
    }
}

impl From<MemoryScheduledOp> for RawMemoryScheduledOp {
    fn from(op: MemoryScheduledOp) -> Self {
        let (pass, tool) = match op.op {
            MemoryScheduledOpKind::Pass(pass) => (Some(pass), None),
        };
        Self {
            trigger: op.trigger,
            interval_turns: op.interval_turns,
            pass,
            tool,
        }
    }
}

/// Descriptor-level errors — rules that need the whole `[memory]` section but
/// nothing outside it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryDescriptorError {
    #[error(
        "[memory] declares more than one scheduled op for trigger `{trigger}`; at most one op \
         per trigger is supported"
    )]
    DuplicateScheduledTrigger { trigger: &'static str },
}

/// The `[memory]` surface of a v3 manifest: the extension is a memory provider
/// (a backend for the host memory adapter) and declares the host-initiated
/// lifecycle hooks it participates in. `lifecycle` may be empty or absent — a
/// provider that declares `[memory]` with no lifecycle contributes its
/// declared tools only and is never called on any host-initiated hook.
/// Parsing/validation is fail-closed: unknown fields and unknown lifecycle
/// tokens are rejected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryDescriptor {
    /// Host-initiated lifecycle hooks this provider participates in. Any
    /// subset, including empty. An undeclared hook is NEVER called.
    #[serde(default)]
    pub lifecycle: Vec<MemoryLifecycleHook>,
    /// Bundled asset holding the provider's memory guidance for the model —
    /// when to save a durable fact, how to phrase it, what never to save — to
    /// be appended to the system prompt while this provider is the bound one
    /// (#7185).
    ///
    /// Provider-owned on purpose. The guidance names this provider's own tools
    /// and describes its own recall behavior, so it belongs beside the manifest
    /// that declares them, not in the loop tier: a backend whose recall is
    /// search-first needs to tell the model something different from one that
    /// serves a standing document, and neither should have to edit host code to
    /// say it.
    ///
    /// Optional and fail-quiet: absent means this provider ships no guidance
    /// and nothing is appended. It is NOT a lifecycle hook — guidance is static
    /// text, not a call — so it is declared here rather than in `lifecycle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance_doc: Option<CapabilityProfileSchemaRef>,
    /// Upkeep operations this provider declares, each bound to a host trigger.
    ///
    /// The provider names its own work and its own cadence; the host owns the
    /// clock, the invocation envelope, and the authority — declaration is
    /// selection, never authority. Empty by default and absent-compatible: a
    /// manifest written before this field existed parses unchanged and
    /// schedules nothing.
    ///
    /// Two rules cannot be checked from the section alone and are enforced by
    /// the manifest layer, which can see the whole file: a pass op's `tools`
    /// must be a subset of the SAME manifest's declared `[[tools]]` ids (a
    /// memory provider must not schedule passes wielding another extension's
    /// tools), and only a first-party/trusted manifest may declare a pass op at
    /// all — the same default-deny wall the after-turn hook tiers use, for the
    /// same reason: a pass is a manifest-authored prompt running with write
    /// tools as every user, on a schedule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scheduled_ops: Vec<MemoryScheduledOp>,
}

impl MemoryDescriptor {
    /// Whether this provider declares the given lifecycle hook.
    pub fn declares(&self, hook: MemoryLifecycleHook) -> bool {
        self.lifecycle.contains(&hook)
    }

    /// Validate the rules that need the whole section but nothing outside it.
    ///
    /// At most one scheduled op per trigger: the host holds one interval
    /// counter per trigger per owner, so a second op on the same trigger has no
    /// well-defined cadence. Rejecting it keeps v0 honest rather than picking a
    /// winner silently.
    pub fn validate(&self) -> Result<(), MemoryDescriptorError> {
        let mut seen: Vec<MemoryScheduledTrigger> = Vec::with_capacity(self.scheduled_ops.len());
        for scheduled in &self.scheduled_ops {
            if seen.contains(&scheduled.trigger) {
                return Err(MemoryDescriptorError::DuplicateScheduledTrigger {
                    trigger: scheduled.trigger.as_str(),
                });
            }
            seen.push(scheduled.trigger);
        }
        Ok(())
    }

    /// The scheduled op declared for `trigger`, if any. At most one exists
    /// once [`MemoryDescriptor::validate`] has passed.
    pub fn scheduled_op(&self, trigger: MemoryScheduledTrigger) -> Option<&MemoryScheduledOp> {
        self.scheduled_ops
            .iter()
            .find(|scheduled| scheduled.trigger == trigger)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire tokens are the contract: manifests spell hooks exactly this
    /// way, and `as_str` must match the serde snake_case output.
    #[test]
    fn lifecycle_hook_wire_tokens_are_stable() {
        for (hook, token) in [
            (MemoryLifecycleHook::ReadLongTerm, "read_long_term"),
            (MemoryLifecycleHook::ReadShortTerm, "read_short_term"),
            (MemoryLifecycleHook::RecordInteraction, "record_interaction"),
            (MemoryLifecycleHook::ProfileRead, "profile_read"),
        ] {
            assert_eq!(hook.as_str(), token);
            let serialized = serde_json::to_string(&hook).expect("serialize hook");
            assert_eq!(serialized, format!("\"{token}\""));
            let back: MemoryLifecycleHook =
                serde_json::from_str(&serialized).expect("deserialize hook");
            assert_eq!(back, hook);
        }
    }

    #[test]
    fn unknown_lifecycle_token_fails_closed() {
        assert!(serde_json::from_str::<MemoryLifecycleHook>("\"on_boot\"").is_err());
    }

    #[test]
    fn descriptor_defaults_to_an_empty_lifecycle() {
        let descriptor: MemoryDescriptor = serde_json::from_str("{}").expect("empty descriptor");
        assert!(descriptor.lifecycle.is_empty());
        for hook in MemoryLifecycleHook::ALL {
            assert!(!descriptor.declares(hook));
        }
    }

    #[test]
    fn declares_reflects_the_declared_set() {
        let descriptor: MemoryDescriptor =
            serde_json::from_str(r#"{"lifecycle": ["read_long_term", "profile_read"]}"#)
                .expect("descriptor parses");
        assert!(descriptor.declares(MemoryLifecycleHook::ReadLongTerm));
        assert!(descriptor.declares(MemoryLifecycleHook::ProfileRead));
        assert!(!descriptor.declares(MemoryLifecycleHook::ReadShortTerm));
        assert!(!descriptor.declares(MemoryLifecycleHook::RecordInteraction));
    }

    /// Guidance is optional and absent by default. A provider that ships none
    /// must not be treated as declaring an empty one: the host appends nothing
    /// rather than an empty section, so the distinction has to survive parsing.
    #[test]
    fn guidance_doc_is_absent_unless_declared() {
        let without: MemoryDescriptor =
            serde_json::from_str(r#"{"lifecycle": ["read_long_term"]}"#)
                .expect("descriptor parses");
        assert!(without.guidance_doc.is_none());

        let with: MemoryDescriptor =
            serde_json::from_str(r#"{"guidance_doc": "prompts/memory-guidance.md"}"#)
                .expect("descriptor parses");
        assert_eq!(
            with.guidance_doc.as_ref().map(|doc| doc.as_str()),
            Some("prompts/memory-guidance.md")
        );
        assert!(
            with.lifecycle.is_empty(),
            "guidance is static text, not a hook: declaring it must not imply any lifecycle"
        );
    }

    /// The ref is a validated bundled-asset path, the same type the tool
    /// surface uses for `prompt_doc_ref`. An absolute or escaping path must
    /// fail the manifest parse rather than reach an asset lookup.
    #[test]
    fn guidance_doc_rejects_a_path_outside_the_package() {
        for bad in ["/etc/passwd", "../../secrets.md"] {
            let raw = format!(r#"{{"guidance_doc": "{bad}"}}"#);
            assert!(
                serde_json::from_str::<MemoryDescriptor>(&raw).is_err(),
                "{bad:?} must not parse as a bundled asset ref"
            );
        }
    }

    #[test]
    fn descriptor_rejects_unknown_fields() {
        assert!(serde_json::from_str::<MemoryDescriptor>(r#"{"hooks": []}"#).is_err());
    }

    /// Parses the documented `[memory]` TOML the same way manifest ingestion
    /// does, so the array-of-tables spelling is what the tests below exercise.
    fn parse_memory_section(toml_body: &str) -> Result<MemoryDescriptor, toml::de::Error> {
        toml::from_str::<MemoryDescriptor>(toml_body)
    }

    const VALID_PASS_OP: &str = r#"
lifecycle = ["record_interaction"]

[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = { prompt = "prompts/memory_curation.md", tools = ["ironclaw.memory.read", "ironclaw.memory.write"], max_model_calls = 10 }
"#;

    /// The documented declaration must survive a parse and a re-serialize with
    /// every field intact: leg B reads these values to build the invocation.
    #[test]
    fn pass_op_round_trips_through_the_declared_shape() {
        let descriptor = parse_memory_section(VALID_PASS_OP).expect("descriptor parses");
        descriptor.validate().expect("section is valid");

        let scheduled = descriptor
            .scheduled_op(MemoryScheduledTrigger::AfterTurn)
            .expect("after_turn op declared");
        assert_eq!(scheduled.interval_turns.get(), 10);
        let MemoryScheduledOpKind::Pass(pass) = &scheduled.op;
        assert_eq!(pass.prompt.as_str(), "prompts/memory_curation.md");
        assert_eq!(
            pass.tools
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["ironclaw.memory.read", "ironclaw.memory.write"]
        );
        assert_eq!(pass.max_model_calls.get(), 10);

        let json = serde_json::to_string(&descriptor).expect("serialize descriptor");
        let back: MemoryDescriptor = serde_json::from_str(&json).expect("descriptor round-trips");
        assert_eq!(back, descriptor);
    }

    /// v3 compatibility: `scheduled_ops` is additive. Every manifest written
    /// before the field existed must keep parsing, and must schedule nothing —
    /// absent is not "declare an empty op", it is "declare no upkeep".
    #[test]
    fn scheduled_ops_absent_in_an_older_manifest_means_none() {
        let descriptor = parse_memory_section(
            r#"
lifecycle = ["read_long_term", "record_interaction"]
guidance_doc = "prompts/memory-guidance.md"
"#,
        )
        .expect("pre-scheduled_ops descriptor parses");
        assert!(descriptor.scheduled_ops.is_empty());
        descriptor.validate().expect("section is valid");
        for trigger in MemoryScheduledTrigger::ALL {
            assert!(descriptor.scheduled_op(trigger).is_none());
        }
    }

    /// The trigger vocabulary is host-owned and closed. An unrecognized token
    /// must fail the parse: silently dropping it would present as a provider
    /// whose declared upkeep never runs.
    #[test]
    fn unknown_trigger_fails_the_parse() {
        let error = parse_memory_section(
            r#"
[[scheduled_ops]]
trigger = "at_midnight"
interval_turns = 10
pass = { prompt = "prompts/p.md", tools = [], max_model_calls = 4 }
"#,
        )
        .expect_err("unknown trigger must not parse");
        assert!(
            error.to_string().contains("at_midnight"),
            "error must name the rejected token: {error}"
        );
    }

    /// `tool` ops are recognized-and-rejected rather than unknown: a manifest
    /// written against the eventual schema must fail with intent.
    #[test]
    fn tool_op_is_recognized_and_rejected_with_intent() {
        let error = parse_memory_section(
            r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
tool = "memory_run_maintenance"
"#,
        )
        .expect_err("tool ops are not supported yet");
        let rendered = error.to_string();
        assert!(
            rendered.contains("scheduled tool ops are not yet supported"),
            "must reject with the intentional message, not an unknown-key error: {rendered}"
        );
        assert!(
            !rendered.contains("unknown field"),
            "the key must be recognized, not unknown: {rendered}"
        );
    }

    /// Exactly one op kind per entry. Both keys is ambiguous; neither declares
    /// a cadence with nothing to run.
    #[test]
    fn an_entry_must_declare_exactly_one_op_kind() {
        let both = parse_memory_section(
            r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
tool = "memory_run_maintenance"
pass = { prompt = "prompts/p.md", tools = [], max_model_calls = 4 }
"#,
        )
        .expect_err("both op kinds must be rejected");
        assert!(
            both.to_string().contains("declares both"),
            "both-keys error must say so: {both}"
        );

        let neither = parse_memory_section(
            r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
"#,
        )
        .expect_err("an entry with no op kind must be rejected");
        assert!(
            neither.to_string().contains("declares neither"),
            "no-op-kind error must say so: {neither}"
        );
    }

    /// The cost floor is the reason this rule exists — a manifest must not be
    /// able to demand per-turn invocation on someone else's deployment — so the
    /// error has to name the floor an author must clear.
    #[test]
    fn interval_below_the_cost_floor_is_rejected_and_names_it() {
        let error = parse_memory_section(
            r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 1
pass = { prompt = "prompts/p.md", tools = [], max_model_calls = 4 }
"#,
        )
        .expect_err("interval 1 must be rejected");
        let rendered = error.to_string();
        assert!(
            rendered.contains(&MIN_SCHEDULED_OP_INTERVAL_TURNS.to_string()),
            "error must name the floor: {rendered}"
        );
    }

    /// "Every 0 turns" must not even be representable — `NonZeroU32` rejects it
    /// before any interval rule runs.
    #[test]
    fn interval_zero_is_not_representable() {
        assert!(
            parse_memory_section(
                r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 0
pass = { prompt = "prompts/p.md", tools = [], max_model_calls = 4 }
"#,
            )
            .is_err()
        );
    }

    /// A scheduled pass is unwatched background spend, so the manifest sizes
    /// its budget only up to the host's ceiling.
    #[test]
    fn max_model_calls_above_the_ceiling_is_rejected() {
        let error = parse_memory_section(&format!(
            r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = {{ prompt = "prompts/p.md", tools = [], max_model_calls = {} }}
"#,
            MAX_SCHEDULED_PASS_MODEL_CALLS + 1
        ))
        .expect_err("over-ceiling budget must be rejected");
        assert!(
            error
                .to_string()
                .contains(&MAX_SCHEDULED_PASS_MODEL_CALLS.to_string()),
            "error must name the ceiling: {error}"
        );

        assert!(
            parse_memory_section(&format!(
                r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = {{ prompt = "prompts/p.md", tools = [], max_model_calls = {MAX_SCHEDULED_PASS_MODEL_CALLS} }}
"#,
            ))
            .is_ok(),
            "the ceiling itself must be accepted"
        );
    }

    /// The host holds one interval counter per trigger, so a second op on the
    /// same trigger has no well-defined cadence — reject rather than pick one.
    #[test]
    fn duplicate_trigger_is_rejected() {
        let descriptor = parse_memory_section(
            r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = { prompt = "prompts/a.md", tools = [], max_model_calls = 4 }

[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 20
pass = { prompt = "prompts/b.md", tools = [], max_model_calls = 4 }
"#,
        )
        .expect("two well-formed entries parse");
        assert_eq!(
            descriptor.validate(),
            Err(MemoryDescriptorError::DuplicateScheduledTrigger {
                trigger: "after_turn"
            })
        );
    }

    /// The prompt ref is the same validated bundled-asset type `guidance_doc`
    /// uses: an escaping or absolute path must fail the manifest parse rather
    /// than reach a host-side asset lookup.
    #[test]
    fn pass_prompt_rejects_a_path_outside_the_package() {
        for bad in ["/etc/passwd", "../../secrets.md"] {
            assert!(
                parse_memory_section(&format!(
                    r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = {{ prompt = "{bad}", tools = [], max_model_calls = 4 }}
"#,
                ))
                .is_err(),
                "{bad:?} must not parse as a bundled asset ref"
            );
        }
    }

    #[test]
    fn scheduled_op_rejects_unknown_fields() {
        assert!(
            parse_memory_section(
                r#"
[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
interval_seconds = 30
pass = { prompt = "prompts/p.md", tools = [], max_model_calls = 4 }
"#,
            )
            .is_err()
        );
    }

    #[test]
    fn scheduled_trigger_wire_token_is_stable() {
        assert_eq!(MemoryScheduledTrigger::AfterTurn.as_str(), "after_turn");
        assert_eq!(
            serde_json::to_string(&MemoryScheduledTrigger::AfterTurn).expect("serialize trigger"),
            "\"after_turn\""
        );
    }
}
