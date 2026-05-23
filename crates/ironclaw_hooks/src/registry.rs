//! Hook registry — the per-run table of active hook bindings.
//!
//! Sourced from the active `RunProfile` (not from a global table) so that
//! hook composition is deterministic per run and replay refuses on version
//! drift. The skeleton in this PR exposes the binding shape and a simple
//! resolver; the actual `RunProfile.hooks` field and the manifest→binding
//! installer pipeline land in follow-up slices that touch
//! `ironclaw_turns::run_profile` and the extension installer.

use std::collections::HashMap;

use ironclaw_host_api::ExtensionId;
use serde::{Deserialize, Serialize};

use crate::error::HookError;
use crate::identity::{HookId, HookVersion};
use crate::ordering::{HookPhase, HookPriority};
use crate::trust::HookTrustClass;

/// A single hook registration for an active run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookBinding {
    pub hook_id: HookId,
    pub hook_version: HookVersion,
    pub trust_class: HookTrustClass,
    pub phase: HookPhase,
    /// Author-chosen ordering tiebreaker within a phase. The dispatcher
    /// composes `(phase, priority, hook_id)` into [`HookOrderKey`] so two
    /// hooks at the same phase order deterministically by priority then
    /// content-addressed id. Defaults to [`HookPriority::DEFAULT`] for
    /// checkpoint payloads serialized before this field existed.
    #[serde(default = "default_priority")]
    pub priority: HookPriority,
    /// Coarse description of where the hook fires. The actual hook
    /// implementation (the trait object) is stored separately so this type
    /// remains serializable for checkpoint payloads.
    pub point: HookPointSpec,
    /// Extension that authored this hook. `None` for `Builtin` and `Trusted`
    /// hooks (which observe globally). `Some` for `Installed` hooks; the
    /// dispatcher consults this in combination with [`Self::scope`] to decide
    /// whether the hook fires against a given capability invocation.
    #[serde(default)]
    pub owning_extension: Option<ExtensionId>,
    /// Scope of capability invocations this hook fires against. Combined with
    /// [`Self::owning_extension`] to enforce manifest-declared scope at
    /// dispatch time. Defaults to [`HookBindingScope::Global`] so existing
    /// checkpoint payloads (pre-C3) deserialize to "always fire" behavior,
    /// which is the conservative interpretation for Builtin/Trusted bindings.
    #[serde(default)]
    pub scope: HookBindingScope,
    /// `true` if the dispatcher poisoned this slot during the current run.
    /// Persisted so resume cannot re-enable a hook that already crashed.
    pub poisoned: bool,
}

/// Runtime scope of a hook binding. Distinct from
/// [`crate::manifest::HookManifestScope`]: the manifest scope is what the
/// extension *declared*; this is what the dispatcher *enforces*. The two are
/// related but not identical because `Builtin` and `Trusted` hooks have no
/// manifest and are intrinsically `Global`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookBindingScope {
    /// Hook fires against every capability invocation regardless of provider.
    /// Used by `Builtin` and `Trusted` hooks, and by `Installed` hooks
    /// granted host-wide observation.
    #[default]
    Global,
    /// Hook fires only when `ctx.provider == binding.owning_extension`. When
    /// the provider cannot be resolved (capability has no known provider, or
    /// the middleware has no resolver wired in), the hook does NOT fire — the
    /// conservative default.
    OwnCapabilities,
    /// Hook fires regardless of capability provider, but still scoped to the
    /// current tenant. Today the dispatcher is per-tenant already, so this
    /// variant behaves like `Global` in terms of capability filtering. It is
    /// preserved as a distinct variant so audit / replay can tell the two
    /// authorities apart.
    SameTenant,
}

