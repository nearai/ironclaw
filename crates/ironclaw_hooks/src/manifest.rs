//! Extension manifest `[[hooks]]` schema.
//!
//! Extensions declare hooks in their manifest alongside capabilities and
//! credentials. The registry installer reads `[[hooks]]` entries, validates
//! them (well-formedness + scope-vs-grant), pins each to a content-addressed
//! [`HookId`], and produces [`crate::registry::HookBinding`] entries. The
//! manifest schema itself stays in this crate so the validation contract is
//! reusable across whatever physical format the registry ships
//! (TOML, JSON, future).
//!
//! What the manifest cannot do:
//!
//! - Claim a trust class. Trust is determined by *where the hook came from*
//!   (registry-sourced ⇒ Installed). The manifest carries no `trust_class`
//!   field.
//! - Mint `Allow`-style decisions. Predicates emit `deny`, `pause_approval`,
//!   or value-cap actions; the predicate AST has no `Allow` variant.
//! - Register at `Validation` or `Authorization` phases. Those are
//!   Builtin-only and the registry installer rejects manifest hooks that
//!   request them.

use serde::{Deserialize, Serialize};

use crate::evaluator::validate_window;
use crate::identity::HookLocalId;
use crate::ordering::{HookPhase, HookPriority};
use crate::predicate::{CapabilityPredicate, HookPredicateSpec, ValueOrRateBound};

/// Maximum nesting depth of a `CapabilityPredicate::All`/`Any` tree. Bounds
/// stack/CPU exposure when a registry-supplied manifest is walked at hook
/// evaluation time.
pub const MAX_PREDICATE_DEPTH: usize = 8;
/// Maximum total node count in a `CapabilityPredicate` tree.
pub const MAX_PREDICATE_NODES: usize = 64;
/// Maximum length, in bytes, of an individual string field
/// (`NameEquals.name`, `NameStartsWith.prefix`) inside a predicate.
pub const MAX_PREDICATE_STRING_BYTES: usize = 256;
/// Maximum length, in bytes, of a manifest-supplied audit `reason` string.
/// Enforced at install time so a hostile manifest cannot smuggle large
/// payloads through the audit-reason channel even before runtime
/// truncation in [`crate::telemetry::sanitize_audit_reason`].
pub const MAX_MANIFEST_REASON_BYTES: usize = 512;

/// A single hook declaration in an extension manifest. Use [`Self::validate`]
/// at install time to surface format violations as structured errors.
///
/// Marked `#[non_exhaustive]` so future optional fields (versioning,
/// attribution, additional scopes) can be added without breaking
/// downstream construction sites. External callers must use the
/// [`Self::new`] constructor + the `with_*` builder methods; struct
/// literals from outside the crate will not compile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HookManifestEntry {
    pub id: HookLocalId,
    pub kind: HookManifestKind,
    #[serde(default)]
    pub scope: HookManifestScope,
    #[serde(default = "default_phase")]
    pub phase: HookPhase,
    #[serde(default = "default_priority")]
    pub priority: HookPriority,
    #[serde(default)]
    pub description: Option<String>,
    /// Cross-extension or wider scope requires explicit grant identifier; the
    /// registry installer compares this against the user's granted scope at
    /// install time.
    #[serde(default)]
    pub requires_grant: Option<String>,
    /// Hook body — either declarative predicate or programmatic WASM.
    pub body: HookManifestBody,
}

impl HookManifestEntry {
    /// Construct an entry with the three required fields; everything else
    /// uses the schema defaults. Chain `with_*` builder methods to set
    /// optional fields.
    ///
    /// ```ignore
    /// HookManifestEntry::new(local_id, HookManifestKind::BeforeCapability, body)
    ///     .with_scope(HookManifestScope::OwnCapabilities)
    ///     .with_description("Cap polymarket orders at 10/day")
    /// ```
    pub fn new(id: HookLocalId, kind: HookManifestKind, body: HookManifestBody) -> Self {
        Self {
            id,
            kind,
            scope: HookManifestScope::default(),
            phase: default_phase(),
            priority: default_priority(),
            description: None,
            requires_grant: None,
            body,
        }
    }

