//! Bridge between extension-manifest `[[hooks]]` entries and a configured
//! [`HookDispatcher`].
//!
//! The [`HookRegistrar`] is the single seam that the registry installer (or
//! anything that ships an extension's hook block into a live dispatcher)
//! goes through. For each manifest entry it:
//!
//! 1. Validates the entry's well-formedness via
//!    [`crate::manifest::HookManifestEntry::validate`].
//! 2. Derives a content-addressed [`HookId`] from the extension identity +
//!    entry id + versions.
//! 3. Builds a [`HookBinding`] tagged `HookTrustClass::Installed` and
//!    inserts it into the registry (which re-checks phase × trust).
//! 4. Constructs the runtime impl from the manifest body and installs it
//!    against the same `HookId` in the dispatcher.
//!
//! Trust class is *not* settable here — registry-sourced hooks are always
//! `Installed`. Builtin and Trusted hooks bypass this path entirely.

use std::collections::HashMap;
use std::sync::Arc;

use crate::dispatch::HookDispatcherBuilder;
use crate::error::HookError;
use crate::evaluator::PredicateEvaluator;
use crate::identity::{ExtensionId, HookId, HookVersion};
use crate::installed_hook::PredicateBackedBeforeCapabilityHook;
use crate::manifest::{HookManifestBody, HookManifestEntry, HookManifestKind, HookManifestScope};
use crate::registry::{HookBindingScope, HookPointSpec};
use crate::wasm::{
    WasmBeforeCapabilityHook, WasmBeforePromptHook, WasmHookModuleRequest, WasmHookRuntime,
    WasmObserverHook,
};

/// Maximum number of hooks a single extension may register, summed across
/// every attach-point kind. Prevents a malicious or buggy extension from
/// flooding the dispatcher with bindings (threat-model finding D3). The
/// value is intentionally generous: typical extensions register 1–5 hooks,
/// and complex policy extensions reach 10–15. If an extension legitimately
/// needs more, the right move is a design review, not raising this cap.
pub const MAX_HOOKS_PER_EXTENSION: usize = 32;

/// Maximum number of hooks a single extension may register at one
/// attach-point kind (e.g., `BeforeCapability`). Prevents flooding *one*
/// dispatch point with bindings from a single extension even when the
/// per-extension total cap isn't yet reached (threat-model finding D4).
pub const MAX_HOOKS_PER_EXTENSION_PER_KIND: usize = 8;

/// Converts validated [`HookManifestEntry`] values into installed bindings +
/// dispatcher impls. One registrar per run; the shared
/// [`PredicateEvaluator`] threads sliding-window state across every
/// predicate-backed hook the registrar produces.
pub struct HookRegistrar {
    evaluator: Arc<PredicateEvaluator>,
    /// Grant tokens the host has verified for the current extension.
    /// `HookManifestEntry::requires_grant` is checked against this set
    /// at install time — a manifest claiming `requires_grant = "X"` is
    /// only accepted when `X` is in the verified set. Empty by default
    /// (default-deny): a manifest with any `requires_grant` is rejected
    /// unless callers explicitly thread the verified grants through
    /// [`Self::with_verified_grants`]. Closes the trust-boundary gap
    /// serrrfirat P1 #1 on PR #3573 flagged: previously the registrar
    /// validated only that the field was *set*, not that the host had
    /// actually issued the grant.
    verified_grants: std::collections::HashSet<String>,
    /// Optional WASM runtime used to instantiate Installed-tier WASM hooks
    /// declared by `HookManifestBody::Wasm`. When absent, WASM bodies are
    /// rejected at install time with `HookError::WasmRuntimeUnavailable`.
    wasm_runtime: Option<Arc<WasmHookRuntime>>,
}

impl HookRegistrar {
    pub fn new(evaluator: Arc<PredicateEvaluator>) -> Self {
        Self {
            evaluator,
            verified_grants: std::collections::HashSet::new(),
            wasm_runtime: None,
        }
    }

