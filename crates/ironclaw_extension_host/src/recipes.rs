//! Auth-recipe resolution over resolved extension manifests.
//!
//! Implements the `ironclaw_auth::AuthRecipeResolver` port (overview §4.3):
//! recipe DATA per vendor id, resolved from the active snapshot with a
//! fallback catalog (bundled manifests) — never a string-keyed provider
//! implementation lookup.
//!
//! Shared vendors (overview §3.2): every extension using a vendor embeds the
//! recipe; recipes for one vendor must be identical except `scopes` and
//! `display_name`, the scope ceiling is the union across extensions, and an
//! incompatible pair is a conflict.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthRecipeResolver, ResolvedVendorAuthRecipe};
use ironclaw_extensions::{ExtensionInstallationStorePort, ResolvedExtensionManifest};
use ironclaw_host_api::{ids::ExtensionId, recipe::VendorAuthRecipe};

use crate::SnapshotWatch;

/// Two active extensions declared incompatible recipes for one vendor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "extensions `{first_extension}` and `{second_extension}` declare incompatible \
     [auth.{vendor}] recipes (recipes for a shared vendor must be identical except \
     scopes and display_name)"
)]
pub struct VendorRecipeConflict {
    pub vendor: String,
    pub first_extension: String,
    pub second_extension: String,
}

/// Unify the vendor recipes declared across `manifests` (overview §3.2):
/// identical-except-`scopes`/`display_name` recipes merge with a scope-ceiling
/// union; anything else conflicts.
pub fn unified_vendor_recipes<'a>(
    manifests: impl IntoIterator<Item = &'a ResolvedExtensionManifest>,
) -> Result<Vec<ResolvedVendorAuthRecipe>, VendorRecipeConflict> {
    let mut unified: BTreeMap<String, (String, ResolvedVendorAuthRecipe)> = BTreeMap::new();
    for manifest in manifests {
        let extension_id = manifest.id.as_str().to_string();
        let resource = manifest.mcp.as_ref().map(|mcp| mcp.server.clone());
        for surface in &manifest.auth {
            let Some(recipe) = &surface.recipe else {
                // v2 manifests synthesize auth surfaces without recipes; they
                // contribute nothing the engine can execute.
                continue;
            };
            let vendor = surface.vendor.as_str().to_string();
            match unified.get_mut(&vendor) {
                None => {
                    unified.insert(
                        vendor.clone(),
                        (
                            extension_id.clone(),
                            ResolvedVendorAuthRecipe {
                                vendor,
                                recipe: recipe.clone(),
                                token_exchange_resource: resource.clone(),
                                protected_resource_metadata_url: surface
                                    .protected_resource_metadata_url
                                    .clone(),
                            },
                        ),
                    );
                }
                Some((first_extension, existing)) => {
                    if !existing.recipe.compatible_for_shared_vendor(recipe) {
                        return Err(VendorRecipeConflict {
                            vendor,
                            first_extension: first_extension.clone(),
                            second_extension: extension_id.clone(),
                        });
                    }
                    if let (
                        VendorAuthRecipe::Oauth2Code(unified_recipe),
                        VendorAuthRecipe::Oauth2Code(incoming),
                    ) = (&mut existing.recipe, recipe)
                    {
                        for scope in &incoming.scopes {
                            if !unified_recipe.scopes.contains(scope) {
                                unified_recipe.scopes.push(scope.clone());
                            }
                        }
                    }
                    if existing.token_exchange_resource.is_none() {
                        existing.token_exchange_resource = resource.clone();
                    }
                    if existing.protected_resource_metadata_url.is_none() {
                        existing.protected_resource_metadata_url =
                            surface.protected_resource_metadata_url.clone();
                    }
                }
            }
        }
    }
    Ok(unified.into_values().map(|(_, recipe)| recipe).collect())
}

/// Resolve one recipe from exactly one manifest. A caller can never borrow a
/// same-named vendor recipe from another installed extension.
fn recipe_for_manifest(
    manifest: &ResolvedExtensionManifest,
    vendor: &str,
) -> Option<ResolvedVendorAuthRecipe> {
    let surface = manifest
        .auth
        .iter()
        .find(|surface| surface.vendor.as_str() == vendor)?;
    let recipe = surface.recipe.clone()?;
    Some(ResolvedVendorAuthRecipe {
        vendor: vendor.to_string(),
        recipe,
        token_exchange_resource: manifest.mcp.as_ref().map(|mcp| mcp.server.clone()),
        protected_resource_metadata_url: surface.protected_resource_metadata_url.clone(),
    })
}

