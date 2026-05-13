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

use std::sync::Arc;

use crate::dispatch::HookDispatcherBuilder;
use crate::error::HookError;
use crate::evaluator::PredicateEvaluator;
use crate::identity::{ExtensionId, HookId, HookVersion};
use crate::installed_hook::PredicateBackedBeforeCapabilityHook;
use crate::manifest::{HookManifestBody, HookManifestEntry, HookManifestKind, HookManifestScope};
use crate::registry::HookBindingScope;

/// Converts validated [`HookManifestEntry`] values into installed bindings +
/// dispatcher impls. One registrar per run; the shared
/// [`PredicateEvaluator`] threads sliding-window state across every
/// predicate-backed hook the registrar produces.
pub struct HookRegistrar {
    evaluator: Arc<PredicateEvaluator>,
}

impl HookRegistrar {
    pub fn new(evaluator: Arc<PredicateEvaluator>) -> Self {
        Self { evaluator }
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
        let identity_extension = ExtensionId(extension.as_str().to_string());
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

        let hook_version = HookVersion::ONE;
        let hook_id = HookId::derive(
            identity_extension,
            extension_version,
            &entry.id,
            hook_version,
        );
        let binding_scope = manifest_scope_to_binding_scope(entry.scope);

        match entry.body {
            HookManifestBody::Predicate { spec } => match entry.kind {
                HookManifestKind::BeforeCapability => {
                    let hook = PredicateBackedBeforeCapabilityHook::new(
                        hook_id,
                        spec,
                        Arc::clone(&self.evaluator),
                    );
                    builder
                        .dispatcher_mut()
                        .install_installed_before_capability(
                            hook_id,
                            entry.phase,
                            owning_extension.clone(),
                            binding_scope,
                            Box::new(hook),
                        )?;
                }
                other => {
                    return Err(HookError::RegistryConstruction(format!(
                        "predicate body is only supported for `before_capability` hooks; \
                         entry `{}` declared kind {:?}",
                        entry.id, other
                    )));
                }
            },
            HookManifestBody::Wasm { .. } => {
                return Err(HookError::RegistryConstruction(format!(
                    "WASM hook execution is not yet implemented; entry `{}` was \
                     rejected by the registrar",
                    entry.id
                )));
            }
        }

        Ok(hook_id)
    }
}

/// Map the manifest-declared scope to the dispatcher's runtime scope. The
/// manifest enum is parsed at install time; the dispatcher consults the
/// runtime enum on every invocation, so we eagerly translate here.
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
        ExtensionId(extension().as_str().to_string())
    }

    fn predicate_entry(local: &str) -> HookManifestEntry {
        HookManifestEntry {
            id: HookLocalId(local.to_string()),
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

    #[test]
    fn install_rejects_wasm_body_for_now() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let entry = HookManifestEntry {
            id: HookLocalId("wasm-hook".to_string()),
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
            .expect_err("wasm body must be rejected");
        match err {
            HookError::RegistryConstruction(msg) => {
                assert!(msg.contains("WASM"), "unexpected message: {msg}");
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

    #[tokio::test]
    async fn installer_propagates_owning_extension_and_scope_from_manifest() {
        // Two entries, distinct manifest scopes; assert each is reflected in
        // the resulting `HookBinding`.
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
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
}