    /// Attach the set of grant tokens the host has verified for the
    /// extension being installed. Required for `same_tenant`-scoped
    /// hooks (and any other manifest entry declaring `requires_grant`)
    /// to install successfully. The host should populate this from its
    /// extension-grants store after authenticating the installing
    /// extension's grants.
    #[must_use]
    pub fn with_verified_grants(mut self, grants: impl IntoIterator<Item = String>) -> Self {
        self.verified_grants = grants.into_iter().collect();
        self
    }

    /// Attach a [`WasmHookRuntime`] so the registrar can instantiate
    /// Installed-tier WASM hook bodies. Without this, `HookManifestBody::Wasm`
    /// entries fail at install time.
    #[must_use]
    pub fn with_wasm_runtime(mut self, runtime: Arc<WasmHookRuntime>) -> Self {
        self.wasm_runtime = Some(runtime);
        self
    }

    /// Install all entries against `builder`, returning the updated
    /// builder along with the [`HookId`]s in the same order as `entries`.
    /// If any entry fails validation or impl construction, the registrar
    /// returns the error without rolling back earlier inserts — callers
    /// wanting all-or-nothing semantics should build into a scratch
    /// builder first.
    ///
    /// Threading the builder through by value keeps the dispatcher
    /// type-state intact: once the caller chains `.build_arc()` there is
    /// no further opportunity to mutate the dispatcher.
    pub fn install(
        &self,
        extension: ironclaw_host_api::ExtensionId,
        extension_version: String,
        entries: Vec<HookManifestEntry>,
        mut builder: HookDispatcherBuilder,
    ) -> Result<(HookDispatcherBuilder, Vec<HookId>), HookError> {
        // Mirror the host-validated `ExtensionId` into the content-addressed
        // identity wrapper used by `HookId::derive`. The two types coexist:
        // `ironclaw_host_api::ExtensionId` is the authority-bearing identifier
        // (validated, comparable across the host); `crate::identity::ExtensionId`
        // is a transparent string newtype the hash derivation consumes.
        let identity_extension: ExtensionId = (&extension).into();
        Self::enforce_registration_caps(&extension, &entries, builder.dispatcher_mut())?;
        let mut installed = Vec::with_capacity(entries.len());
        for entry in entries {
            let hook_id = self.install_one(
                &extension,
                &identity_extension,
                &extension_version,
                entry,
                &mut builder,
            )?;
            installed.push(hook_id);
        }
        Ok((builder, installed))
    }

    /// Reject the install batch wholesale before any binding is inserted
    /// if the extension is asking for more bindings than the caps allow.
    /// Pre-flight rejection is required so a partially-installed batch
    /// can't slip past the cap (the registrar otherwise inserts entries
    /// one at a time without rollback).
    ///
    /// **Cumulative across calls**: the caps apply to the total of
    /// already-installed bindings for `extension` plus the new batch. A
    /// repeated `install()` for the same extension (hot-reload, paginated
    /// manifest, multi-source loader) cannot bypass the cap by splitting
    /// the registrations into multiple batches.
    fn enforce_registration_caps(
        extension: &ironclaw_host_api::ExtensionId,
        entries: &[HookManifestEntry],
        dispatcher: &crate::dispatch::HookDispatcher,
    ) -> Result<(), HookError> {
        let already_total = dispatcher.count_bindings_for_extension(extension);
        if already_total + entries.len() > MAX_HOOKS_PER_EXTENSION {
            return Err(HookError::RegistryConstruction(format!(
                "extension `{}` would exceed the per-extension cap of {} hooks \
                 ({} already installed + {} in this batch); the cap is \
                 cumulative across `install()` calls (threat-model finding D3 \
                 / hook registration flood)",
                extension.as_str(),
                MAX_HOOKS_PER_EXTENSION,
                already_total,
                entries.len(),
            )));
        }
        let mut per_kind: HashMap<HookManifestKind, usize> = HashMap::new();
        for entry in entries {
            let count = per_kind.entry(entry.kind).or_insert(0);
            *count += 1;
            let already_at_kind = dispatcher
                .count_bindings_for_extension_at(extension, manifest_kind_to_point(entry.kind));
            if already_at_kind + *count > MAX_HOOKS_PER_EXTENSION_PER_KIND {
                return Err(HookError::RegistryConstruction(format!(
                    "extension `{}` would exceed the per-kind cap of {} hooks at \
                     attach point {:?} ({} already installed + {} in this batch); \
                     the cap is cumulative across `install()` calls \
                     (threat-model finding D4)",
                    extension.as_str(),
                    MAX_HOOKS_PER_EXTENSION_PER_KIND,
                    entry.kind,
                    already_at_kind,
                    *count,
                )));
            }
        }
        Ok(())
    }