/// Requester-bound resolver over the durable installation manifest source.
///
/// This deliberately reads the existing installation store instead of a
/// recipe sidecar or a vendor-global registry. Store failures and missing
/// manifests fail closed because a recipe is authorization-sensitive input.
#[derive(Clone)]
pub struct InstalledManifestAuthRecipeResolver {
    store: Arc<dyn ExtensionInstallationStorePort>,
}

impl std::fmt::Debug for InstalledManifestAuthRecipeResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InstalledManifestAuthRecipeResolver")
            .finish_non_exhaustive()
    }
}

impl InstalledManifestAuthRecipeResolver {
    pub fn new(store: Arc<dyn ExtensionInstallationStorePort>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AuthRecipeResolver for InstalledManifestAuthRecipeResolver {
    async fn resolve(
        &self,
        requester_extension: Option<&ExtensionId>,
        vendor: &str,
    ) -> Option<ResolvedVendorAuthRecipe> {
        // The scope ceiling is the UNION across every installed extension
        // declaring this vendor, not just the requester's own manifest.
        //
        // This resolver is the path connect flows actually take: they run
        // before activation completes, so the active snapshot is still empty
        // and the snapshot resolver delegates here. Several extensions can
        // share one credential account for a vendor, and that account holds a
        // single scope set that each exchange replaces rather than merges — so
        // a per-requester ceiling clamps the grant to the requester's own
        // scopes and wipes every sibling's, leaving already-connected
        // extensions reporting that setup is still needed.
        //
        // silent-ok: list_manifests read for recipe resolution; AuthRecipeResolver is Option-valued, so a store failure must fail closed (no recipe) rather than resolve to none.
        let records = self.store.list_manifests().await.ok()?;
        let manifests: Vec<&ResolvedExtensionManifest> =
            records.iter().map(|record| record.resolved()).collect();
        match unified_vendor_recipes(manifests.into_iter()) {
            Ok(recipes) => recipes.into_iter().find(|recipe| recipe.vendor == vendor),
            Err(conflict) => {
                // Activation-time conflict checks should have prevented this;
                // fail closed for the conflicting vendor rather than picking
                // an arbitrary declaration.
                tracing::warn!(
                    %conflict,
                    "installed manifests carry conflicting vendor recipes"
                );
                let _ = requester_extension;
                None
            }
        }
    }
}

/// [`AuthRecipeResolver`] over the live active snapshot, with a fallback
/// resolver (typically the bundled-manifest catalog) for vendors whose
/// extension is installed but not yet active — connect flows run before
/// activation completes.
#[derive(Clone)]
pub struct SnapshotAuthRecipeResolver {
    watch: SnapshotWatch,
    fallback: Arc<dyn AuthRecipeResolver>,
}

impl std::fmt::Debug for SnapshotAuthRecipeResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SnapshotAuthRecipeResolver")
            .field("fallback", &self.fallback)
            .finish()
    }
}

impl SnapshotAuthRecipeResolver {
    pub fn new(watch: SnapshotWatch, fallback: Arc<dyn AuthRecipeResolver>) -> Self {
        Self { watch, fallback }
    }
}