impl HookBindingScope {
    /// Returns `true` if a hook with this scope should fire against an
    /// invocation whose resolved provider is `invocation_provider`.
    ///
    /// `owning_extension` is the binding's declared author. For `Global` and
    /// `SameTenant` this is ignored and the hook always fires. For
    /// `OwnCapabilities` the hook fires only when both the binding's owning
    /// extension and the invocation's provider are `Some` and equal.
    ///
    /// The `OwnCapabilities` case is intentionally conservative: when the
    /// invocation provider is `None` (capability without a known provider,
    /// e.g., no resolver wired in), the hook does not fire. This is the
    /// documented behavior — see this crate's `CLAUDE.md` and audit finding
    /// C3.
    pub fn permits(
        &self,
        owning_extension: Option<&ExtensionId>,
        invocation_provider: Option<&ExtensionId>,
    ) -> bool {
        match self {
            HookBindingScope::Global | HookBindingScope::SameTenant => true,
            HookBindingScope::OwnCapabilities => {
                match (owning_extension, invocation_provider) {
                    (Some(owner), Some(provider)) => owner == provider,
                    // Conservative default: refuse to fire when either side
                    // is unknown. An attacker cannot bypass scope by stripping
                    // provider info from the descriptor.
                    _ => false,
                }
            }
        }
    }
}

fn default_priority() -> HookPriority {
    HookPriority::DEFAULT
}

/// Returns `true` if dispatches at `point` carry a resolved capability
/// `provider` in their context. Only those points can meaningfully enforce
/// `HookBindingScope::OwnCapabilities`. See finding #2.
fn point_has_capability_context(point: HookPointSpec) -> bool {
    match point {
        HookPointSpec::BeforeCapability | HookPointSpec::AfterCapability => true,
        HookPointSpec::BeforePrompt
        | HookPointSpec::AfterModel
        | HookPointSpec::AfterCheckpoint => false,
    }
}

/// Identifies which dispatcher point a binding registers against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPointSpec {
    BeforeCapability,
    BeforePrompt,
    AfterModel,
    AfterCapability,
    AfterCheckpoint,
}

/// Bindings grouped by dispatcher point for cheap lookup during a tick.
#[derive(Debug, Default)]
pub struct HookRegistry {
    by_point: HashMap<HookPointSpec, Vec<HookBinding>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from an iterator of bindings. Returns
    /// [`HookError::RegistryConstruction`] if any binding fails the
    /// phase-vs-trust gate (e.g., an Installed hook attempts to register at
    /// `Validation`).
    pub fn from_bindings<I>(bindings: I) -> Result<Self, HookError>
    where
        I: IntoIterator<Item = HookBinding>,
    {
        let mut registry = Self::new();
        for binding in bindings {
            registry.insert(binding)?;
        }
        Ok(registry)
    }

    pub fn insert(&mut self, binding: HookBinding) -> Result<(), HookError> {
        if !binding.phase.permits_trust(binding.trust_class) {
            return Err(HookError::RegistryConstruction(format!(
                "{:?}-tier hook cannot register at phase {:?}",
                binding.trust_class, binding.phase
            )));
        }
        // Scope-vs-point compatibility (serrrfirat finding #2). Some hook
        // points have no per-capability invocation context (e.g.
        // `BeforePrompt` fires once per prompt assembly, not per capability
        // invocation). For those points the `OwnCapabilities` scope is
        // meaningless — there is no `provider` to compare against, so the
        // dispatcher cannot enforce manifest-declared capability scope.
        // Rather than silently fire-against-everything (the pre-fix behavior)
        // or silently never-fire (which hides misconfiguration), reject the
        // binding at install time so the operator sees the mistake.
        if matches!(binding.scope, HookBindingScope::OwnCapabilities)
            && !point_has_capability_context(binding.point)
        {
            return Err(HookError::RegistryConstruction(format!(
                "scope `OwnCapabilities` not supported at point {:?}: \
                 this point has no per-capability invocation context. \
                 Use `Global` or `SameTenant` instead.",
                binding.point
            )));
        }
        // Hook IDs must be globally unique across the registry. A duplicate
        // ID at the same point would allow the same physical hook to appear
        // twice in a single dispatch snapshot; a duplicate at a different
        // point would let an attacker side-load a second binding for the
        // same hook id and observe its slot from outside the original point.
        // Either case violates the "one hook id, one slot" property the
        // poison machinery relies on.
        let duplicate = self
            .by_point
            .values()
            .flat_map(|bindings| bindings.iter())
            .any(|existing| existing.hook_id == binding.hook_id);
        if duplicate {
            return Err(HookError::RegistryConstruction(format!(
                "duplicate hook id `{}` rejected: each hook id may register \
                 against the registry at most once",
                binding.hook_id
            )));
        }
        self.by_point
            .entry(binding.point)
            .or_default()
            .push(binding);
        Ok(())
    }