    fn install_one(
        &self,
        owning_extension: &ironclaw_host_api::ExtensionId,
        identity_extension: &ExtensionId,
        extension_version: &str,
        entry: HookManifestEntry,
        builder: &mut HookDispatcherBuilder,
    ) -> Result<HookId, HookError> {
        entry.validate().map_err(|e| {
            HookError::RegistryConstruction(format!(
                "manifest entry `{}` failed validation: {}",
                entry.id, e
            ))
        })?;

        // serrrfirat P1 #1 on PR #3573: enforce verified-grant binding.
        // `validate()` only confirmed the field is present; the registrar
        // must now confirm the host has actually issued that grant.
        // Default-deny: an empty `verified_grants` set rejects any
        // manifest claiming `requires_grant`.
        if let Some(grant) = entry.requires_grant.as_ref()
            && !self.verified_grants.contains(grant)
        {
            return Err(HookError::RegistryConstruction(format!(
                "manifest entry `{}` declares `requires_grant = {:?}` but the \
                 host has not verified this grant for the installing extension; \
                 wire `HookRegistrar::with_verified_grants` from the extension \
                 grants store before installing",
                entry.id, grant
            )));
        }

        let hook_version = HookVersion::ONE;
        let binding_scope = manifest_scope_to_binding_scope(entry.scope);

        match entry.body {
            HookManifestBody::Predicate { spec } => match entry.kind {
                HookManifestKind::BeforeCapability => {
                    let hook_id = HookId::derive(
                        identity_extension,
                        extension_version,
                        &entry.id,
                        hook_version,
                    );
                    let hook = PredicateBackedBeforeCapabilityHook::new(
                        hook_id,
                        spec,
                        Arc::clone(&self.evaluator),
                    );
                    let dispatcher = builder.dispatcher_mut();
                    dispatcher.install_installed_before_capability(
                        hook_id,
                        entry.phase,
                        owning_extension.clone(),
                        binding_scope,
                        Box::new(hook),
                    )?;
                    // Apply the manifest-declared priority. The installer
                    // defaults to `HookPriority::DEFAULT`; the registrar is
                    // the only path that knows the manifest's `priority`
                    // field, so we set it post-insert.
                    dispatcher.set_binding_priority(hook_id, entry.priority);
                    Ok(hook_id)
                }
                other => Err(HookError::RegistryConstruction(format!(
                    "predicate body is only supported for `before_capability` hooks; \
                         entry `{}` declared kind {:?}",
                    entry.id, other
                ))),
            },
            HookManifestBody::Wasm { export, budget } => {
                let runtime = self.wasm_runtime.as_ref().ok_or_else(|| {
                    HookError::RegistryConstruction(format!(
                        "WASM hook runtime is not configured; entry `{}` cannot be installed",
                        entry.id
                    ))
                })?;
                let request = WasmHookModuleRequest {
                    extension_id: owning_extension,
                    extension_version,
                    hook_local_id: &entry.id,
                    kind: entry.kind,
                    export: &export,
                };
                let prepared = runtime.prepare(&request, budget).map_err(|error| {
                    HookError::RegistryConstruction(format!(
                        "WASM hook `{}` failed to prepare: {error}",
                        entry.id
                    ))
                })?;
                let wasm_identity_version =
                    WasmVersionMaterial::new(extension_version, &prepared.module_digest_hex())
                        .to_string();
                let hook_id = HookId::derive(
                    identity_extension,
                    &wasm_identity_version,
                    &entry.id,
                    hook_version,
                );
                let dispatcher = builder.dispatcher_mut();
                match entry.kind {
                    HookManifestKind::BeforeCapability => {
                        dispatcher.install_installed_wasm_before_capability(
                            hook_id,
                            entry.phase,
                            owning_extension.clone(),
                            binding_scope,
                            WasmBeforeCapabilityHook::new(Arc::clone(runtime), prepared),
                        )?;
                    }
                    HookManifestKind::BeforePrompt => {
                        dispatcher.install_installed_wasm_before_prompt(
                            hook_id,
                            entry.phase,
                            owning_extension.clone(),
                            binding_scope,
                            WasmBeforePromptHook::new(Arc::clone(runtime), prepared),
                        )?;
                    }
                    HookManifestKind::AfterModel
                    | HookManifestKind::AfterCapability
                    | HookManifestKind::AfterCheckpoint => {
                        let point = manifest_kind_to_point(entry.kind);
                        dispatcher.install_installed_wasm_observer(
                            hook_id,
                            entry.phase,
                            point,
                            owning_extension.clone(),
                            binding_scope,
                            WasmObserverHook::new(Arc::clone(runtime), prepared, point),
                        )?;
                    }
                }
                dispatcher.set_binding_priority(hook_id, entry.priority);
                Ok(hook_id)
            }
        }
    }
}

