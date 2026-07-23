//! Slice-C kernel capability vocabulary — the one invocation payload.
//!
//! This module lands the first types of the capability-path DTO collapse
//! described in `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`
//! (§3 "one payload, authority as a fold"; §3.1 "the three real states, named";
//! §5.2.1 "origin is part of the `Invocation`"). Per the migration plan (§9), the
//! kernel vocabulary lands in `ironclaw_host_api` *first*, ahead of any wiring —
//! subsequent slices thread `&Invocation` through the four capability mediators
//! and retire the mirror request DTOs.
//!
//! ## What [`Invocation`] replaces
//!
//! The retired path re-wrapped a single capability call through ~5
//! near-identical request shapes across the crate graph (§1.1):
//! `CapabilityInvocation` (`ironclaw_turns`), `RuntimeCapabilityRequest`
//! (`ironclaw_host_runtime`), `CapabilityInvocationRequest`
//! (`ironclaw_capabilities`) and `RuntimeAdapterRequest` (`ironclaw_dispatcher`).
//! The live names are `LoopRequest`, runtime tuple parts, direct
//! `CapabilityHost` parameters, and the private runtime-lane request.
//! The field-level diff shows only
//! **three** genuinely distinct states; the rest is duplication forced by the
//! dependency DAG plus dead transitional fields. `Invocation` is the middle state —
//! *the host-side payload, resolved at the membrane* — and lives here, the bottom
//! crate everyone already depends on, so both upper and lower crates reference the
//! one definition (Golden Boundary #1: `host_api` stays vocabulary-only).
//!
//! ## Coexistence during migration (§9)
//!
//! `Invocation` is introduced **additively**: the five request DTOs above still
//! exist and are still wired. The doc's plan is explicit that the type count rises
//! before it falls (~14 → ~18 → ~11) while the new vocabulary and the old shapes
//! coexist; the mirror-DTO ratchet's frozen allowlist is what will make the old
//! shapes "may only disappear".
//!
//! These vocabulary types are now consumed on the live path (the wiring slice has
//! landed): [`InvocationOrigin`] is sealed at the membrane and read by the
//! capability authorization fold ([`crate::ExecutionContext::resolved_origin`],
//! the `origin`→gate matrix) and by the first-party trigger-mutation policy that
//! denies `ScheduledLoopRun`, and [`Invocation`]/[`Actor`] back the
//! `ironclaw_capabilities` `authorize()` path. The retired shapes still coexist,
//! but the earlier "nothing in this module is wired into the dispatch path yet"
//! note no longer holds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ActivityId, CapabilityId, CorrelationId, ProcessId, ProductKind, ResourceEstimate,
    ResourceScope, RoutineId, RunId, UserId,
};

/// Where a capability invocation originated — sealed at the membrane, exactly
/// like `actor` and `scope` (§5.2.1).
///
/// Each entry point can mint only its own variant: the loop host mints
/// [`InvocationOrigin::LoopRun`], product ingress mints
/// [`InvocationOrigin::Product`], and the routine/heartbeat scheduler mints
/// [`InvocationOrigin::Automation`] — none can claim another's origin. The single
/// `authorize()` fold consults `origin` to pick the per-descriptor gate policy
/// (the origin→gate matrix, §5.2.1): gate-by-default for model-initiated
/// `LoopRun` calls, direct-user consent semantics for `Product`, and the routine's
/// own budget/policy for `Automation`.
///
/// `LoopRun` carries [`RunId`] — this crate's prompt-visible loop turn-run
/// identity (the `TurnRunId` the design doc names is `ironclaw_turns`' higher-level
/// alias for the same run; `host_api` cannot depend on `turns`, so the run identity
/// is modeled here as `RunId`, matching [`crate::ExecutionContext::run_id`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOrigin {
    /// Model-initiated, trust-attenuated: a tool call from inside an agent loop
    /// turn-run. Gated by default (§5.2.1).
    LoopRun(RunId),
    /// Model-initiated call from a host-trusted scheduled routine/trigger run.
    /// It shares loop-run gate semantics with [`Self::LoopRun`] while keeping
    /// the scheduled lineage typed for stricter capability policy.
    ScheduledLoopRun(RunId),
    /// A direct, authenticated user action from a product surface (settings
    /// mutation, admin action). The user's gesture is consent evidence bound to
    /// this `(capability, input)` pair, honored per the descriptor's matrix.
    Product(ProductKind),
    /// Routine / heartbeat / scheduled work: autonomous but not model-initiated,
    /// metered against the owning routine's budget (§5.3.3).
    Automation(RoutineId),
}

impl InvocationOrigin {
    /// Stable discriminant string for logs and per-origin accounting views,
    /// without matching on the variant. Matches the serde tag.
    pub fn kind(&self) -> &'static str {
        match self {
            InvocationOrigin::LoopRun(_) => "loop_run",
            InvocationOrigin::ScheduledLoopRun(_) => "scheduled_loop_run",
            InvocationOrigin::Product(_) => "product",
            InvocationOrigin::Automation(_) => "automation",
        }
    }
}

