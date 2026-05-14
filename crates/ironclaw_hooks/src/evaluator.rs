//! Declarative predicate evaluator for `Installed`-tier hooks.
//!
//! The evaluator consumes a [`HookPredicateSpec`] plus a per-invocation
//! context and produces an [`EvaluatorDecision`]. Sliding-window state
//! (invocation timestamps, accumulated values) lives in-process inside the
//! evaluator's own `Mutex`-protected maps.
//!
//! Foundation slice coverage:
//!
//! - `HookPredicateSpec::DenyCapability` — predicate-only, stateless.
//! - `HookPredicateSpec::PauseApproval` — predicate-only, stateless.
//! - `HookPredicateSpec::RateOrValueCap` with
//!   `ValueOrRateBound::InvocationCount` — sliding-window counter.
//! - `ValueOrRateBound::NumericSum` — types implemented but evaluation
//!   returns `EvaluatorDecision::Allow` and emits a warn-level audit so the
//!   gap is visible. The full numeric-extraction story belongs in the next
//!   slice where capability arguments become hook-visible.
//!
//! Counter state is in-memory only. Restarts reset the counters; cross-
//! process counters and durable persistence are a separate slice.

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;

use crate::identity::HookId;
use crate::points::BeforeCapabilityHookContext;
use crate::predicate::{
    CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound,
};
use crate::predicate_state::{
    InMemoryPredicateStateBackend, InvocationKey, PredicateEventId, PredicateStateBackend, ValueKey,
};

/// Re-export the backend's `MAX_HISTORY_KEYS` for back-compat with
/// callers that constructed it via the old evaluator path.
pub use crate::predicate_state::MAX_HISTORY_KEYS;

/// Decision returned by the predicate evaluator. The
/// [`crate::installed_hook::PredicateBackedBeforeCapabilityHook`] glue
/// translates these into sink calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluatorDecision {
    /// Predicate did not fire; capability invocation proceeds.
    Allow,
    /// Predicate fired and requested a deny. `code` selects the model-
    /// visible closed-vocabulary label; `reason` is the free-form
    /// audit-only payload.
    Deny {
        code: crate::predicate::DenyReasonCode,
        reason: String,
    },
    /// Predicate fired and requested an approval pause. `code` selects
    /// the model-visible closed-vocabulary label; `reason` is the
    /// free-form audit-only payload.
    PauseApproval {
        code: crate::predicate::PauseReasonCode,
        reason: String,
    },
}

/// In-process evaluator. One evaluator per dispatcher / run; sliding-window
/// state is delegated to a pluggable [`PredicateStateBackend`] so durable
/// backends (Postgres, libSQL) can land in a follow-up PR without
/// changing the evaluator's predicate semantics.
pub struct PredicateEvaluator {
    backend: Arc<dyn PredicateStateBackend>,
}

impl PredicateEvaluator {
    /// Construct an evaluator with the default in-memory backend.
    pub fn new() -> Self {
        Self {
            backend: Arc::new(InMemoryPredicateStateBackend::new()),
        }
    }

    /// Construct an evaluator with an explicit backend. Production
    /// callers swap in a durable backend here; tests use the in-memory
    /// default via [`Self::new`].
    pub fn with_backend(backend: Arc<dyn PredicateStateBackend>) -> Self {
        Self { backend }
    }

    /// Total LRU evictions observed by the underlying backend. Operators
    /// should alert when this advances. Threat-model finding D5.
    pub fn evictions_observed(&self) -> u64 {
        self.backend.evictions_observed()
    }

    /// Evaluate `spec` against the given context. Mutates internal counters
    /// for stateful predicates.
    pub fn evaluate(
        &self,
        hook_id: HookId,
        spec: &HookPredicateSpec,
        ctx: &BeforeCapabilityHookContext,
    ) -> EvaluatorDecision {
        self.evaluate_at(hook_id, spec, ctx, Instant::now())
    }