#[async_trait]
impl AuthRecipeResolver for SnapshotAuthRecipeResolver {
    async fn resolve(
        &self,
        requester_extension: Option<&ExtensionId>,
        vendor: &str,
    ) -> Option<ResolvedVendorAuthRecipe> {
        // The scope ceiling for a vendor is the UNION across every installed
        // extension that uses it — not just the requesting extension's own
        // declaration.
        //
        // Several extensions can share one credential account for a vendor,
        // and that account stores a single scope set which each exchange
        // *replaces* rather than merges. Resolving a narrower, per-extension
        // ceiling therefore clamps the granted scopes to the requester's own
        // and overwrites every sibling's — so completing setup for one
        // extension silently strips the scopes of the ones already connected,
        // and they fall back to reporting that setup is still needed.
        let snapshot = self.watch.current();
        let manifests: Vec<Arc<ResolvedExtensionManifest>> = snapshot
            .extension_ids()
            .into_iter()
            .filter_map(|id| snapshot.extension(&id))
            .map(|extension| Arc::clone(&extension.resolved))
            .collect();
        match unified_vendor_recipes(manifests.iter().map(Arc::as_ref)) {
            Ok(recipes) => {
                if let Some(recipe) = recipes.into_iter().find(|recipe| recipe.vendor == vendor) {
                    return Some(recipe);
                }
            }
            Err(conflict) => {
                // Activation-time conflict checks should have prevented this;
                // fail closed for the conflicting vendor, still allow the
                // fallback catalog to answer.
                tracing::warn!(%conflict, "active snapshot carries conflicting vendor recipes");
            }
        }
        self.fallback.resolve(requester_extension, vendor).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extensions::ResolvedAuthSurface;
    use ironclaw_host_api::{capability::RuntimeCredentialAccountSetup, ids::ExtensionId};

    fn oauth_recipe(scopes: &[&str], token_endpoint: &str) -> VendorAuthRecipe {
        serde_json::from_value(serde_json::json!({
            "method": "oauth2_code",
            "display_name": "Vendor account",
            "authorization_endpoint": "https://vendor.example/authorize",
            "token_endpoint": token_endpoint,
            "scopes": scopes,
            "token_response": { "access_token": "/access_token" },
        }))
        .expect("recipe parses")
    }

    fn manifest_with_recipe(
        extension: &str,
        vendor: &str,
        recipe: VendorAuthRecipe,
    ) -> ResolvedExtensionManifest {
        ResolvedExtensionManifest {
            schema_version: "reborn.extension_manifest.v3".to_string(),
            id: ExtensionId::new(extension).expect("extension id"),
            name: extension.to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            requested_trust: ironclaw_host_api::trust::RequestedTrustClass::ThirdParty,
            runtime: ironclaw_extensions::ExtensionRuntimeV2::FirstParty {
                service: format!("{extension}/v1"),
            },
            root_binding: ironclaw_extensions::PackageRootBinding::FabricateOnLoad,
            mcp: None,
            tools: Vec::new(),
            channel: None,
            memory: None,
            admin_configuration: Vec::new(),
            auth: vec![ResolvedAuthSurface {
                vendor: ironclaw_host_api::ids::VendorId::new(vendor).expect("vendor id"),
                setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
                recipe: Some(recipe),
                protected_resource_metadata_url: None,
            }],
            host_apis: Vec::new(),
            section_surfaces: Vec::new(),
            hooks: Vec::new(),
        }
    }

    #[test]
    fn shared_vendor_recipes_union_scopes_and_reject_conflicts() {
        let first = manifest_with_recipe(
            "mail-ext",
            "vendorco",
            oauth_recipe(&["mail:read"], "https://vendor.example/token"),
        );
        let second = manifest_with_recipe(
            "cal-ext",
            "vendorco",
            oauth_recipe(&["cal:read", "mail:read"], "https://vendor.example/token"),
        );
        let unified = unified_vendor_recipes([&first, &second]).expect("compatible recipes unify");
        assert_eq!(unified.len(), 1);
        let VendorAuthRecipe::Oauth2Code(recipe) = &unified[0].recipe else {
            panic!("oauth recipe");
        };
        assert_eq!(recipe.scopes, vec!["mail:read", "cal:read"]);

        // A differing token endpoint is a conflict, not a silent last-wins.
        let conflicting = manifest_with_recipe(
            "docs-ext",
            "vendorco",
            oauth_recipe(&["docs:read"], "https://other.example/token"),
        );
        let error =
            unified_vendor_recipes([&first, &conflicting]).expect_err("incompatible recipes");
        assert_eq!(error.vendor, "vendorco");
        assert_eq!(error.first_extension, "mail-ext");
        assert_eq!(error.second_extension, "docs-ext");
    }

    #[test]
    fn requester_manifest_recipe_lookup_does_not_cross_vendor() {
        let manifest = manifest_with_recipe(
            "calendar-ext",
            "calendar-vendor",
            oauth_recipe(&["calendar:read"], "https://vendor.example/token"),
        );

        assert!(recipe_for_manifest(&manifest, "calendar-vendor").is_some());
        assert!(recipe_for_manifest(&manifest, "other-vendor").is_none());
    }
}
