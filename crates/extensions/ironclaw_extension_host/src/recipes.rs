//! Auth-recipe resolution over resolved extension manifests.
//!
//! Implements the `ironclaw_auth::AuthRecipeResolver` port (overview §4.3):
//! recipe DATA per vendor id, resolved from the durable installation store —
//! never a string-keyed provider implementation lookup.
//!
//! Shared vendors (overview §3.2): every extension using a vendor embeds the
//! recipe; recipes for one vendor must be identical except scope and
//! presentation metadata, OAuth resource bindings must match exactly, the
//! scope ceiling is the union across extensions, and an incompatible pair is
//! a conflict.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthRecipeResolver, ResolvedVendorAuthRecipe};
use ironclaw_extension_contracts::recipe::VendorAuthRecipe;
use ironclaw_extension_registry::{ExtensionInstallationStorePort, ResolvedExtensionManifest};
use ironclaw_host_api::ids::{ExtensionId, UserId};

/// Two active extensions declared incompatible recipes for one vendor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "extensions `{first_extension}` and `{second_extension}` declare incompatible \
     [auth.{vendor}] recipes or OAuth resource bindings (shared-vendor recipes must be \
     identical except scope and presentation metadata, and OAuth bindings must match)"
)]
pub struct VendorRecipeConflict {
    pub vendor: String,
    pub first_extension: String,
    pub second_extension: String,
}

