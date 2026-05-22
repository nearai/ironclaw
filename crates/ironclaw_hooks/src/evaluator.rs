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

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// Maximum number of distinct keys retained in either sliding-window history
/// map (`invocation_history` or `value_history`). Bounds the evaluator's
/// memory footprint against threat-model finding **D5** (unbounded growth
/// across `tenant × capability × hook × field` permutations). The cap is
/// per-map; the two maps together can hold up to `2 × MAX_HISTORY_KEYS`
/// entries.
///
/// When the cap is reached and a new key arrives, the LRU entry (the key
/// whose oldest retained timestamp is earliest) is evicted and the
/// `evictions_observed` counter is incremented so operators can detect
/// pressure. The cap is intentionally generous — typical deployments
/// hold dozens of keys; reaching 8192 indicates either pathological hook
/// density or an active attack on counter state.
pub const MAX_HISTORY_KEYS: usize = 8_192;

/// Maximum number of samples retained per `(hook, capability, tenant[, field])`
/// key in either sliding-window history. Without this cap, an installed hook
/// could declare a very large window on a hot capability and force the
/// evaluator to retain every invocation in the window, exhausting memory.
/// Once this cap is reached for a key, oldest samples are dropped to make
/// room for the new one — the predicate continues to evaluate against the
/// most-recent `MAX_SAMPLES_PER_KEY` samples in the window, which is the
/// conservative bound for a rate/value cap.
pub const MAX_SAMPLES_PER_KEY: usize = 4_096;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;

use crate::identity::HookId;
use crate::points::BeforeCapabilityHookContext;
use crate::predicate::{
    CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound,
};

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
/// state is shared across all predicate-backed hooks the evaluator serves.
pub struct PredicateEvaluator {
    /// `(hook_id, capability_name)` → recent invocation timestamps.
    invocation_history: Mutex<HashMap<HistoryKey, VecDeque<Instant>>>,
    /// `(tenant_id, hook_id, capability_name, field_path)` → recent
    /// (timestamp, numeric value) entries for `NumericSum` accumulation.
    /// Tenant-keyed so that one tenant's spend cannot affect another's
    /// rolling cap.
    value_history: Mutex<HashMap<ValueHistoryKey, VecDeque<(Instant, Decimal)>>>,
    /// Count of LRU evictions observed across both history maps. Exposed
    /// via [`Self::evictions_observed`] for operators monitoring D5
    /// pressure.
    evictions: AtomicU64,
}

impl PredicateEvaluator {
    pub fn new() -> Self {
        Self {
            invocation_history: Mutex::new(HashMap::new()),
            value_history: Mutex::new(HashMap::new()),
            evictions: AtomicU64::new(0),
        }
    }