    /// Apply a manifest-declared priority to an already-inserted binding.
    /// No-op if the hook id is not registered. Used by the registrar
    /// post-install (the tier-specific installers default priority to
    /// `HookPriority::DEFAULT` and the registrar then sets the
    /// manifest value).
    pub fn set_priority(&mut self, hook_id: HookId, priority: HookPriority) {
        for bindings in self.by_point.values_mut() {
            for binding in bindings.iter_mut() {
                if binding.hook_id == hook_id {
                    binding.priority = priority;
                    return;
                }
            }
        }
    }

    /// Active (non-poisoned) bindings at a point.
    pub fn active_at(&self, point: HookPointSpec) -> impl Iterator<Item = &HookBinding> {
        self.by_point
            .get(&point)
            .into_iter()
            .flat_map(|v| v.iter())
            .filter(|b| !b.poisoned)
    }

    /// Count of bindings whose `owning_extension` matches `extension`,
    /// summed across every attach point. Used by the registrar to enforce
    /// the per-extension cap cumulatively across multiple `install()`
    /// calls — threat-model finding D3 / hook registration flood.
    pub fn count_for_extension(&self, extension: &ExtensionId) -> usize {
        self.by_point
            .values()
            .flat_map(|bindings| bindings.iter())
            .filter(|b| b.owning_extension.as_ref() == Some(extension))
            .count()
    }

    /// Like [`Self::count_for_extension`] but restricted to one attach
    /// point. Used by D4 (per-extension-per-kind cap).
    pub fn count_for_extension_at(&self, extension: &ExtensionId, point: HookPointSpec) -> usize {
        self.by_point
            .get(&point)
            .map(|bindings| {
                bindings
                    .iter()
                    .filter(|b| b.owning_extension.as_ref() == Some(extension))
                    .count()
            })
            .unwrap_or(0)
    }

    /// Mark a hook's slot poisoned for the rest of the run.
    pub fn poison(&mut self, hook_id: HookId) {
        for bindings in self.by_point.values_mut() {
            for binding in bindings.iter_mut() {
                if binding.hook_id == hook_id {
                    binding.poisoned = true;
                }
            }
        }
    }

    pub fn is_poisoned(&self, hook_id: HookId) -> bool {
        self.by_point
            .values()
            .flat_map(|bindings| bindings.iter())
            .any(|b| b.hook_id == hook_id && b.poisoned)
    }