    pub fn with_scope(mut self, scope: HookManifestScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_phase(mut self, phase: HookPhase) -> Self {
        self.phase = phase;
        self
    }

    pub fn with_priority(mut self, priority: HookPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_requires_grant(mut self, grant: impl Into<String>) -> Self {
        self.requires_grant = Some(grant.into());
        self
    }
}

/// What kind of hook this is (which point it registers at).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookManifestKind {
    BeforeCapability,
    BeforePrompt,
    AfterModel,
    AfterCapability,
    AfterCheckpoint,
}

/// Hook scope. Determines whether the hook can observe / restrict only its
/// own extension's capability calls or also those of other extensions in the
/// same tenant.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookManifestScope {
    /// Hook fires only on capabilities owned by the declaring extension.
    /// Safe default; no user grant required.
    #[default]
    OwnCapabilities,
    /// Hook fires on capabilities owned by other extensions in the same
    /// tenant. Requires explicit user grant.
    SameTenant,
}

/// Hook body — either declarative predicate or programmatic WASM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookManifestBody {
    /// Declarative predicate evaluated by the host. No WASM invoked at hook
    /// time.
    Predicate { spec: HookPredicateSpec },
    /// Programmatic hook — a WASM function exported by the extension. The
    /// dispatcher runs it inside the extension's WASM sandbox with a typed
    /// `HookSink` host import.
    Wasm {
        export: String,
        #[serde(default)]
        budget: WasmBudget,
    },
}

/// Per-hook execution budget for WASM hooks. Defaults match the dispatcher's
/// per-hook timeout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmBudget {
    #[serde(default = "default_fuel")]
    pub fuel: u64,
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
    #[serde(default = "default_wall_ms")]
    pub wall_ms: u32,
}

impl Default for WasmBudget {
    fn default() -> Self {
        Self {
            fuel: default_fuel(),
            memory_mb: default_memory_mb(),
            wall_ms: default_wall_ms(),
        }
    }
}

fn default_fuel() -> u64 {
    100_000
}
fn default_memory_mb() -> u32 {
    4
}
fn default_wall_ms() -> u32 {
    50
}
fn default_phase() -> HookPhase {
    HookPhase::Policy
}
fn default_priority() -> HookPriority {
    HookPriority::DEFAULT
}

/// Errors surfaced by [`HookManifestEntry::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookManifestValidationError(pub String);

impl std::fmt::Display for HookManifestValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HookManifestValidationError {}

impl HookManifestEntry {
    /// Validate manifest-level invariants that don't require external context
    /// (trust class assignment, scope grant matching, hook-id pinning all
    /// happen later in the installer).
    pub fn validate(&self) -> Result<(), HookManifestValidationError> {
        // Note: the previous empty-id guard here is unreachable now that
        // `HookLocalId::new` rejects empty strings at construction time
        // (see `crates/ironclaw_hooks/src/identity.rs`), so manifest
        // deserialization fails before this method is ever called. Removed
        // per henrypark133 review of PR #3912 (finding L1).
        //
        // Phase × Trust: a manifest hook is always Installed, so it cannot
        // register at Validation or Authorization.
        if matches!(self.phase, HookPhase::Validation | HookPhase::Authorization) {
            return Err(HookManifestValidationError(format!(
                "hook `{}` cannot register at phase {:?}: that phase is reserved for builtin hooks",
                self.id.as_str(),
                self.phase
            )));
        }
        // SameTenant scope requires an explicit grant identifier.
        if matches!(self.scope, HookManifestScope::SameTenant) && self.requires_grant.is_none() {
            return Err(HookManifestValidationError(format!(
                "hook `{}` scope = same_tenant requires `requires_grant` to be set",
                self.id.as_str()
            )));
        }
        // Cross-extension scope cannot be combined with Mutator kinds without
        // additional review; reject for now and surface as a follow-up if a
        // legitimate use case emerges.
        if matches!(self.scope, HookManifestScope::SameTenant)
            && matches!(self.kind, HookManifestKind::BeforePrompt)
        {
            return Err(HookManifestValidationError(format!(
                "hook `{}` cannot combine scope = same_tenant with kind = before_prompt",
                self.id.as_str()
            )));
        }
        // Validate predicate bodies that carry a sliding-window string. We
        // surface unparseable windows at install time rather than letting
        // them fail closed at every evaluation.
        if let HookManifestBody::Predicate { spec } = &self.body {
            let (when, reason_strs, window) = match spec {
                HookPredicateSpec::DenyCapability { when, reason } => {
                    (Some(when), vec![reason.as_str()], None)
                }
                HookPredicateSpec::PauseApproval { when, reason } => {
                    (Some(when), vec![reason.as_str()], None)
                }
                HookPredicateSpec::RateOrValueCap {
                    when,
                    bound,
                    on_exceeded,
                } => {
                    let window = match bound {
                        ValueOrRateBound::InvocationCount { window, .. } => Some(window.as_str()),
                        ValueOrRateBound::NumericSum { window, .. } => Some(window.as_str()),
                    };
                    let reasons: Vec<&str> = match on_exceeded {
                        crate::predicate::OnExceededAction::Deny { reason }
                        | crate::predicate::OnExceededAction::DenyWithCode { reason, .. }
                        | crate::predicate::OnExceededAction::PauseApproval { reason }
                        | crate::predicate::OnExceededAction::PauseApprovalWithCode {
                            reason,
                            ..
                        } => vec![reason.as_str()],
                    };
                    (Some(when), reasons, window)
                }
            };
            if let Some(when) = when {
                validate_predicate_tree(self.id.as_str(), when)?;
            }
            for reason in reason_strs {
                if reason.len() > MAX_MANIFEST_REASON_BYTES {
                    return Err(HookManifestValidationError(format!(
                        "hook `{}` reason exceeds {} bytes (got {})",
                        self.id.as_str(),
                        MAX_MANIFEST_REASON_BYTES,
                        reason.len()
                    )));
                }
            }
            if let Some(window) = window {
                validate_window(window).map_err(|msg| {
                    HookManifestValidationError(format!(
                        "hook `{}` has invalid window: {}",
                        self.id.as_str(),
                        msg
                    ))
                })?;
            }
        }
        Ok(())
    }
}