/// Identity material for a WASM-bodied hook. `HookId::derive` hashes a
/// `&str` for its version material, but the registrar always composes
/// the extension version with the compiled module digest in a fixed
/// shape — `"{extension_version}+wasm:{module_digest_hex}"` — so the
/// concatenated string never floats free as a stringly-typed argument
/// (henrypark133 LOW #20 on PR #3634).
#[derive(Debug, Clone)]
struct WasmVersionMaterial {
    extension_version: String,
    module_digest_hex: String,
}

impl WasmVersionMaterial {
    fn new(extension_version: &str, module_digest_hex: &str) -> Self {
        Self {
            extension_version: extension_version.to_string(),
            module_digest_hex: module_digest_hex.to_string(),
        }
    }
}

impl std::fmt::Display for WasmVersionMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}+wasm:{}",
            self.extension_version, self.module_digest_hex
        )
    }
}

/// Map the manifest-declared scope to the dispatcher's runtime scope. The
/// manifest enum is parsed at install time; the dispatcher consults the
/// runtime enum on every invocation, so we eagerly translate here.
fn manifest_kind_to_point(kind: HookManifestKind) -> HookPointSpec {
    match kind {
        HookManifestKind::BeforeCapability => HookPointSpec::BeforeCapability,
        HookManifestKind::BeforePrompt => HookPointSpec::BeforePrompt,
        HookManifestKind::AfterModel => HookPointSpec::AfterModel,
        HookManifestKind::AfterCapability => HookPointSpec::AfterCapability,
        HookManifestKind::AfterCheckpoint => HookPointSpec::AfterCheckpoint,
    }
}