/// Unify the vendor recipes declared across `manifests` (overview §3.2):
/// Recipes that differ only in scope or presentation metadata merge with a
/// scope-ceiling union. OAuth recipes must also carry identical effective
/// resource and protected-resource metadata bindings; these fields are not
/// meaningful for API-key recipes. Anything else conflicts.
pub fn unified_vendor_recipes<'a>(
    manifests: impl IntoIterator<Item = &'a ResolvedExtensionManifest>,
) -> Result<Vec<ResolvedVendorAuthRecipe>, VendorRecipeConflict> {
    let mut unified: BTreeMap<String, (String, ResolvedVendorAuthRecipe)> = BTreeMap::new();
    for manifest in manifests {
        let extension_id = manifest.id.as_str().to_string();
        let mcp_resource = manifest.mcp.as_ref().map(|mcp| mcp.server.clone());
        for surface in &manifest.auth {
            let Some(recipe) = &surface.recipe else {
                // v2 manifests synthesize auth surfaces without recipes; they
                // contribute nothing the engine can execute.
                continue;
            };
            let vendor = surface.vendor.as_str().to_string();
            let resource = surface
                .oauth_resource
                .as_ref()
                .map(|resource| resource.as_str().to_string())
                .or_else(|| mcp_resource.clone());
            let incoming = ResolvedVendorAuthRecipe {
                vendor: vendor.clone(),
                recipe: recipe.clone(),
                token_exchange_resource: resource,
                protected_resource_metadata_url: surface.protected_resource_metadata_url.clone(),
            };
            match unified.get_mut(&vendor) {
                None => {
                    unified.insert(vendor.clone(), (extension_id.clone(), incoming));
                }
                Some((first_extension, existing)) => {
                    let oauth_bindings_match = match (&existing.recipe, &incoming.recipe) {
                        (VendorAuthRecipe::Oauth2Code(_), VendorAuthRecipe::Oauth2Code(_)) => {
                            existing.token_exchange_resource == incoming.token_exchange_resource
                                && existing.protected_resource_metadata_url
                                    == incoming.protected_resource_metadata_url
                        }
                        _ => true,
                    };
                    if !existing
                        .recipe
                        .compatible_for_shared_vendor(&incoming.recipe)
                        || !oauth_bindings_match
                    {
                        return Err(VendorRecipeConflict {
                            vendor,
                            first_extension: first_extension.clone(),
                            second_extension: extension_id.clone(),
                        });
                    }
                    if let (
                        VendorAuthRecipe::Oauth2Code(unified_recipe),
                        VendorAuthRecipe::Oauth2Code(incoming),
                    ) = (&mut existing.recipe, &incoming.recipe)
                    {
                        for scope in &incoming.scopes {
                            if !unified_recipe.scopes.contains(scope) {
                                unified_recipe.scopes.push(scope.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(unified.into_values().map(|(_, recipe)| recipe).collect())
}

/// Vendor-scoped resolver over the durable installation manifest source.
///
/// This deliberately reads the existing installation store instead of a
/// recipe sidecar or a vendor-global registry. Store failures and missing
/// manifests fail closed because a recipe is authorization-sensitive input.
/// Resolution is scoped to the vendor, not to the requesting extension: see
/// `resolve` for why a per-requester ceiling breaks extensions that share a
/// provider account.
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
        caller: Option<&UserId>,
        vendor: &str,
    ) -> Option<ResolvedVendorAuthRecipe> {
        // The scope ceiling is the union across every extension THIS CALLER
        // installed that declares the vendor — not every extension registered
        // in the deployment.
        //
        // Registration and installation are different layers: a manifest row
        // is tenant-wide ("this extension is registered here, and this is its
        // recipe"), while membership rows record who actually installed it.
        // Unioning over registrations pooled other users' extensions into this
        // caller's consent screen, so a user who installed one extension of a
        // vendor was asked to grant the scopes of a sibling extension that a
        // DIFFERENT user had installed (#7078).
        //
        // `list_installations` is the membership-aware read: it already drops
        // removed and lease-held rows, and carries the `InstallationOwner` that
        // `visible_to` needs.
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
        // silent-ok: installation/manifest reads for recipe resolution; AuthRecipeResolver is Option-valued, so a store failure must fail closed (no recipe) rather than resolve to none.
        let installations = self.store.list_installations().await.ok()?;
        let mut installed_for_caller: BTreeSet<ExtensionId> = installations
            .into_iter()
            .filter(|installation| match caller {
                Some(caller) => installation.owner().visible_to(caller),
                // No caller identity: fail closed to the requester's own
                // manifest rather than pooling the whole deployment.
                None => Some(installation.extension_id()) == requester_extension,
            })
            .map(|installation| installation.extension_id().clone())
            .collect();
        // The requester's OWN manifest is always in its own ceiling, whether or
        // not its installation row is currently listable. `list_installations`
        // hides rows that are removed or hold a mutation lease, so an install
        // still converging (WASM packages take a preparation lease) would
        // otherwise resolve a ceiling that omits the requesting extension's own
        // scopes — and the flow it is starting for itself would be rejected as
        // exceeding that ceiling. Callers reach this only for an extension the
        // route already authorized as installed for them.
        if let Some(requester_extension) = requester_extension {
            installed_for_caller.insert(requester_extension.clone());
        }
        let records = self.store.list_manifests().await.ok()?;
        let manifests: Vec<&ResolvedExtensionManifest> = records
            .iter()
            .filter(|record| installed_for_caller.contains(record.extension_id()))
            .map(|record| record.resolved())
            .collect();
        match unified_vendor_recipes(manifests) {
            Ok(recipes) => recipes.into_iter().find(|recipe| recipe.vendor == vendor),
            Err(conflict) => {
                // Activation-time conflict checks should have prevented this;
                // fail closed for the conflicting vendor rather than picking
                // an arbitrary declaration.
                tracing::warn!(
                    %conflict,
                    "installed manifests carry conflicting vendor recipes"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_registry::ResolvedAuthSurface;
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
            runtime: ironclaw_extension_registry::ExtensionRuntimeV2::FirstParty {
                service: format!("{extension}/v1"),
            },
            root_binding: ironclaw_extension_registry::PackageRootBinding::FabricateOnLoad,
            mcp: None,
            tools: Vec::new(),
            channel: None,
            memory: None,
            admin_configuration: Vec::new(),
            auth: vec![ResolvedAuthSurface {
                vendor: ironclaw_host_api::ids::VendorId::new(vendor).expect("vendor id"),
                setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
                recipe: Some(recipe),
                oauth_resource: None,
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
    fn legacy_manifest_without_oauth_resource_falls_back_to_mcp_server() {
        let mut manifest = manifest_with_recipe(
            "mcp-ext",
            "mcp-vendor",
            oauth_recipe(&[], "https://auth.example/token"),
        );
        manifest.mcp = Some(ironclaw_extension_registry::ResolvedMcpDeclaration {
            server: "https://mcp.example/mcp".to_string(),
            namespace: "mcp-ext".to_string(),
            max_tools: 16,
            default_permission: ironclaw_host_api::capability::PermissionMode::Deny,
            effects: Vec::new(),
            credential_handles: Vec::new(),
            dynamic_input_schemas: Default::default(),
            registration_auth:
                ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::OAuth {
                    client_profile_id: None,
                },
        });
        let legacy_wire = serde_json::to_value(&manifest).expect("legacy manifest serializes");
        assert!(
            legacy_wire["auth"][0].get("oauth_resource").is_none(),
            "the compatibility fixture must exercise the pre-field wire shape"
        );
        let restored: ResolvedExtensionManifest =
            serde_json::from_value(legacy_wire).expect("legacy manifest remains readable");

        let recipes = unified_vendor_recipes([&restored]).expect("legacy recipe resolves");
        assert_eq!(
            recipes[0].token_exchange_resource.as_deref(),
            Some("https://mcp.example/mcp")
        );
    }

    #[test]
    fn shared_vendor_recipes_allow_extension_specific_setup_copy() {
        let mut mail_recipe = oauth_recipe(&["mail:read"], "https://vendor.example/token");
        let VendorAuthRecipe::Oauth2Code(mail_oauth) = &mut mail_recipe else {
            panic!("oauth recipe");
        };
        mail_oauth.instructions = Some("Register the mail extension.".to_string());
        mail_oauth.setup_url = Some(
            ironclaw_extension_contracts::recipe::HttpsEndpoint::new(
                "https://vendor.example/settings/mail",
            )
            .expect("mail setup URL"),
        );

        let mut calendar_recipe = oauth_recipe(&["calendar:read"], "https://vendor.example/token");
        let VendorAuthRecipe::Oauth2Code(calendar_oauth) = &mut calendar_recipe else {
            panic!("oauth recipe");
        };
        calendar_oauth.instructions = Some("Register the calendar extension.".to_string());
        calendar_oauth.setup_url = Some(
            ironclaw_extension_contracts::recipe::HttpsEndpoint::new(
                "https://vendor.example/settings/calendar",
            )
            .expect("calendar setup URL"),
        );

        let mail = manifest_with_recipe("mail-ext", "vendorco", mail_recipe);
        let calendar = manifest_with_recipe("cal-ext", "vendorco", calendar_recipe);
        let unified = unified_vendor_recipes([&mail, &calendar])
            .expect("presentation-only setup differences must not conflict");

        let VendorAuthRecipe::Oauth2Code(recipe) = &unified[0].recipe else {
            panic!("oauth recipe");
        };
        assert_eq!(recipe.scopes, vec!["mail:read", "calendar:read"]);
        assert_eq!(
            recipe.instructions.as_deref(),
            Some("Register the mail extension."),
            "the first installed extension remains the presentation source"
        );
    }

    /// Resolution is vendor-scoped: unioning ceilings across installed
    /// extensions must never hand a caller a recipe for a different vendor.
    #[test]
    fn recipe_lookup_does_not_cross_vendor() {
        let manifest = manifest_with_recipe(
            "calendar-ext",
            "calendar-vendor",
            oauth_recipe(&["calendar:read"], "https://vendor.example/token"),
        );
        let recipes = unified_vendor_recipes([&manifest]).expect("single manifest unions cleanly");

        assert!(
            recipes
                .iter()
                .any(|recipe| recipe.vendor == "calendar-vendor")
        );
        assert!(!recipes.iter().any(|recipe| recipe.vendor == "other-vendor"));
    }
}