/// Recursively validate a `CapabilityPredicate` tree against the manifest
/// safety bounds: maximum depth, total node count, and per-string byte
/// length. Bounds enforced here prevent a hostile registry-supplied manifest
/// from installing a predicate tree that recursively walks deeply or carries
/// multi-megabyte string fields at every match check.
fn validate_predicate_tree(
    hook_id: &str,
    predicate: &CapabilityPredicate,
) -> Result<(), HookManifestValidationError> {
    let mut node_count = 0usize;
    validate_predicate_inner(hook_id, predicate, 0, &mut node_count)
}

fn validate_predicate_inner(
    hook_id: &str,
    predicate: &CapabilityPredicate,
    depth: usize,
    node_count: &mut usize,
) -> Result<(), HookManifestValidationError> {
    if depth > MAX_PREDICATE_DEPTH {
        return Err(HookManifestValidationError(format!(
            "hook `{hook_id}` predicate tree exceeds max depth {MAX_PREDICATE_DEPTH}"
        )));
    }
    *node_count += 1;
    if *node_count > MAX_PREDICATE_NODES {
        return Err(HookManifestValidationError(format!(
            "hook `{hook_id}` predicate tree exceeds max node count {MAX_PREDICATE_NODES}"
        )));
    }
    match predicate {
        CapabilityPredicate::Always => Ok(()),
        CapabilityPredicate::NameEquals { name } => check_string(hook_id, "name", name),
        CapabilityPredicate::NameStartsWith { prefix } => check_string(hook_id, "prefix", prefix),
        CapabilityPredicate::All { predicates } | CapabilityPredicate::Any { predicates } => {
            if predicates.len() > MAX_PREDICATE_NODES {
                return Err(HookManifestValidationError(format!(
                    "hook `{hook_id}` predicate has fanout {} exceeding max {}",
                    predicates.len(),
                    MAX_PREDICATE_NODES
                )));
            }
            for child in predicates {
                validate_predicate_inner(hook_id, child, depth + 1, node_count)?;
            }
            Ok(())
        }
    }
}