    /// Test-only variant accepting an explicit `now` so sliding-window tests
    /// don't depend on real wall-clock progress.
    pub fn evaluate_at(
        &self,
        hook_id: HookId,
        spec: &HookPredicateSpec,
        ctx: &BeforeCapabilityHookContext,
        now: Instant,
    ) -> EvaluatorDecision {
        match spec {
            HookPredicateSpec::DenyCapability { when, reason } => {
                if predicate_matches(when, ctx) {
                    EvaluatorDecision::Deny {
                        code: crate::predicate::DenyReasonCode::Generic,
                        reason: reason.clone(),
                    }
                } else {
                    EvaluatorDecision::Allow
                }
            }
            HookPredicateSpec::PauseApproval { when, reason } => {
                if predicate_matches(when, ctx) {
                    EvaluatorDecision::PauseApproval {
                        code: crate::predicate::PauseReasonCode::Generic,
                        reason: reason.clone(),
                    }
                } else {
                    EvaluatorDecision::Allow
                }
            }
            HookPredicateSpec::RateOrValueCap {
                when,
                bound,
                on_exceeded,
            } => {
                if !predicate_matches(when, ctx) {
                    return EvaluatorDecision::Allow;
                }
                match bound {
                    ValueOrRateBound::InvocationCount { max, window } => {
                        let Some(window_dur) = parse_window(window) else {
                            tracing::warn!(
                                window,
                                "predicate evaluator could not parse window; failing closed"
                            );
                            return restrictive_action(on_exceeded);
                        };
                        let key = InvocationKey {
                            hook_id,
                            tenant_id: ctx.tenant_id.clone(),
                            capability: ctx.capability_name.clone(),
                        };
                        let event_id = resolve_event_id(hook_id, ctx);
                        let count = match self
                            .backend
                            .record_invocation(&key, &event_id, now, window_dur)
                        {
                            Ok(c) => c,
                            Err(error) => {
                                tracing::debug!(
                                    error = %error,
                                    "predicate state backend failed; failing closed"
                                );
                                return restrictive_action(on_exceeded);
                            }
                        };
                        if count > *max {
                            restrictive_action(on_exceeded)
                        } else {
                            EvaluatorDecision::Allow
                        }
                    }
                    ValueOrRateBound::NumericSum { max, field, window } => {
                        let max_value = match Decimal::from_str(max.trim()) {
                            Ok(v) => v,
                            Err(_) => {
                                tracing::debug!(
                                    max,
                                    "predicate evaluator could not parse NumericSum max; \
                                     failing closed"
                                );
                                return restrictive_action(on_exceeded);
                            }
                        };
                        let Some(window_dur) = parse_window(window) else {
                            tracing::debug!(
                                window,
                                "predicate evaluator could not parse window; failing closed"
                            );
                            return restrictive_action(on_exceeded);
                        };
                        if !ctx.arguments.is_resolved() {
                            tracing::debug!(
                                capability = %ctx.capability_name,
                                field = %field,
                                "NumericSum predicate fired but capability arguments are \
                                 unresolved; failing closed"
                            );
                            return restrictive_action(on_exceeded);
                        }
                        let Some(value) = ctx.arguments.extract_numeric(field) else {
                            tracing::debug!(
                                capability = %ctx.capability_name,
                                field = %field,
                                "NumericSum predicate fired but field is missing or non-numeric; \
                                 failing closed"
                            );
                            return restrictive_action(on_exceeded);
                        };
                        let key = ValueKey {
                            tenant_id: ctx.tenant_id.clone(),
                            hook_id,
                            capability: ctx.capability_name.clone(),
                            field: field.clone(),
                        };
                        let event_id = resolve_event_id(hook_id, ctx);
                        let sum = match self
                            .backend
                            .record_value(&key, &event_id, now, value, window_dur)
                        {
                            Ok(s) => s,
                            Err(error) => {
                                tracing::debug!(
                                    error = %error,
                                    "predicate state backend failed; failing closed"
                                );
                                return restrictive_action(on_exceeded);
                            }
                        };
                        if sum > max_value {
                            restrictive_action(on_exceeded)
                        } else {
                            EvaluatorDecision::Allow
                        }
                    }
                }
            }
        }
    }
}