/// The authenticated actor an [`Invocation`] runs under, sealed at the membrane
/// (§5.2.1). Modeled as an explicit two-variant type (not `Option<UserId>`) so
/// no consumer can silently treat an actor-less system/one-shot context as a
/// user: the distinction between "sealed to a specific human" and "no human
/// actor" is unignorable at every `match`. Forge-resistance still comes from the
/// membrane sealing this — a caller cannot mint [`Actor::Sealed`] for a human it
/// did not authenticate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    /// A specific authenticated human, sealed at the membrane.
    Sealed(UserId),
    /// No authenticated human actor: a system service or one-shot product
    /// invocation. Policy must treat it as its own class, never as a wildcard or
    /// a stand-in user.
    System,
}

impl Actor {
    /// The sealed human, if any; `None` for [`Actor::System`]. Consumers that
    /// need the acting user must handle the `None` (system) case explicitly.
    pub fn user_id(&self) -> Option<&UserId> {
        match self {
            Actor::Sealed(user_id) => Some(user_id),
            Actor::System => None,
        }
    }

    /// Stable discriminant string for logs / per-actor accounting views.
    pub fn kind(&self) -> &'static str {
        match self {
            Actor::Sealed(_) => "sealed",
            Actor::System => "system",
        }
    }
}

/// The host-side capability payload — resolved at the membrane and consumed as
/// the input of `authorize()`, referenced by every layer below it (§3, §4.1).
///
/// This is the "one payload" the DTO collapse is built around: the fields never
/// change shape as the invocation moves down the stack. Extra per-layer context is
/// threaded by reference (`&Invocation`) rather than by re-wrapping.
///
/// It carries the caller-set pre-authorization facts `authorize()` needs and that
/// are neither derivable nor authorization outputs — `actor`, `origin`,
/// `correlation_id`, and the spawn-lineage `process_id`/`parent_process_id` — but
/// **omits `mounts`, `grants`, `trust`, and `resource_reservation`**: trust,
/// grants, and mounts are *derived or produced by* `authorize()`, and
/// mounts/reservation are authorization *outputs* that live on the sealed
/// [`crate::Authorized`] witness, never on the request. `Invocation` is the
/// pre-auth input; [`crate::Authorized`] is the post-auth witness.
///
/// This is an in-process payload (`input` is arbitrary JSON, not `Eq`), so it
/// derives `PartialEq` but not `Eq` and is not itself a wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    /// Idempotency identity of this invocation (§11.3). Stable across retries.
    pub activity_id: ActivityId,
    /// The capability being invoked.
    pub capability: CapabilityId,
    /// Deref'd request input. The loop expresses input by reference; the membrane
    /// resolves the reference to the raw value carried here.
    pub input: Value,
    /// The authority envelope (tenant/user/project/... identity) this invocation
    /// runs under.
    pub scope: ResourceScope,
    /// The authenticated actor, or an explicit system/one-shot actor — sealed at
    /// the membrane (§5.2.1).
    pub actor: Actor,
    /// Where the call came from — the only fact the kernel consults about origin.
    pub origin: InvocationOrigin,
    /// Host-derived resource estimate, consumed by `authorize()` at reservation
    /// (§5.3.3). Never model-supplied.
    pub estimate: ResourceEstimate,
    /// Correlation identity for this invocation — a distinct identity from
    /// `activity_id`, restored across an auth-resume so gate-resume correlation
    /// stays continuous (never re-minted on resume).
    pub correlation_id: CorrelationId,
    /// Owning process for a spawn-lineage invocation; `None` for the common
    /// (non-process) case.
    pub process_id: Option<ProcessId>,
    /// Parent process in the spawn lineage; `None` at the root.
    pub parent_process_id: Option<ProcessId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InvocationId;

    fn sample_scope() -> ResourceScope {
        ResourceScope::local_default(UserId::new("user1").unwrap(), InvocationId::new()).unwrap()
    }

    #[test]
    fn invocation_origin_serde_is_snake_case_tagged_and_roundtrips() {
        let run = RunId::new();
        let origin = InvocationOrigin::LoopRun(run);
        let json = serde_json::to_value(&origin).unwrap();
        // Externally-tagged newtype variant with snake_case tag.
        assert_eq!(json, serde_json::json!({ "loop_run": run.to_string() }));
        let back: InvocationOrigin = serde_json::from_value(json).unwrap();
        assert_eq!(back, origin);

        let scheduled_run = RunId::new();
        let scheduled = InvocationOrigin::ScheduledLoopRun(scheduled_run);
        let json = serde_json::to_value(&scheduled).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "scheduled_loop_run": scheduled_run.to_string() })
        );
        let back: InvocationOrigin = serde_json::from_value(json).unwrap();
        assert_eq!(back, scheduled);

        let product = InvocationOrigin::Product(ProductKind::new("settings").unwrap());
        assert_eq!(
            serde_json::to_value(&product).unwrap(),
            serde_json::json!({ "product": "settings" })
        );

        let automation = InvocationOrigin::Automation(RoutineId::new("heartbeat").unwrap());
        assert_eq!(
            serde_json::to_value(&automation).unwrap(),
            serde_json::json!({ "automation": "heartbeat" })
        );
    }

    #[test]
    fn invocation_origin_kind_matches_serde_tag() {
        // The discriminant helper must not drift from the wire tag — per-origin
        // accounting views (§5.3.3) key on it.
        for (origin, tag) in [
            (InvocationOrigin::LoopRun(RunId::new()), "loop_run"),
            (
                InvocationOrigin::ScheduledLoopRun(RunId::new()),
                "scheduled_loop_run",
            ),
            (
                InvocationOrigin::Product(ProductKind::new("chat").unwrap()),
                "product",
            ),
            (
                InvocationOrigin::Automation(RoutineId::new("nightly").unwrap()),
                "automation",
            ),
        ] {
            let wire = serde_json::to_value(&origin).unwrap();
            let tag_on_wire = wire.as_object().unwrap().keys().next().unwrap().clone();
            assert_eq!(origin.kind(), tag);
            assert_eq!(tag_on_wire, tag);
        }
    }

    #[test]
    fn actor_kind_matches_serde_tag_and_user_id_accessor() {
        // Same drift guard as `invocation_origin_kind_matches_serde_tag`, plus the
        // `user_id()` accessor: `Sealed` must yield its user, `System` must yield
        // `None` (never a stand-in user).
        let sealed = Actor::Sealed(UserId::new("user1").unwrap());
        assert_eq!(sealed.kind(), "sealed");
        assert_eq!(sealed.user_id(), Some(&UserId::new("user1").unwrap()));
        assert_eq!(
            serde_json::to_value(&sealed).unwrap(),
            serde_json::json!({ "sealed": "user1" })
        );

        assert_eq!(Actor::System.kind(), "system");
        assert_eq!(Actor::System.user_id(), None);
        assert_eq!(serde_json::to_value(&Actor::System).unwrap(), "system");
    }

    #[test]
    fn origin_id_newtypes_reject_invalid_and_accept_valid() {
        // Assert the specific rejection (kind + reason), not just is_err(), so
        // an infrastructure failure can't masquerade as a validation pass.
        let empty = ProductKind::new("").unwrap_err().to_string();
        assert!(
            empty.contains("product") && empty.contains("must not be empty"),
            "unexpected rejection: {empty}"
        );
        let empty_routine = RoutineId::new("").unwrap_err().to_string();
        assert!(
            empty_routine.contains("routine") && empty_routine.contains("must not be empty"),
            "unexpected rejection: {empty_routine}"
        );
        // Uppercase-leading is rejected by the name-segment validator.
        let upper = ProductKind::new("Settings").unwrap_err().to_string();
        assert!(upper.contains("product"), "unexpected rejection: {upper}");
        assert!(ProductKind::new("settings").is_ok());
        assert!(RoutineId::new("heartbeat.30m").is_ok());
    }

    #[test]
    fn activity_id_is_a_stable_carried_identity() {
        // Idempotency turns on carrying the SAME id across a retry, so a parsed /
        // reconstructed id must equal its origin (not a fresh mint).
        let id = ActivityId::new();
        let reparsed = ActivityId::parse(&id.to_string()).unwrap();
        assert_eq!(id, reparsed);
        assert_eq!(ActivityId::from_uuid(id.as_uuid()), id);
    }

    #[test]
    fn invocation_carries_one_payload_for_each_origin() {
        for origin in [
            InvocationOrigin::LoopRun(RunId::new()),
            InvocationOrigin::Product(ProductKind::new("settings").unwrap()),
            InvocationOrigin::Automation(RoutineId::new("heartbeat").unwrap()),
        ] {
            let kind = origin.kind();
            let inv = Invocation {
                activity_id: ActivityId::new(),
                capability: CapabilityId::new("shell.exec").unwrap(),
                input: serde_json::json!({ "cmd": "echo hi" }),
                scope: sample_scope(),
                actor: Actor::Sealed(UserId::new("user1").unwrap()),
                origin,
                estimate: ResourceEstimate::default(),
                correlation_id: CorrelationId::new(),
                process_id: None,
                parent_process_id: None,
            };
            // The payload shape is identical across origins — origin is one field,
            // not a parallel type (§3.1, Mechanisms 2 & 4 dissolve).
            assert_eq!(inv.origin.kind(), kind);
            assert_eq!(inv.capability.as_str(), "shell.exec");
            // Clone-equality holds (in-process payload, PartialEq).
            assert_eq!(inv.clone(), inv);
        }
    }
}