    /// Total LRU evictions observed since construction across both
    /// history maps. Operators should alert when this counter advances —
    /// it means the evaluator hit its cap (`MAX_HISTORY_KEYS`) and
    /// started dropping the oldest tracked window. Threat-model finding D5.
    pub fn evictions_observed(&self) -> u64 {
        self.evictions.load(AtomicOrdering::Relaxed)
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
                        let key = HistoryKey {
                            hook_id,
                            tenant_id: ctx.tenant_id.clone(),
                            capability: ctx.capability_name.clone(),
                        };
                        let mut history = self
                            .invocation_history
                            .lock()
                            .expect("predicate history mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
                        if !history.contains_key(&key) && history.len() >= MAX_HISTORY_KEYS {
                            evict_lru_invocation(&mut history, &self.evictions);
                        }
                        let entries = history.entry(key).or_default();
                        // Trim entries outside the window.
                        let cutoff = now.checked_sub(window_dur).unwrap_or(now);
                        while let Some(front) = entries.front() {
                            if *front < cutoff {
                                entries.pop_front();
                            } else {
                                break;
                            }
                        }
                        // Per-key sample cap: drop the oldest sample to make
                        // room. This bounds memory under attacker-triggered
                        // hot capabilities; the predicate still evaluates
                        // against the most recent `MAX_SAMPLES_PER_KEY`
                        // samples in the window.
                        while entries.len() >= MAX_SAMPLES_PER_KEY {
                            entries.pop_front();
                        }
                        entries.push_back(now);
                        let count = entries.len() as u32;
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
                        let key = ValueHistoryKey {
                            tenant_id: ctx.tenant_id.clone(),
                            hook_id,
                            capability: ctx.capability_name.clone(),
                            field: field.clone(),
                        };
                        let mut history = self
                            .value_history
                            .lock()
                            .expect("predicate value history mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
                        if !history.contains_key(&key) && history.len() >= MAX_HISTORY_KEYS {
                            evict_lru_value(&mut history, &self.evictions);
                        }
                        let entries = history.entry(key).or_default();
                        let cutoff = now.checked_sub(window_dur).unwrap_or(now);
                        while let Some((ts, _)) = entries.front() {
                            if *ts < cutoff {
                                entries.pop_front();
                            } else {
                                break;
                            }
                        }
                        while entries.len() >= MAX_SAMPLES_PER_KEY {
                            entries.pop_front();
                        }
                        entries.push_back((now, value));
                        let sum: Decimal = entries.iter().map(|(_, v)| *v).sum();
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

impl Default for PredicateEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HistoryKey {
    hook_id: HookId,
    tenant_id: ironclaw_host_api::TenantId,
    capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ValueHistoryKey {
    tenant_id: TenantId,
    hook_id: HookId,
    capability: String,
    field: String,
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

/// Evict the entry with the earliest "front" timestamp — that is, the key
/// whose oldest retained sample is older than any other key's oldest sample.
/// This is a conservative LRU approximation: it preferentially drops keys
/// that have been idle the longest. The full O(N) scan is acceptable here
/// because this path runs only at-cap and the cap is sized so reaching it
/// is rare.
fn evict_lru_invocation(
    history: &mut HashMap<HistoryKey, VecDeque<Instant>>,
    evictions: &AtomicU64,
) {
    let victim = history
        .iter()
        .filter_map(|(k, v)| v.front().map(|ts| (k.clone(), *ts)))
        .min_by_key(|(_, ts)| *ts)
        .map(|(k, _)| k);
    if let Some(k) = victim {
        history.remove(&k);
        evictions.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

fn evict_lru_value(
    history: &mut HashMap<ValueHistoryKey, VecDeque<(Instant, Decimal)>>,
    evictions: &AtomicU64,
) {
    let victim = history
        .iter()
        .filter_map(|(k, v)| v.front().map(|(ts, _)| (k.clone(), *ts)))
        .min_by_key(|(_, ts)| *ts)
        .map(|(k, _)| k);
    if let Some(k) = victim {
        history.remove(&k);
        evictions.fetch_add(1, AtomicOrdering::Relaxed);
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
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
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

    /// Threat-model finding D5: when the invocation_history map hits its
    /// cap, the oldest tracked key must be evicted and the eviction
    /// counter must advance. Synthesized by injecting a private
    /// constant-shrunk test (we can't actually fill a map with
    /// MAX_HISTORY_KEYS=8192 distinct hooks in a unit test cheaply, so
    /// we exercise the path with a smaller cap analog by triggering the
    /// same LRU helper directly).
    #[test]
    fn lru_eviction_increments_counter_and_drops_oldest_key() {
        // Build an evaluator and call the LRU helper directly with a
        // crafted map. This bypasses the threshold check (we'd need an
        // 8192-key map otherwise) but exercises the exact helper used
        // when the threshold fires.
        let evaluator = PredicateEvaluator::new();
        assert_eq!(evaluator.evictions_observed(), 0);

        let mut map: HashMap<HistoryKey, VecDeque<Instant>> = HashMap::new();
        let now = Instant::now();
        let oldest_key = HistoryKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.oldest".to_string(),
        };
        let newer_key = HistoryKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.newer".to_string(),
        };
        let mut oldest_entries = VecDeque::new();
        oldest_entries.push_back(now.checked_sub(Duration::from_secs(60)).unwrap_or(now));
        let mut newer_entries = VecDeque::new();
        newer_entries.push_back(now);
        map.insert(oldest_key.clone(), oldest_entries);
        map.insert(newer_key.clone(), newer_entries);

        evict_lru_invocation(&mut map, &evaluator.evictions);

        assert_eq!(evaluator.evictions_observed(), 1);
        assert!(
            !map.contains_key(&oldest_key),
            "LRU key should have been evicted"
        );
        assert!(map.contains_key(&newer_key), "newer key should be retained");
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