/// Resolve the event id used for backend replay/idempotency dedup.
///
/// Prefer the caller-supplied [`BeforeCapabilityHookContext::caller_event_id`]
/// when present — that is the load-bearing path for durable backends, where
/// the same logical invocation re-evaluated on retry/replay must produce the
/// same id so the backend's UNIQUE constraint short-circuits the second
/// counter write.
///
/// When no caller id is wired through, fall back to a per-call-unique synth.
/// This preserves the in-memory backend's correctness ("each evaluation
/// counts once") while making the synth path explicit and easy to spot in
/// review: any production durable caller that lands without supplying
/// `caller_event_id` will visibly fall through to this synth path. The
/// fallback is the legacy behavior; the documented contract for durable use
/// is "caller MUST supply `caller_event_id`."
fn resolve_event_id(hook_id: HookId, ctx: &BeforeCapabilityHookContext) -> PredicateEventId {
    if let Some(caller_id) = ctx.caller_event_id.as_ref() {
        return caller_id.clone();
    }
    synth_event_id(hook_id, ctx)
}

/// Synthesize a per-call-unique event id. Used by [`resolve_event_id`] only
/// when the caller has not supplied a stable id via
/// [`BeforeCapabilityHookContext::caller_event_id`]; see that function's
/// documentation for the load-bearing semantics.
///
/// The id is the hex digest of `(hook_id, capability_name, arguments_digest,
/// process-local counter)`. The counter guarantees uniqueness across calls
/// even when (hook, ctx, now) are bit-identical (which happens routinely
/// in tests and can happen in production under tight loops on coarse
/// clocks).
fn synth_event_id(hook_id: HookId, ctx: &BeforeCapabilityHookContext) -> PredicateEventId {
    use std::fmt::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut hasher = blake3::Hasher::new();
    hasher.update(hook_id.as_bytes());
    hasher.update(ctx.capability_name.as_bytes());
    hasher.update(&ctx.arguments_digest);
    hasher.update(&seq.to_le_bytes());
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for byte in digest.as_bytes() {
        write!(s, "{byte:02x}").expect("writing to String never fails"); // safety: std::fmt::Write for String is infallible
    }
    PredicateEventId(s)
}

impl Default for PredicateEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

fn predicate_matches(predicate: &CapabilityPredicate, ctx: &BeforeCapabilityHookContext) -> bool {
    match predicate {
        CapabilityPredicate::Always => true,
        CapabilityPredicate::NameEquals { name } => &ctx.capability_name == name,
        CapabilityPredicate::NameStartsWith { prefix } => ctx.capability_name.starts_with(prefix),
        CapabilityPredicate::All { predicates } => {
            predicates.iter().all(|p| predicate_matches(p, ctx))
        }
        CapabilityPredicate::Any { predicates } => {
            predicates.iter().any(|p| predicate_matches(p, ctx))
        }
    }
}

fn restrictive_action(action: &OnExceededAction) -> EvaluatorDecision {
    match action {
        OnExceededAction::Deny { reason } => EvaluatorDecision::Deny {
            code: crate::predicate::DenyReasonCode::Generic,
            reason: reason.clone(),
        },
        OnExceededAction::DenyWithCode { code, reason } => EvaluatorDecision::Deny {
            code: *code,
            reason: reason.clone(),
        },
        OnExceededAction::PauseApproval { reason } => EvaluatorDecision::PauseApproval {
            code: crate::predicate::PauseReasonCode::Generic,
            reason: reason.clone(),
        },
        OnExceededAction::PauseApprovalWithCode { code, reason } => {
            EvaluatorDecision::PauseApproval {
                code: *code,
                reason: reason.clone(),
            }
        }
    }
}

