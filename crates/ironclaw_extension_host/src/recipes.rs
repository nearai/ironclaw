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

use ironclaw_auth::{AuthRecipeResolver, ResolvedVendorAuthRecipe};
use ironclaw_extensions::ResolvedExtensionManifest;
use ironclaw_host_api::VendorAuthRecipe;

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
                }
            }
        }
    }
    Ok(unified.into_values().map(|(_, recipe)| recipe).collect())
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

impl AuthRecipeResolver for SnapshotAuthRecipeResolver {
    fn recipe_for_vendor(&self, vendor: &str) -> Option<ResolvedVendorAuthRecipe> {
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
        self.fallback.recipe_for_vendor(vendor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extensions::ResolvedAuthSurface;
    use ironclaw_host_api::{ExtensionId, RuntimeCredentialAccountSetup};

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
            requested_trust: ironclaw_host_api::RequestedTrustClass::ThirdParty,
            runtime: ironclaw_extensions::ExtensionRuntimeV2::FirstParty {
                service: format!("{extension}/v1"),
            },
            mcp: None,
            tools: Vec::new(),
            channel: None,
            memory: None,
            admin_configuration: Vec::new(),
            auth: vec![ResolvedAuthSurface {
                vendor: ironclaw_host_api::VendorId::new(vendor).expect("vendor id"),
                setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
                recipe: Some(recipe),
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
}