    /// Iterator over all currently-poisoned hook ids. Used by the dispatcher
    /// to snapshot the poison set under a single lock acquisition (see
    /// `HookDispatcher::ordered_bindings_with_poison_snapshot`) so hot
    /// dispatch paths can perform O(1) `HashSet` membership checks instead
    /// of O(H) per-binding scans.
    pub fn poisoned_ids(&self) -> impl Iterator<Item = HookId> + '_ {
        let mut seen: std::collections::HashSet<HookId> = std::collections::HashSet::new();
        self.by_point
            .values()
            .flat_map(|bindings| bindings.iter())
            .filter(|b| b.poisoned)
            .filter_map(move |b| {
                if seen.insert(b.hook_id) {
                    Some(b.hook_id)
                } else {
                    None
                }
            })
    }

    /// Total number of bindings, poisoned or not.
    pub fn len(&self) -> usize {
        self.by_point.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_point.values().all(Vec::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId};

    fn installed_binding(local: &str, phase: HookPhase, point: HookPointSpec) -> HookBinding {
        let hook_id = HookId::derive(
            &ExtensionId::new("ext").expect("valid ExtensionId in test"),
            "1.0",
            &HookLocalId::new(local).expect("valid HookLocalId in test"),
            HookVersion::ONE,
        );
        HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            priority: HookPriority::DEFAULT,
            point,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        }
    }

    #[test]
    fn rejects_installed_at_validation_phase() {
        let mut registry = HookRegistry::new();
        let result = registry.insert(installed_binding(
            "alpha",
            HookPhase::Validation,
            HookPointSpec::BeforeCapability,
        ));
        match result {
            Err(HookError::RegistryConstruction(msg)) => {
                assert!(msg.contains("Validation"));
                assert!(msg.contains("Installed"));
            }
            other => panic!("expected registry construction error, got {other:?}"),
        }
    }

    #[test]
    fn accepts_installed_at_policy_phase() {
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                "alpha",
                HookPhase::Policy,
                HookPointSpec::BeforeCapability,
            ))
            .expect("policy phase is open to Installed");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn rejects_duplicate_hook_id_at_same_point() {
        let mut registry = HookRegistry::new();
        let first = installed_binding("alpha", HookPhase::Policy, HookPointSpec::BeforeCapability);
        let id = first.hook_id;
        registry.insert(first).expect("first insert ok");

        // Same id, same point. Must be rejected to keep "one hook id, one
        // slot" intact for the poison re-check in dispatch.
        let dup = HookBinding {
            hook_id: id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        match registry.insert(dup) {
            Err(HookError::RegistryConstruction(msg)) => {
                assert!(msg.contains("duplicate"), "unexpected msg: {msg}");
            }
            other => panic!("expected duplicate rejection, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_hook_id_at_different_point() {
        let mut registry = HookRegistry::new();
        let first = installed_binding("alpha", HookPhase::Policy, HookPointSpec::BeforeCapability);
        let id = first.hook_id;
        registry.insert(first).expect("first insert ok");

        let dup_at_other_point = HookBinding {
            hook_id: id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Telemetry,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::AfterCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        assert!(matches!(
            registry.insert(dup_at_other_point),
            Err(HookError::RegistryConstruction(_))
        ));
    }

    #[test]
    fn rejects_own_capabilities_at_before_prompt() {
        // serrrfirat finding #2 regression: `BeforePrompt` has no
        // per-capability provider in its context, so `OwnCapabilities`
        // cannot be enforced at dispatch. Reject at install time rather
        // than silently fire-against-everything or silently never-fire.
        let mut registry = HookRegistry::new();
        let mut binding =
            installed_binding("alpha", HookPhase::Policy, HookPointSpec::BeforePrompt);
        binding.scope = HookBindingScope::OwnCapabilities;
        binding.owning_extension = Some(ironclaw_host_api::ExtensionId::new("ext").expect("valid"));
        match registry.insert(binding) {
            Err(HookError::RegistryConstruction(msg)) => {
                assert!(msg.contains("OwnCapabilities"), "unexpected msg: {msg}");
                assert!(msg.contains("BeforePrompt"), "unexpected msg: {msg}");
            }
            other => panic!("expected registry construction error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_own_capabilities_at_after_model() {
        // Same as BeforePrompt: AfterModel observers have no
        // per-capability provider, so `OwnCapabilities` is meaningless.
        let mut registry = HookRegistry::new();
        let mut binding =
            installed_binding("alpha", HookPhase::Telemetry, HookPointSpec::AfterModel);
        binding.scope = HookBindingScope::OwnCapabilities;
        binding.owning_extension = Some(ironclaw_host_api::ExtensionId::new("ext").expect("valid"));
        assert!(matches!(
            registry.insert(binding),
            Err(HookError::RegistryConstruction(_))
        ));
    }

    #[test]
    fn accepts_own_capabilities_at_before_capability() {
        // Sanity: capability-bound points still accept OwnCapabilities.
        let mut registry = HookRegistry::new();
        let mut binding =
            installed_binding("alpha", HookPhase::Policy, HookPointSpec::BeforeCapability);
        binding.scope = HookBindingScope::OwnCapabilities;
        binding.owning_extension = Some(ironclaw_host_api::ExtensionId::new("ext").expect("valid"));
        registry.insert(binding).expect("ok at before_capability");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn poisoned_hooks_are_filtered_from_active() {
        let mut registry = HookRegistry::new();
        let binding =
            installed_binding("alpha", HookPhase::Policy, HookPointSpec::BeforeCapability);
        let id = binding.hook_id;
        registry.insert(binding).expect("ok");
        assert_eq!(
            registry.active_at(HookPointSpec::BeforeCapability).count(),
            1
        );

        registry.poison(id);
        assert_eq!(
            registry.active_at(HookPointSpec::BeforeCapability).count(),
            0
        );
        assert!(registry.is_poisoned(id));
    }
}