fn manifest_scope_to_binding_scope(scope: HookManifestScope) -> HookBindingScope {
    match scope {
        HookManifestScope::OwnCapabilities => HookBindingScope::OwnCapabilities,
        HookManifestScope::SameTenant => HookBindingScope::SameTenant,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::HookLocalId;
    use crate::manifest::{HookManifestBody, HookManifestKind, HookManifestScope, WasmBudget};
    use crate::ordering::{HookPhase, HookPriority};
    use crate::points::BeforeCapabilityHookContext;
    use crate::predicate::{CapabilityPredicate, HookPredicateSpec};
    use crate::registry::HookRegistry;

    fn extension() -> ironclaw_host_api::ExtensionId {
        ironclaw_host_api::ExtensionId::new("polymarket-trader").expect("valid extension id")
    }

    fn identity_extension() -> ExtensionId {
        (&extension()).into()
    }

    fn predicate_entry(local: &str) -> HookManifestEntry {
        HookManifestEntry {
            id: HookLocalId::new(local).expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Predicate {
                spec: HookPredicateSpec::DenyCapability {
                    when: CapabilityPredicate::NameEquals {
                        name: "shell.exec".to_string(),
                    },
                    reason: "shell denied".to_string(),
                },
            },
        }
    }

    #[tokio::test]
    async fn install_predicate_entry_builds_binding_and_installs_hook() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let (builder, ids) = registrar
            .install(
                extension(),
                "0.4.2".to_string(),
                vec![predicate_entry("deny-shell")],
                builder,
            )
            .expect("install ok");
        assert_eq!(ids.len(), 1);
        let dispatcher = builder.build_arc();

        // Dispatch and confirm the registered predicate fires. The default
        // manifest scope is `OwnCapabilities`, so the dispatch ctx must
        // include a `provider` matching the registrar's extension or the hook
        // is filtered out as out-of-scope.
        let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
        let ctx = BeforeCapabilityHookContext::new(
            tenant,
            "shell.exec".to_string(),
            [0u8; 32],
            crate::points::SanitizedArguments::unresolved(),
            Some(extension()),
        );
        let outcome = dispatcher.dispatch_before_capability(&ctx).await;
        assert!(!outcome.decision.permits());
    }

    /// Registrar happy-path test for a WASM body: a valid module that
    /// satisfies the host-import surface installs, returns a hook id, and
    /// places exactly one active binding into the resulting registry.
    /// Companion to the existing `install_wasm_body_requires_runtime`
    /// negative case. Test #16 on PR #3634.
    #[test]
    fn install_wasm_body_with_runtime_succeeds_and_produces_binding() {
        use crate::wasm::{WasmHookModuleRequest, WasmHookModuleResolver, WasmHookRuntime};
        use std::sync::Mutex as StdMutex;

        const WASM_PASS: &str = r#"
(module
  (import "ic:hooks/before-capability@1" "pass" (func $pass (result i32)))
  (func (export "evaluate")
    call $pass
    drop)
)
"#;

        struct StaticResolver {
            bytes: StdMutex<Vec<u8>>,
        }
        impl WasmHookModuleResolver for StaticResolver {
            fn resolve_module(
                &self,
                _request: &WasmHookModuleRequest<'_>,
            ) -> Result<Vec<u8>, crate::wasm::WasmHookRuntimeError> {
                Ok(self.bytes.lock().expect("resolver lock").clone())
            }
        }

        let bytes = wat::parse_str(WASM_PASS).expect("wat parses");
        let resolver = Arc::new(StaticResolver {
            bytes: StdMutex::new(bytes),
        });
        let runtime = Arc::new(WasmHookRuntime::new(resolver).expect("runtime"));
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()))
            .with_wasm_runtime(runtime)
            .with_verified_grants(["installed-wasm-happy".to_string()]);
        let entry = HookManifestEntry {
            id: HookLocalId::new("wasm-happy").expect("valid local id"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::SameTenant,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: Some("installed-wasm-happy".to_string()),
            body: HookManifestBody::Wasm {
                export: "evaluate".to_string(),
                budget: WasmBudget::default(),
            },
        };
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let (builder, ids) = registrar
            .install(extension(), "0.1.0".to_string(), vec![entry], builder)
            .expect("wasm install ok");
        assert_eq!(ids.len(), 1, "exactly one binding produced");
        let dispatcher = builder.build_arc();
        // Round-trip the dispatcher: the installed hook is *findable* via
        // its hook id and the binding is not yet poisoned. This proves the
        // registrar didn't quietly drop the binding after install-time
        // validation.
        assert!(
            dispatcher
                .registry_for_test()
                .lock()
                .expect("registry lock")
                .contains_hook(ids[0]),
            "installed wasm binding must be visible in the registry"
        );
        assert!(
            !dispatcher
                .registry_for_test()
                .lock()
                .expect("registry lock")
                .is_poisoned(ids[0]),
            "fresh install must not be poisoned"
        );
    }

    #[test]
    fn install_wasm_body_requires_runtime() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let entry = HookManifestEntry {
            id: HookLocalId::new("wasm-hook").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Wasm {
                export: "evaluate".to_string(),
                budget: WasmBudget::default(),
            },
        };
        let err = registrar
            .install(extension(), "0.1.0".to_string(), vec![entry], builder)
            .expect_err("wasm body needs a configured runtime");
        match err {
            HookError::RegistryConstruction(msg) => {
                assert!(
                    msg.contains("WASM hook runtime is not configured"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected RegistryConstruction, got {other:?}"),
        }
    }

    #[test]
    fn install_rejects_invalid_phase_for_installed_tier() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let mut entry = predicate_entry("bad-phase");
        // Validation phase is Builtin-only — manifest validation rejects it
        // before the registry would.
        entry.phase = HookPhase::Validation;
        let err = registrar
            .install(extension(), "0.1.0".to_string(), vec![entry], builder)
            .expect_err("validation phase must be rejected");
        assert!(matches!(err, HookError::RegistryConstruction(_)));
    }

    /// End-to-end registrar test for `DenyWithCode`: a manifest carrying
    /// `OnExceededAction::DenyWithCode { code, reason }` installs cleanly
    /// AND dispatches with the code's static label as the model-visible
    /// reason. Bridges the gap codex flagged: prior tests covered the
    /// enum serde + direct hook evaluation; this one drives the full
    /// install-then-dispatch path the registrar exposes.
    #[tokio::test]
    async fn install_deny_with_code_manifest_surfaces_code_label_on_dispatch() {
        use crate::predicate::{DenyReasonCode, OnExceededAction, ValueOrRateBound};

        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let entry = HookManifestEntry {
            id: HookLocalId::new("rate-cap-with-code").expect("valid HookLocalId in test"),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Predicate {
                spec: HookPredicateSpec::RateOrValueCap {
                    when: CapabilityPredicate::NameEquals {
                        name: "polymarket.place_order".to_string(),
                    },
                    bound: ValueOrRateBound::InvocationCount {
                        max: 0,
                        window: "1h".to_string(),
                    },
                    on_exceeded: OnExceededAction::DenyWithCode {
                        code: DenyReasonCode::RateLimit,
                        reason: "audit-only".to_string(),
                    },
                },
            },
        };
        let (builder, ids) = registrar
            .install(extension(), "0.4.2".to_string(), vec![entry], builder)
            .expect("install ok");
        assert_eq!(ids.len(), 1);
        let dispatcher = builder.build_arc();

        let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
        let ctx = BeforeCapabilityHookContext::new(
            tenant,
            "polymarket.place_order".to_string(),
            [0u8; 32],
            crate::points::SanitizedArguments::unresolved(),
            Some(extension()),
        );
        let outcome = dispatcher.dispatch_before_capability(&ctx).await;
        match outcome.decision.view() {
            crate::kinds::gate::GateDecisionView::Deny { reason } => {
                assert_eq!(
                    reason.as_str(),
                    "hook_rate_limit",
                    "DenyWithCode manifest must surface the code's label \
                     (hook_rate_limit) through the registrar→dispatcher path"
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn install_returns_hook_ids_in_input_order() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let entries = vec![
            predicate_entry("first"),
            predicate_entry("second"),
            predicate_entry("third"),
        ];
        let expected: Vec<HookId> = entries
            .iter()
            .map(|e| HookId::derive(&identity_extension(), "0.4.2", &e.id, HookVersion::ONE))
            .collect();

        let (_builder, actual) = registrar
            .install(extension(), "0.4.2".to_string(), entries, builder)
            .expect("install ok");
        assert_eq!(actual, expected);
    }

    /// Threat-model finding D3 regression: an extension cannot register
    /// more than `MAX_HOOKS_PER_EXTENSION` hooks in a single install
    /// batch. The rejection is pre-flight (no partial install), and the
    /// error message carries enough context for an operator to diagnose
    /// why the install was rejected.
    #[test]
    fn install_rejects_when_total_exceeds_per_extension_cap() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let entries: Vec<HookManifestEntry> = (0..(MAX_HOOKS_PER_EXTENSION + 1))
            .map(|i| predicate_entry(&format!("h-{i}")))
            .collect();
        let err = registrar
            .install(extension(), "0.1.0".to_string(), entries, builder)
            .expect_err("over-cap install must be rejected");
        match err {
            HookError::RegistryConstruction(msg) => {
                assert!(msg.contains("per-extension cap"), "msg = {msg}");
                assert!(
                    msg.contains("D3"),
                    "msg must cite the threat-model finding: {msg}"
                );
            }
            other => panic!("expected RegistryConstruction, got {other:?}"),
        }
    }

    /// Threat-model finding D4 regression: an extension cannot stack more
    /// than `MAX_HOOKS_PER_EXTENSION_PER_KIND` hooks at one attach point
    /// even if the per-extension total is still under cap. This is the
    /// stronger of the two caps — a flood concentrated at one dispatch
    /// point is worse than the same flood spread out, because it widens
    /// the dispatch fan-out exactly where back-pressure shows up.
    #[test]
    fn install_rejects_when_per_kind_cap_exceeded() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        // Stay under the per-extension cap but exceed per-kind: all
        // entries default to `BeforeCapability`.
        let entries: Vec<HookManifestEntry> = (0..(MAX_HOOKS_PER_EXTENSION_PER_KIND + 1))
            .map(|i| predicate_entry(&format!("h-{i}")))
            .collect();
        assert!(entries.len() <= MAX_HOOKS_PER_EXTENSION);
        let err = registrar
            .install(extension(), "0.1.0".to_string(), entries, builder)
            .expect_err("over-kind install must be rejected");
        match err {
            HookError::RegistryConstruction(msg) => {
                assert!(msg.contains("per-kind cap"), "msg = {msg}");
                assert!(
                    msg.contains("D4"),
                    "msg must cite the threat-model finding: {msg}"
                );
            }
            other => panic!("expected RegistryConstruction, got {other:?}"),
        }
    }

    /// At-cap installs must succeed — the cap is a ceiling, not a strict
    /// inequality. Important so the test below documenting the cap value
    /// can't get out of sync with the enforcement.
    #[tokio::test]
    async fn install_accepts_at_per_extension_cap() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        // Build a batch exactly at the per-kind ceiling so neither cap
        // trips. (`per-kind` is the tighter constraint for the default
        // `BeforeCapability` kind every `predicate_entry` produces.)
        let entries: Vec<HookManifestEntry> = (0..MAX_HOOKS_PER_EXTENSION_PER_KIND)
            .map(|i| predicate_entry(&format!("h-{i}")))
            .collect();
        let (_builder, ids) = registrar
            .install(extension(), "0.1.0".to_string(), entries, builder)
            .expect("at-cap install must succeed");
        assert_eq!(ids.len(), MAX_HOOKS_PER_EXTENSION_PER_KIND);
    }

    /// Regression for Firat's D3-per-call finding: repeated `install()`
    /// calls for the same extension must not bypass the cap by splitting
    /// the registrations across batches. The second call sees the first
    /// call's bindings via dispatcher.count_bindings_for_extension and
    /// rejects when the cumulative total would exceed
    /// `MAX_HOOKS_PER_EXTENSION_PER_KIND`.
    #[tokio::test]
    async fn install_cumulative_cap_rejects_second_batch_over_per_kind() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let mut builder = HookDispatcherBuilder::new(HookRegistry::new());
        // First batch: half the per-kind cap. Should succeed.
        let first_batch: Vec<HookManifestEntry> = (0..MAX_HOOKS_PER_EXTENSION_PER_KIND)
            .map(|i| predicate_entry(&format!("first-{i}")))
            .collect();
        let (b, ids) = registrar
            .install(extension(), "0.1.0".to_string(), first_batch, builder)
            .expect("first at-cap install must succeed");
        assert_eq!(ids.len(), MAX_HOOKS_PER_EXTENSION_PER_KIND);
        builder = b;
        // Second batch: one more entry on top of an already-at-cap extension.
        // The cumulative check must reject this even though the second batch
        // by itself is small (just 1 entry).
        let second_batch = vec![predicate_entry("overflow")];
        let err = registrar
            .install(extension(), "0.1.0".to_string(), second_batch, builder)
            .expect_err("cumulative over-cap install must be rejected");
        match err {
            HookError::RegistryConstruction(msg) => {
                assert!(
                    msg.contains("per-kind cap") && msg.contains("D4"),
                    "expected D4 cap rejection, got: {msg}"
                );
                assert!(
                    msg.contains("already installed"),
                    "msg should cite cumulative state, got: {msg}"
                );
            }
            other => panic!("expected RegistryConstruction, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn installer_propagates_owning_extension_and_scope_from_manifest() {
        // Two entries, distinct manifest scopes; assert each is reflected in
        // the resulting `HookBinding`.
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()))
            .with_verified_grants(["cross_extension_observation".to_string()]);
        let builder = HookDispatcherBuilder::new(HookRegistry::new());

        let mut own = predicate_entry("own-scope");
        own.scope = HookManifestScope::OwnCapabilities;

        let mut tenant_scope = predicate_entry("tenant-scope");
        tenant_scope.scope = HookManifestScope::SameTenant;
        tenant_scope.requires_grant = Some("cross_extension_observation".to_string());

        let host_extension =
            ironclaw_host_api::ExtensionId::new("polymarket-trader").expect("valid ext id");
        let (builder, ids) = registrar
            .install(
                host_extension.clone(),
                "0.4.2".to_string(),
                vec![own.clone(), tenant_scope.clone()],
                builder,
            )
            .expect("install ok");
        assert_eq!(ids.len(), 2);

        let dispatcher = builder.build_arc();
        let registry = dispatcher
            .registry_for_test()
            .lock()
            .expect("registry mutex");
        let bindings: Vec<_> = registry
            .active_at(crate::registry::HookPointSpec::BeforeCapability)
            .cloned()
            .collect();
        assert_eq!(bindings.len(), 2);

        let own_binding = bindings
            .iter()
            .find(|b| b.hook_id == ids[0])
            .expect("own-scope binding present");
        assert_eq!(
            own_binding.scope,
            HookBindingScope::OwnCapabilities,
            "manifest OwnCapabilities must map to binding OwnCapabilities"
        );
        assert_eq!(
            own_binding.owning_extension.as_ref(),
            Some(&host_extension),
            "binding must carry the installer's extension id"
        );

        let tenant_binding = bindings
            .iter()
            .find(|b| b.hook_id == ids[1])
            .expect("tenant-scope binding present");
        assert_eq!(tenant_binding.scope, HookBindingScope::SameTenant);
        assert_eq!(
            tenant_binding.owning_extension.as_ref(),
            Some(&host_extension)
        );
    }

    /// serrrfirat P1 #1 regression on PR #3573: a manifest declaring
    /// `requires_grant` must NOT install when the host has not verified
    /// that grant for the extension. Previously the manifest's mere
    /// presence of the field was sufficient — anyone could write
    /// `requires_grant = "anything"` and get a cross-extension binding.
    #[tokio::test]
    async fn install_rejects_same_tenant_without_verified_grant() {
        // Registrar with NO verified grants — default-deny.
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());

        let mut tenant_scope = predicate_entry("tenant-scope-no-grant");
        tenant_scope.scope = HookManifestScope::SameTenant;
        tenant_scope.requires_grant = Some("cross_extension_observation".to_string());

        let err = registrar
            .install(
                extension(),
                "0.1.0".to_string(),
                vec![tenant_scope],
                builder,
            )
            .expect_err("install must reject when the host has not verified the requested grant");
        match err {
            HookError::RegistryConstruction(msg) => {
                assert!(
                    msg.contains("cross_extension_observation"),
                    "error must cite the missing grant; got: {msg}"
                );
                assert!(
                    msg.contains("requires_grant"),
                    "error must cite the manifest field; got: {msg}"
                );
            }
            other => panic!("expected RegistryConstruction, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn install_rejects_same_tenant_when_verified_grants_mismatch() {
        // Registrar with a verified grant, but the manifest asks for a
        // different one — still rejected.
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()))
            .with_verified_grants(["some_other_grant".to_string()]);
        let builder = HookDispatcherBuilder::new(HookRegistry::new());

        let mut tenant_scope = predicate_entry("tenant-scope-wrong-grant");
        tenant_scope.scope = HookManifestScope::SameTenant;
        tenant_scope.requires_grant = Some("cross_extension_observation".to_string());

        let err = registrar
            .install(
                extension(),
                "0.1.0".to_string(),
                vec![tenant_scope],
                builder,
            )
            .expect_err("install must reject when the requested grant is not in the verified set");
        assert!(matches!(err, HookError::RegistryConstruction(_)));
    }
}
