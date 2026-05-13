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

use crate::identity::HookLocalId;
use crate::ordering::{HookPhase, HookPriority};
use crate::predicate::HookPredicateSpec;

/// A single hook declaration in an extension manifest. Use [`Self::validate`]
/// at install time to surface format violations as structured errors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[serde(tag = "mode", rename_all = "snake_case")]
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
        if self.id.0.is_empty() {
            return Err(HookManifestValidationError("hook id is empty".to_string()));
        }
        // Phase × Trust: a manifest hook is always Installed, so it cannot
        // register at Validation or Authorization.
        if matches!(self.phase, HookPhase::Validation | HookPhase::Authorization) {
            return Err(HookManifestValidationError(format!(
                "hook `{}` cannot register at phase {:?}: that phase is reserved for builtin hooks",
                self.id.0, self.phase
            )));
        }
        // SameTenant scope requires an explicit grant identifier.
        if matches!(self.scope, HookManifestScope::SameTenant) && self.requires_grant.is_none() {
            return Err(HookManifestValidationError(format!(
                "hook `{}` scope = same_tenant requires `requires_grant` to be set",
                self.id.0
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
                self.id.0
            )));
        }
        Ok(())
    }
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
            id: HookLocalId("daily-cap".to_string()),
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
            id: HookLocalId("h".to_string()),
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
            id: HookLocalId("h".to_string()),
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
            id: HookLocalId("h".to_string()),
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
            id: HookLocalId("h".to_string()),
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
    fn full_entry_round_trips_through_toml() {
        let entry = HookManifestEntry {
            id: HookLocalId("daily-cap".to_string()),
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

    #[test]
    fn wasm_body_round_trips_with_defaults() {
        let entry = HookManifestEntry {
            id: HookLocalId("telemetry".to_string()),
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