fn check_string(hook_id: &str, field: &str, s: &str) -> Result<(), HookManifestValidationError> {
    if s.len() > MAX_PREDICATE_STRING_BYTES {
        return Err(HookManifestValidationError(format!(
            "hook `{hook_id}` predicate string `{field}` exceeds {MAX_PREDICATE_STRING_BYTES} bytes (got {})",
            s.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predicate::{CapabilityPredicate, OnExceededAction, ValueOrRateBound};

    fn predicate_body() -> HookManifestBody {
        HookManifestBody::Predicate {
            spec: HookPredicateSpec::RateOrValueCap {
                when: CapabilityPredicate::NameEquals {
                    name: "polymarket.place_order".to_string(),
                },
                bound: ValueOrRateBound::InvocationCount {
                    max: 10,
                    window: "24h".to_string(),
                },
                on_exceeded: OnExceededAction::Deny {
                    reason: "daily cap exceeded".to_string(),
                },
            },
        }
    }

    #[test]
    fn minimal_entry_validates() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("daily-cap").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: Some("Cap orders at 10/day".to_string()),
            requires_grant: None,
            body: predicate_body(),
        };
        entry.validate().expect("valid");
    }

    #[test]
    fn rejects_validation_phase_for_manifest_hooks() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("h").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Validation,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: predicate_body(),
        };
        assert!(entry.validate().is_err());
    }

    #[test]
    fn same_tenant_requires_grant() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("h").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::SameTenant,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: predicate_body(),
        };
        let err = entry.validate().unwrap_err();
        assert!(err.0.contains("requires_grant"));
    }

    #[test]
    fn same_tenant_with_grant_succeeds() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("h").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::SameTenant,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: Some("cross_extension_observation".to_string()),
            body: predicate_body(),
        };
        entry.validate().expect("valid with grant");
    }

    #[test]
    fn same_tenant_mutator_rejected() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("h").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforePrompt,
            scope: HookManifestScope::SameTenant,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: Some("g".to_string()),
            body: predicate_body(),
        };
        assert!(entry.validate().is_err());
    }

    #[test]
    fn validate_rejects_unparseable_window() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("bad-window").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Predicate {
                spec: HookPredicateSpec::RateOrValueCap {
                    when: CapabilityPredicate::Always,
                    bound: ValueOrRateBound::InvocationCount {
                        max: 1,
                        window: "24™".to_string(),
                    },
                    on_exceeded: OnExceededAction::Deny {
                        reason: "x".to_string(),
                    },
                },
            },
        };
        let err = entry.validate().expect_err("bad window must reject");
        assert!(err.0.contains("window"), "unexpected msg: {}", err.0);
    }

    #[test]
    fn validate_rejects_zero_duration_window() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("zero").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Predicate {
                spec: HookPredicateSpec::RateOrValueCap {
                    when: CapabilityPredicate::Always,
                    bound: ValueOrRateBound::InvocationCount {
                        max: 1,
                        window: "0s".to_string(),
                    },
                    on_exceeded: OnExceededAction::Deny {
                        reason: "x".to_string(),
                    },
                },
            },
        };
        assert!(entry.validate().is_err());
    }

    #[test]
    fn full_entry_round_trips_through_toml() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("daily-cap").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: Some("Cap orders at 10/day".to_string()),
            requires_grant: None,
            body: predicate_body(),
        };
        let toml_text = toml::to_string(&entry).expect("ser");
        let back: HookManifestEntry = toml::from_str(&toml_text).expect("de");
        assert_eq!(entry, back);
    }

    /// Unknown manifest fields must fail loud at parse time, not silently
    /// drop. A hostile or buggy extension that adds `trust_class = "trusted"`
    /// (a field that *does not exist* on the manifest — trust is determined
    /// by hook origin, never claimed) should be rejected by serde rather
    /// than silently accepted as an Installed hook.
    /// A predicate tree nested beyond `MAX_PREDICATE_DEPTH` must be rejected
    /// at install time — otherwise a hostile manifest could install a deep
    /// `All`/`Any` tree and force a recursive walk on every capability
    /// invocation.
    #[test]
    fn rejects_predicate_tree_exceeding_max_depth() {
        let mut node = CapabilityPredicate::Always;
        // Wrap deeply, well past MAX_PREDICATE_DEPTH (8).
        for _ in 0..(MAX_PREDICATE_DEPTH + 2) {
            node = CapabilityPredicate::All {
                predicates: vec![node],
            };
        }
        let entry = HookManifestEntry::new(
            HookLocalId::new("deep").expect("valid HookLocalId in test"),
            HookManifestKind::BeforeCapability,
            HookManifestBody::Predicate {
                spec: HookPredicateSpec::DenyCapability {
                    when: node,
                    reason: "x".to_string(),
                },
            },
        );
        let err = entry.validate().expect_err("must reject deep tree");
        assert!(
            err.0.contains("depth") || err.0.contains("nodes"),
            "unexpected msg: {}",
            err.0
        );
    }

    #[test]
    fn rejects_predicate_tree_exceeding_max_nodes() {
        let many: Vec<CapabilityPredicate> = (0..(MAX_PREDICATE_NODES + 8))
            .map(|i| CapabilityPredicate::NameEquals {
                name: format!("c{i}"),
            })
            .collect();
        let entry = HookManifestEntry::new(
            HookLocalId::new("fanout").expect("valid HookLocalId in test"),
            HookManifestKind::BeforeCapability,
            HookManifestBody::Predicate {
                spec: HookPredicateSpec::DenyCapability {
                    when: CapabilityPredicate::Any { predicates: many },
                    reason: "x".to_string(),
                },
            },
        );
        let err = entry.validate().expect_err("must reject huge fanout");
        assert!(
            err.0.contains("fanout") || err.0.contains("nodes"),
            "{}",
            err.0
        );
    }

    #[test]
    fn rejects_predicate_string_exceeding_max_bytes() {
        let entry = HookManifestEntry::new(
            HookLocalId::new("huge-name").expect("valid HookLocalId in test"),
            HookManifestKind::BeforeCapability,
            HookManifestBody::Predicate {
                spec: HookPredicateSpec::DenyCapability {
                    when: CapabilityPredicate::NameEquals {
                        name: "a".repeat(MAX_PREDICATE_STRING_BYTES + 1),
                    },
                    reason: "x".to_string(),
                },
            },
        );
        let err = entry.validate().expect_err("must reject huge string");
        assert!(err.0.contains("exceeds"), "{}", err.0);
    }

    #[test]
    fn rejects_manifest_reason_exceeding_max_bytes() {
        let entry = HookManifestEntry::new(
            HookLocalId::new("verbose").expect("valid HookLocalId in test"),
            HookManifestKind::BeforeCapability,
            HookManifestBody::Predicate {
                spec: HookPredicateSpec::DenyCapability {
                    when: CapabilityPredicate::Always,
                    reason: "x".repeat(MAX_MANIFEST_REASON_BYTES + 1),
                },
            },
        );
        let err = entry.validate().expect_err("must reject huge reason");
        assert!(err.0.contains("reason"), "{}", err.0);
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let toml_text = r#"
id = "h"
kind = "before_capability"
trust_class = "trusted"
[body]
mode = "predicate"
[body.spec]
type = "deny_capability"
reason = "no"
[body.spec.when]
type = "always"
"#;
        let err = toml::from_str::<HookManifestEntry>(toml_text)
            .expect_err("unknown field `trust_class` must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("trust_class") || msg.contains("unknown field"),
            "error message should mention the offending field: {msg}"
        );
    }

    /// Unknown nested body fields must also fail loud — the manifest's nested
    /// DTOs (`HookManifestBody`, `WasmBudget`) carry `deny_unknown_fields`
    /// so typos in WASM budget tuning don't get silently ignored.
    #[test]
    fn rejects_unknown_wasm_budget_field() {
        let toml_text = r#"
id = "h"
kind = "after_capability"
[body]
mode = "wasm"
export = "go"
[body.budget]
fuel = 1000
memory_mb = 1
wall_ms = 10
gas = 999
"#;
        let err = toml::from_str::<HookManifestEntry>(toml_text)
            .expect_err("unknown field `gas` must be rejected");
        assert!(
            err.to_string().contains("gas") || err.to_string().contains("unknown field"),
            "error: {err}"
        );
    }

    #[test]
    fn wasm_body_round_trips_with_defaults() {
        let entry = HookManifestEntry {
            id: HookLocalId::new("telemetry").expect("valid HookLocalId in test"),
            kind: HookManifestKind::AfterCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Telemetry,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Wasm {
                export: "order_telemetry".to_string(),
                budget: WasmBudget::default(),
            },
        };
        let toml_text = toml::to_string(&entry).expect("ser");
        let back: HookManifestEntry = toml::from_str(&toml_text).expect("de");
        assert_eq!(entry, back);
    }
}