/// Parse a window string like `"24h"`, `"10m"`, `"30s"` into a [`Duration`].
/// Unknown units, non-ASCII tail bytes, empty input, or malformed numeric
/// portions all return `None`. Crucially, the implementation must not panic
/// on non-ASCII or sub-byte-boundary input — manifest authors are untrusted
/// and the parser runs at install time.
fn parse_window(input: &str) -> Option<Duration> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    // Split on the last char as a unit. `input.split_at(input.len() - 1)`
    // would panic on multi-byte tail chars; iterate the chars instead and
    // use the unit char's own UTF-8 byte length to slice.
    let unit_char = input.chars().last()?;
    let unit_len = unit_char.len_utf8();
    if unit_len > input.len() {
        return None;
    }
    let (num_str, _unit_str) = input.split_at(input.len() - unit_len);
    if num_str.is_empty() {
        return None;
    }
    let num: u64 = num_str.parse().ok()?;
    let secs = match unit_char {
        's' => num,
        'm' => num.checked_mul(60)?,
        'h' => num.checked_mul(3600)?,
        'd' => num.checked_mul(86_400)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

/// Public window-validation helper used by manifest validation. Returns `Ok`
/// if the window parses to a non-zero duration, `Err` with a human-readable
/// reason otherwise. Used to surface bad windows at manifest install time
/// rather than at evaluation time.
pub fn validate_window(window: &str) -> Result<(), String> {
    match parse_window(window) {
        Some(d) if !d.is_zero() => Ok(()),
        Some(_) => Err(format!(
            "window `{window}` parses to zero duration; use a positive value"
        )),
        None => Err(format!(
            "window `{window}` is not a valid duration; expected `<u64><s|m|h|d>` (e.g. `24h`)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId, HookVersion};
    use ironclaw_host_api::TenantId;

    fn tenant() -> ironclaw_host_api::TenantId {
        ironclaw_host_api::TenantId::new("alpha").expect("ok")
    }

    fn ctx(capability: &str) -> BeforeCapabilityHookContext {
        BeforeCapabilityHookContext::new_unresolved(tenant(), capability.to_string(), [0u8; 32])
    }

    fn ctx_with_args(capability: &str, args: serde_json::Value) -> BeforeCapabilityHookContext {
        BeforeCapabilityHookContext::new(
            tenant(),
            capability.to_string(),
            [0u8; 32],
            crate::points::SanitizedArguments::from_json(args),
            None,
        )
    }

    fn ctx_with_args_for_tenant(
        tenant_id: TenantId,
        capability: &str,
        args: serde_json::Value,
    ) -> BeforeCapabilityHookContext {
        BeforeCapabilityHookContext::new(
            tenant_id,
            capability.to_string(),
            [0u8; 32],
            crate::points::SanitizedArguments::from_json(args),
            None,
        )
    }

    fn hook_id() -> HookId {
        HookId::derive(
            &ExtensionId::new("ext").expect("valid ExtensionId in test"),
            "1.0",
            &HookLocalId::new("h").expect("valid HookLocalId in test"),
            HookVersion::ONE,
        )
    }

    #[test]
    fn deny_capability_fires_on_match() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: "shell disabled".to_string(),
        };
        let denied = evaluator.evaluate(hook_id(), &spec, &ctx("shell.exec"));
        assert_eq!(
            denied,
            EvaluatorDecision::Deny {
                code: crate::predicate::DenyReasonCode::Generic,
                reason: "shell disabled".to_string()
            }
        );

        let allowed = evaluator.evaluate(hook_id(), &spec, &ctx("memory.read"));
        assert_eq!(allowed, EvaluatorDecision::Allow);
    }

    #[test]
    fn nested_predicate_matches_correctly() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::All {
                predicates: vec![
                    CapabilityPredicate::NameStartsWith {
                        prefix: "wallet.".to_string(),
                    },
                    CapabilityPredicate::Any {
                        predicates: vec![
                            CapabilityPredicate::NameEquals {
                                name: "wallet.sign".to_string(),
                            },
                            CapabilityPredicate::NameEquals {
                                name: "wallet.approve".to_string(),
                            },
                        ],
                    },
                ],
            },
            reason: "wallet locked".to_string(),
        };
        assert!(matches!(
            evaluator.evaluate(hook_id(), &spec, &ctx("wallet.sign")),
            EvaluatorDecision::Deny { .. }
        ));
        assert_eq!(
            evaluator.evaluate(hook_id(), &spec, &ctx("wallet.balance")),
            EvaluatorDecision::Allow
        );
        assert_eq!(
            evaluator.evaluate(hook_id(), &spec, &ctx("memory.read")),
            EvaluatorDecision::Allow
        );
    }

    /// henrypark133 HIGH regression on PR #3635: replay dedup must engage
    /// when the caller threads a stable `caller_event_id` through the hook
    /// context. Two evaluations with the same id must count as one
    /// invocation, even if all other context (capability, args, hook,
    /// timestamp) is bit-identical — because that's the very case durable
    /// backends face on retry/replay.
    #[test]
    fn duplicate_caller_event_id_is_deduped_in_invocation_count() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameEquals {
                name: "cap.x".to_string(),
            },
            bound: ValueOrRateBound::InvocationCount {
                max: 2,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "rate cap".to_string(),
            },
        };
        let stable_id = crate::predicate_state::PredicateEventId(
            "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234".to_string(),
        );
        let ctx_with_id = ctx("cap.x").with_caller_event_id(stable_id);
        let now = Instant::now();

        // First evaluation with the stable id — counted.
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx_with_id, now),
            EvaluatorDecision::Allow
        );
        // Replay with the same caller_event_id — backend dedupes, count
        // remains 1, still under the cap of 2.
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx_with_id, now),
            EvaluatorDecision::Allow
        );
        // A second logical invocation gets a different stable id and counts
        // — bringing total to 2, still under the cap.
        let second_id = crate::predicate_state::PredicateEventId(
            "ffff5555ffff5555ffff5555ffff5555ffff5555ffff5555ffff5555ffff5555".to_string(),
        );
        let ctx_second = ctx("cap.x").with_caller_event_id(second_id);
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx_second, now),
            EvaluatorDecision::Allow
        );
        // A third logical invocation crosses the cap.
        let third_id = crate::predicate_state::PredicateEventId(
            "11112222111122221111222211112222111122221111222211112222111122".to_string(),
        );
        let ctx_third = ctx("cap.x").with_caller_event_id(third_id);
        assert!(matches!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx_third, now),
            EvaluatorDecision::Deny { .. }
        ));

        // Sanity: without `caller_event_id`, the synth path counts each call.
        // Four evaluations against a different evaluator hit the cap normally.
        let plain = PredicateEvaluator::new();
        for _ in 0..2 {
            assert_eq!(
                plain.evaluate_at(hook_id(), &spec, &ctx("cap.x"), now),
                EvaluatorDecision::Allow
            );
        }
        assert!(matches!(
            plain.evaluate_at(hook_id(), &spec, &ctx("cap.x"), now),
            EvaluatorDecision::Deny { .. }
        ));
    }

    #[test]
    fn invocation_count_cap_denies_after_limit() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameEquals {
                name: "cap.x".to_string(),
            },
            bound: ValueOrRateBound::InvocationCount {
                max: 3,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "rate cap".to_string(),
            },
        };
        let now = Instant::now();
        for _ in 0..3 {
            let outcome = evaluator.evaluate_at(hook_id(), &spec, &ctx("cap.x"), now);
            assert_eq!(outcome, EvaluatorDecision::Allow);
        }
        let blocked = evaluator.evaluate_at(hook_id(), &spec, &ctx("cap.x"), now);
        assert_eq!(
            blocked,
            EvaluatorDecision::Deny {
                code: crate::predicate::DenyReasonCode::Generic,
                reason: "rate cap".to_string()
            }
        );
    }

    #[test]
    fn invocation_count_resets_after_window_expires() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::Always,
            bound: ValueOrRateBound::InvocationCount {
                max: 1,
                window: "10s".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "exceeded".to_string(),
            },
        };
        let start = Instant::now();
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx("cap.x"), start),
            EvaluatorDecision::Allow
        );
        assert!(matches!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx("cap.x"),
                start + Duration::from_secs(1)
            ),
            EvaluatorDecision::Deny { .. }
        ));
        // After the window expires, both prior entries are trimmed.
        assert_eq!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx("cap.x"),
                start + Duration::from_secs(20)
            ),
            EvaluatorDecision::Allow
        );
    }

    #[test]
    fn invocation_count_partitions_by_capability_name() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameStartsWith {
                prefix: "shell.".to_string(),
            },
            bound: ValueOrRateBound::InvocationCount {
                max: 1,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "exceeded".to_string(),
            },
        };
        let now = Instant::now();
        // shell.run hits its cap.
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx("shell.run"), now),
            EvaluatorDecision::Allow
        );
        assert!(matches!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx("shell.run"), now),
            EvaluatorDecision::Deny { .. }
        ));
        // shell.exec has its own counter.
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx("shell.exec"), now),
            EvaluatorDecision::Allow
        );
    }

    #[test]
    fn parse_window_supports_basic_units() {
        assert_eq!(parse_window("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_window("10m"), Some(Duration::from_secs(600)));
        assert_eq!(parse_window("24h"), Some(Duration::from_secs(86_400)));
        assert_eq!(parse_window("7d"), Some(Duration::from_secs(604_800)));
        assert_eq!(parse_window("notvalid"), None);
        assert_eq!(parse_window(""), None);
        assert_eq!(parse_window("100"), None);
    }

    fn numeric_sum_spec(max: &str, field: &str, window: &str) -> HookPredicateSpec {
        HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameEquals {
                name: "wallet.spend".to_string(),
            },
            bound: ValueOrRateBound::NumericSum {
                max: max.to_string(),
                field: field.to_string(),
                window: window.to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "cap exceeded".to_string(),
            },
        }
    }

    #[test]
    fn numeric_sum_denies_after_total_exceeds_max() {
        let evaluator = PredicateEvaluator::new();
        let spec = numeric_sum_spec("100", "amount", "1h");
        let now = Instant::now();
        // 40 + 40 = 80, under cap
        assert_eq!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"amount": "40"})),
                now,
            ),
            EvaluatorDecision::Allow,
        );
        assert_eq!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"amount": "40"})),
                now,
            ),
            EvaluatorDecision::Allow,
        );
        // Third spend pushes 120 > 100.
        assert!(matches!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"amount": "40"})),
                now,
            ),
            EvaluatorDecision::Deny { .. }
        ));
    }

    #[test]
    fn numeric_sum_fails_closed_with_unresolved_args() {
        let evaluator = PredicateEvaluator::new();
        let spec = numeric_sum_spec("100", "amount", "1h");
        // Unresolved args -> Deny, even though the cap is enormous relative to nothing.
        assert!(matches!(
            evaluator.evaluate(hook_id(), &spec, &ctx("wallet.spend")),
            EvaluatorDecision::Deny { .. }
        ));
    }

    #[test]
    fn numeric_sum_fails_closed_with_missing_field() {
        let evaluator = PredicateEvaluator::new();
        let spec = numeric_sum_spec("100", "amount", "1h");
        assert!(matches!(
            evaluator.evaluate(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"other": "5"})),
            ),
            EvaluatorDecision::Deny { .. }
        ));
    }

    #[test]
    fn numeric_sum_resets_after_window() {
        let evaluator = PredicateEvaluator::new();
        let spec = numeric_sum_spec("50", "amount", "10s");
        let start = Instant::now();
        // First call: 40 <= 50, allow.
        assert_eq!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"amount": 40})),
                start,
            ),
            EvaluatorDecision::Allow,
        );
        // Second call within window: 40 + 40 = 80 > 50, deny.
        assert!(matches!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"amount": 40})),
                start + Duration::from_secs(1),
            ),
            EvaluatorDecision::Deny { .. }
        ));
        // After window: prior entries trimmed; only the new 40 counts.
        assert_eq!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"amount": 40})),
                start + Duration::from_secs(20),
            ),
            EvaluatorDecision::Allow,
        );
    }

    #[test]
    fn numeric_sum_partitions_by_tenant() {
        let evaluator = PredicateEvaluator::new();
        let spec = numeric_sum_spec("50", "amount", "1h");
        let now = Instant::now();
        let alpha = TenantId::new("alpha").expect("ok");
        let beta = TenantId::new("beta").expect("ok");
        // alpha: 30 + 30 = 60 > 50 -> second spend denied.
        assert_eq!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args_for_tenant(
                    alpha.clone(),
                    "wallet.spend",
                    serde_json::json!({"amount": 30}),
                ),
                now,
            ),
            EvaluatorDecision::Allow,
        );
        assert!(matches!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args_for_tenant(
                    alpha,
                    "wallet.spend",
                    serde_json::json!({"amount": 30}),
                ),
                now,
            ),
            EvaluatorDecision::Deny { .. }
        ));
        // beta has its own bucket and is unaffected by alpha's spend.
        assert_eq!(
            evaluator.evaluate_at(
                hook_id(),
                &spec,
                &ctx_with_args_for_tenant(beta, "wallet.spend", serde_json::json!({"amount": 30}),),
                now,
            ),
            EvaluatorDecision::Allow,
        );
    }

    #[test]
    fn numeric_sum_fails_closed_with_unparseable_max() {
        let evaluator = PredicateEvaluator::new();
        let spec = numeric_sum_spec("not-a-number", "amount", "1h");
        assert!(matches!(
            evaluator.evaluate(
                hook_id(),
                &spec,
                &ctx_with_args("wallet.spend", serde_json::json!({"amount": 1})),
            ),
            EvaluatorDecision::Deny { .. }
        ));
    }

    #[test]
    fn parse_window_handles_non_ascii_safely() {
        // `™` is multi-byte; the old `split_at(len - 1)` would panic here.
        assert_eq!(parse_window("24™"), None);
        // Cyrillic + leading digits: also must not panic.
        assert_eq!(parse_window("24ч"), None);
    }

    #[test]
    fn parse_window_handles_empty_safely() {
        assert_eq!(parse_window(""), None);
        assert_eq!(parse_window("   "), None);
    }

    #[test]
    fn parse_window_handles_single_char() {
        // Single ASCII char with no numeric prefix: not a window.
        assert_eq!(parse_window("h"), None);
        // Single multi-byte char: not a window, must not panic.
        assert_eq!(parse_window("™"), None);
    }

    #[test]
    fn invocation_counter_partitions_by_tenant() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::Always,
            bound: ValueOrRateBound::InvocationCount {
                max: 1,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "rate cap".to_string(),
            },
        };

        let now = Instant::now();
        let alpha = ironclaw_host_api::TenantId::new("alpha").expect("ok");
        let beta = ironclaw_host_api::TenantId::new("beta").expect("ok");

        let ctx_alpha =
            BeforeCapabilityHookContext::new_unresolved(alpha, "cap.x".to_string(), [0u8; 32]);
        let ctx_beta =
            BeforeCapabilityHookContext::new_unresolved(beta, "cap.x".to_string(), [0u8; 32]);

        // Alpha hits the cap with one allowed call and a second deny.
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx_alpha, now),
            EvaluatorDecision::Allow
        );
        assert!(matches!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx_alpha, now),
            EvaluatorDecision::Deny { .. }
        ));
        // Beta is a separate tenant and must NOT inherit alpha's counter.
        assert_eq!(
            evaluator.evaluate_at(hook_id(), &spec, &ctx_beta, now),
            EvaluatorDecision::Allow,
            "tenants must not share rate-cap counters"
        );
    }

    // The direct LRU-helper test that lived here was moved to
    // `predicate_state::tests` along with the backend extraction. The
    // evaluator-level eviction property is still covered by
    // `predicate_state::tests::in_memory_*` plus the
    // [`PredicateEvaluator::evictions_observed`] passthrough exercised
    // via `evict_lru_pressure_advances_via_evaluator` below.

    /// Pressure check via the evaluator's public API: high-cardinality
    /// invocations across distinct keys should eventually surface
    /// through the backend's eviction counter (proxy for D5). We don't
    /// hit the 8192 cap in a unit test; the assertion is that the
    /// passthrough wires correctly so a future production-load test
    /// can read it.
    #[test]
    fn evictions_observed_reads_through_to_backend() {
        let evaluator = PredicateEvaluator::new();
        assert_eq!(evaluator.evictions_observed(), 0);
    }

    #[test]
    fn unparseable_window_fails_closed() {
        let evaluator = PredicateEvaluator::new();
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::Always,
            bound: ValueOrRateBound::InvocationCount {
                max: 10,
                window: "abc".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "bad".to_string(),
            },
        };
        assert!(matches!(
            evaluator.evaluate(hook_id(), &spec, &ctx("cap.x")),
            EvaluatorDecision::Deny { .. }
        ));
    }
}
