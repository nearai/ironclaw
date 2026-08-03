//! The host auth engine (`docs/reborn/extension-runtime/overview.md` §4.3).
//!
//! One engine implements `oauth2_code` (with PKCE) and RFC 7591 dynamic client
//! registration for vendors whose recipe carries no deployment client
//! credentials. Vendors differ only in recipe **data**
//! (`ironclaw_extension_contracts::recipe::VendorAuthRecipe`); there is no auth trait in the
//! extension ABI and no per-vendor code path here.
//!
//! Engine-owned, for every vendor:
//! - host-constructed authorize URLs (recipes can never supply or override
//!   `state`, `redirect_uri`, PKCE, `client_id`, `response_type`, or the
//!   scope parameter),
//! - scope requests validated against the recipe ceiling before any vendor
//!   call: a host flow's explicit scopes are honored verbatim, an
//!   extension-scoped flow's scopes are validated as a lower bound and the
//!   full ceiling is requested instead (the vendor account is shared across
//!   that vendor's installed extensions, #7069),
//! - token exchange over `post_body` or `basic` client authentication,
//! - bounded JSON-pointer extraction of token-response and identity fields,
//! - on-demand refresh honoring `rotates_refresh_token` both ways,
//! - the auth-account state machine ([`crate::AuthAccountState`]).
//!
//! Vendor response bodies are size-capped and never logged or embedded in
//! errors; only stable OAuth error codes (`invalid_grant`, …) are extracted.

pub mod admission;
mod dcr;
mod exchange;
mod http;
pub mod keepalive;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::recipe::{
    OAuth2CodeRecipe, PkceMode, RecipeClientCredentials, VendorAuthRecipe,
};
use ironclaw_host_api::{
    http::RuntimeHttpEgress,
    ids::{ExtensionId, UserId},
    resource::ResourceScope,
};
use ironclaw_secrets::SecretStorePort;
use secrecy::SecretString;
use url::Url;

use crate::{
    AuthFlowId, AuthProductError, AuthProductScope, AuthProviderClient, AuthProviderId,
    CredentialAccountLabel, OAuthAuthorizationUrl, OAuthCallbackState, OAuthCallbackStateKind,
    OAuthClientId, OAuthProviderCallbackRequest, OAuthProviderExchange,
    OAuthProviderExchangeContext, OAuthProviderRefresh, OAuthProviderRefreshRequest,
    OAuthRedirectUri, OAuthState, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret,
    ProviderScope, opaque_state_hash, pkce_s256_challenge, pkce_verifier_hash,
    validate_provider_callback_request,
};

pub use dcr::DCR_CLIENT_HANDLE_PREFIX;

/// One vendor's recipe, resolved from active extensions or bundled manifests.
///
/// `token_exchange_resource` is the RFC 8707 resource indicator sent with
/// token requests — for hosted-MCP vendors this is the manifest's
/// `[mcp].server` URL, i.e. still manifest data, never engine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVendorAuthRecipe {
    pub vendor: String,
    pub recipe: VendorAuthRecipe,
    pub token_exchange_resource: Option<String>,
    /// The exact protected-resource metadata document admitted from the MCP
    /// authorization challenge. Dynamic registration must use this location,
    /// rather than reconstructing a well-known path from the resource URL.
    pub protected_resource_metadata_url:
        Option<ironclaw_extension_contracts::recipe::HttpsEndpoint>,
}

/// Resolver port: recipe DATA for a vendor id (never adapters, never code).
///
/// Defined here (the engine is the consumer); implemented over the active
/// extension snapshot and the bundled-manifest catalog by the host/composition
/// layers. Shared vendors resolve to one unified recipe (identical except
/// `scopes`/`display_name`, scope ceiling = union) or fail resolution.
#[async_trait]
pub trait AuthRecipeResolver: Send + Sync + fmt::Debug {
    /// Resolve only the recipe declared by the requesting installed extension.
    /// `requester_extension` is `None` for built-in/static callers;
    /// installed-manifest resolvers must fail closed when it is absent.
    ///
    /// `caller` is the user the flow authorizes for. A shared vendor's scope
    /// ceiling is the union across the extensions that user INSTALLED, so a
    /// resolver reading installation state must narrow to them — registration
    /// is tenant-wide, installation is per user, and pooling the two would put
    /// another user's extensions on this user's consent screen.
    async fn resolve(
        &self,
        requester_extension: Option<&ExtensionId>,
        caller: Option<&UserId>,
        vendor: &str,
    ) -> Option<ResolvedVendorAuthRecipe>;
}

/// Static in-memory recipe resolver for composition and tests.
#[derive(Debug, Clone, Default)]
pub struct StaticAuthRecipeResolver {
    recipes: BTreeMap<String, ResolvedVendorAuthRecipe>,
}

impl StaticAuthRecipeResolver {
    pub fn new(recipes: Vec<ResolvedVendorAuthRecipe>) -> Self {
        Self {
            recipes: recipes
                .into_iter()
                .map(|recipe| (recipe.vendor.clone(), recipe))
                .collect(),
        }
    }

    pub fn vendors(&self) -> Vec<String> {
        self.recipes.keys().cloned().collect()
    }

    /// Synchronous deployment lookup for composition-time static client
    /// material. Runtime callers must use the requester-bound resolver port.
    pub fn recipe_for_vendor(&self, vendor: &str) -> Option<ResolvedVendorAuthRecipe> {
        self.recipes.get(vendor).cloned()
    }
}

#[async_trait]
impl AuthRecipeResolver for StaticAuthRecipeResolver {
    async fn resolve(
        &self,
        _requester_extension: Option<&ExtensionId>,
        _caller: Option<&UserId>,
        vendor: &str,
    ) -> Option<ResolvedVendorAuthRecipe> {
        self.recipes.get(vendor).cloned()
    }
}

/// Deployment-level OAuth client material resolved from the recipe's
/// `client_credentials` handles.
#[derive(Clone)]
pub struct EngineOAuthClientMaterial {
    pub client_id: OAuthClientId,
    pub client_secret: Option<SecretString>,
}

impl fmt::Debug for EngineOAuthClientMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EngineOAuthClientMaterial")
            .field("client_id", &"[REDACTED]")
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Port resolving the deployment client credentials a recipe names by handle.
///
/// Implementations look the handles up in operator-managed secret storage /
/// deployment configuration. Returning `MalformedConfig` means the operator
/// has not configured the vendor's client credentials yet.
#[async_trait]
pub trait EngineClientCredentialsSource: Send + Sync + fmt::Debug {
    async fn resolve(
        &self,
        vendor: &str,
        credentials: &RecipeClientCredentials,
    ) -> Result<EngineOAuthClientMaterial, AuthProductError>;
}

/// The static callback base every vendor callback hangs off:
/// `{base}/{vendor}/callback` (AUTH-13 keeps the existing
/// `/api/reborn/product-auth/oauth/{provider}/callback` shape).
#[derive(Debug, Clone)]
pub struct EngineCallbackBase {
    base: String,
}

impl EngineCallbackBase {
    pub fn new(base: impl Into<String>) -> Result<Self, AuthProductError> {
        let base = base.into();
        let base = base.trim_end_matches('/').to_string();
        let url = Url::parse(&base)
            .map_err(|_| AuthProductError::invalid_request("callback base must be a url"))?;
        let is_loopback_http = url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]"));
        if url.scheme() != "https" && !is_loopback_http {
            return Err(AuthProductError::invalid_request(
                "callback base must use https unless it targets loopback localhost",
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(AuthProductError::invalid_request(
                "callback base must not carry a query or fragment",
            ));
        }
        Ok(Self { base })
    }

    pub fn redirect_uri_for(&self, vendor: &str) -> Result<OAuthRedirectUri, AuthProductError> {
        OAuthRedirectUri::new(format!("{}/{vendor}/callback", self.base))
    }
}

/// Engine construction inputs.
pub struct AuthEngineDeps {
    pub recipes: Arc<dyn AuthRecipeResolver>,
    pub client_credentials: Arc<dyn EngineClientCredentialsSource>,
    pub egress: Arc<dyn RuntimeHttpEgress>,
    pub secret_store: Arc<dyn SecretStorePort>,
    pub callback_base: EngineCallbackBase,
    /// `client_name` sent with RFC 7591 dynamic client registration.
    pub dcr_client_name: String,
}

/// Prepare-flow input: everything the engine needs to mint a vendor
/// authorization URL for one flow.
#[derive(Debug, Clone)]
pub struct PrepareOAuthFlowRequest {
    pub vendor: String,
    /// Extension that declared the recipe. Built-in/static flows use `None`.
    pub requester_extension: Option<ExtensionId>,
    pub scope: AuthProductScope,
    pub flow_id: AuthFlowId,
    pub account_label: CredentialAccountLabel,
    /// Requested scopes; empty means "the recipe's full scope ceiling".
    pub requested_scopes: Vec<ProviderScope>,
}

/// Host-constructed flow material. The raw PKCE verifier is returned exactly
/// once for the caller's verifier store; the durable flow record carries only
/// the hashes.
pub struct PreparedOAuthFlow {
    pub provider: AuthProviderId,
    /// Durable requester identity the flow record must retain for later
    /// recipe revalidation.
    pub requester_extension: Option<ExtensionId>,
    pub authorization_url: OAuthAuthorizationUrl,
    pub requested_scopes: Vec<ProviderScope>,
    pub opaque_state_hash: OpaqueStateHash,
    pub pkce_verifier_hash: PkceVerifierHash,
    pub pkce_verifier: SecretString,
}

impl fmt::Debug for PreparedOAuthFlow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedOAuthFlow")
            .field("provider", &self.provider)
            .field("requested_scopes", &self.requested_scopes)
            .field("pkce_verifier", &"[REDACTED]")
            .finish()
    }
}

/// The recipe-driven auth engine. Implements [`AuthProviderClient`] so the
/// existing durable flow/grant/account services drive it unchanged.
pub struct AuthEngine {
    recipes: Arc<dyn AuthRecipeResolver>,
    client_credentials: Arc<dyn EngineClientCredentialsSource>,
    egress: Arc<dyn RuntimeHttpEgress>,
    secret_store: Arc<dyn SecretStorePort>,
    callback_base: EngineCallbackBase,
    dcr_client_name: String,
    /// Serializes dynamic client registration so concurrent flows for one
    /// vendor register exactly one client.
    dcr_registration_lock: tokio::sync::Mutex<()>,
}

impl fmt::Debug for AuthEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthEngine")
            .field("recipes", &self.recipes)
            .field("callback_base", &self.callback_base)
            .finish()
    }
}

impl AuthEngine {
    pub fn new(deps: AuthEngineDeps) -> Self {
        Self {
            recipes: deps.recipes,
            client_credentials: deps.client_credentials,
            egress: deps.egress,
            secret_store: deps.secret_store,
            callback_base: deps.callback_base,
            dcr_client_name: deps.dcr_client_name,
            dcr_registration_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn recipes(&self) -> &Arc<dyn AuthRecipeResolver> {
        &self.recipes
    }

    async fn resolved_recipe(
        &self,
        requester_extension: Option<&ExtensionId>,
        caller: Option<&UserId>,
        vendor: &str,
    ) -> Result<ResolvedVendorAuthRecipe, AuthProductError> {
        match self
            .recipes
            .resolve(requester_extension, caller, vendor)
            .await
        {
            Some(resolved) => Ok(resolved),
            None => {
                // Every caller maps this to an opaque `malformed_config`
                // (HTTP 503 on the connect route), so without this line an
                // unresolvable recipe leaves no server-side trace of WHICH
                // requester/vendor pair could not be resolved.
                tracing::warn!(
                    vendor = %vendor,
                    requester_extension = requester_extension.map(ExtensionId::as_str).unwrap_or("<host>"),
                    "no auth recipe resolved for this requester and vendor"
                );
                Err(AuthProductError::MalformedConfig)
            }
        }
    }

    async fn oauth2_recipe(
        &self,
        requester_extension: Option<&ExtensionId>,
        caller: Option<&UserId>,
        vendor: &str,
    ) -> Result<
        (
            Box<OAuth2CodeRecipe>,
            Option<String>,
            Option<ironclaw_extension_contracts::recipe::HttpsEndpoint>,
        ),
        AuthProductError,
    > {
        let resolved = self
            .resolved_recipe(requester_extension, caller, vendor)
            .await?;
        match resolved.recipe {
            VendorAuthRecipe::Oauth2Code(recipe) => Ok((
                recipe,
                resolved.token_exchange_resource,
                resolved.protected_resource_metadata_url,
            )),
            VendorAuthRecipe::ApiKey(_) => Err(AuthProductError::MalformedConfig),
        }
    }

    /// Resolve the client material and effective endpoints for a vendor:
    /// deployment `client_credentials` handles when the recipe declares them,
    /// or the persisted dynamically-registered client (RFC 7591) when it does
    /// not. `register_if_missing` is true on flow preparation (registration
    /// side effect allowed) and false on exchange/refresh (the client must
    /// already exist).
    async fn oauth_client_material(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        recipe: &OAuth2CodeRecipe,
        resource: Option<&str>,
        protected_resource_metadata_url: Option<
            &ironclaw_extension_contracts::recipe::HttpsEndpoint,
        >,
        register_if_missing: bool,
    ) -> Result<exchange::EffectiveOAuthClient, AuthProductError> {
        if let Some(credentials) = &recipe.client_credentials {
            let material = self.client_credentials.resolve(vendor, credentials).await?;
            return Ok(exchange::EffectiveOAuthClient {
                client_id: material.client_id,
                client_secret: material.client_secret,
                authorization_endpoint: recipe.authorization_endpoint.as_str().to_string(),
                token_endpoint: recipe.token_endpoint.as_str().to_string(),
            });
        }
        // No deployment client credentials: dynamic client registration is
        // the generic hosted-MCP behavior, implemented once here.
        self.dcr_client(
            scope,
            vendor,
            resource,
            protected_resource_metadata_url,
            register_if_missing,
        )
        .await
    }

    /// Host-constructed authorize URL + state + PKCE for one vendor flow
    /// (AUTH-2/AUTH-4). Scope widening beyond the recipe ceiling is rejected
    /// here — before any vendor interaction.
    pub async fn prepare_oauth_flow(
        &self,
        request: PrepareOAuthFlowRequest,
    ) -> Result<PreparedOAuthFlow, AuthProductError> {
        let (recipe, resource, protected_resource_metadata_url) = self
            .oauth2_recipe(
                request.requester_extension.as_ref(),
                Some(&request.scope.resource.user_id),
                &request.vendor,
            )
            .await?;
        // Enforce the recipe invariants at execution time; manifest-parse
        // validation is not trusted alone (AUTH-2).
        recipe
            .validate()
            .map_err(|_| AuthProductError::MalformedConfig)?;
        let requested_scopes = effective_requested_scopes(
            &recipe,
            request.requested_scopes.clone(),
            request.requester_extension.as_ref(),
        )?;
        let client = self
            .oauth_client_material(
                &request.scope.resource,
                &request.vendor,
                &recipe,
                resource.as_deref(),
                protected_resource_metadata_url.as_ref(),
                true,
            )
            .await?;
        let redirect_uri = self.callback_base.redirect_uri_for(&request.vendor)?;
        let provider = AuthProviderId::new(request.vendor.clone())?;

        let state = OAuthCallbackState::new(
            OAuthCallbackStateKind::RECIPE,
            request.flow_id,
            request.scope.clone(),
            request.account_label.clone(),
            requested_scopes.clone(),
        )?
        .encode()?;
        let opaque_state_hash = opaque_state_hash(state.as_str())?;
        let pkce_verifier = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
        let pkce_secret = PkceVerifierSecret::new(pkce_verifier.clone())?;
        let verifier_hash = pkce_verifier_hash(&pkce_secret)?;
        let authorization_url = build_recipe_authorization_url(
            &recipe,
            &client,
            &redirect_uri,
            &state,
            &pkce_secret,
            &requested_scopes,
        )?;

        Ok(PreparedOAuthFlow {
            provider,
            requester_extension: request.requester_extension,
            authorization_url,
            requested_scopes,
            opaque_state_hash,
            pkce_verifier_hash: verifier_hash,
            pkce_verifier,
        })
    }
}

#[async_trait]
impl AuthProviderClient for AuthEngine {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        validate_provider_callback_request(&request)?;
        let callback_scope = context.scope.resource.clone();
        if callback_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let (recipe, resource, _) = self
            .oauth2_recipe(
                None,
                Some(&context.scope.resource.user_id),
                request.provider.as_str(),
            )
            .await
            .map_err(|_| AuthProductError::TokenExchangeFailed)?;
        // Widening past the ceiling is rejected before the vendor call, on
        // the exchange path too (defense in depth over prepare-time checks).
        validate_scopes_within_ceiling(&recipe, &request.scopes)?;
        self.execute_oauth_exchange(context, request, recipe, resource)
            .await
    }

    async fn exchange_callback_for_requester(
        &self,
        requester_extension: Option<ExtensionId>,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        validate_provider_callback_request(&request)?;
        if context.scope.resource.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let (recipe, resource, _) = self
            .oauth2_recipe(
                requester_extension.as_ref(),
                Some(&context.scope.resource.user_id),
                request.provider.as_str(),
            )
            .await
            .map_err(|_| AuthProductError::TokenExchangeFailed)?;
        let request = match &requester_extension {
            Some(_) => {
                // An extension-scoped flow persists the shared-vendor ceiling as
                // it stood at PREPARE time. A sibling extension uninstalled
                // while the user was on the vendor's consent screen shrinks
                // that ceiling, and rejecting here would fail this flow's
                // callback over an unrelated extension's removal (lifecycle
                // cleanup deliberately does not cancel shared-provider flows).
                // Clamp to the CURRENT ceiling instead: the stored grant still
                // can never exceed what is authorized right now — including on
                // the `fallback_to_requested` path, where the exchange echoes
                // these scopes without clamping them itself.
                clamp_callback_scopes_to_ceiling(&recipe, request)
            }
            None => {
                // A HOST flow has no sibling extension whose uninstall could
                // legitimately shrink the ceiling mid-flow, so silently
                // dropping out-of-ceiling scopes here would only weaken the
                // exchange-time defense-in-depth check and hide
                // misconfiguration. Reject instead, matching `exchange_callback`.
                validate_scopes_within_ceiling(&recipe, &request.scopes)?;
                request
            }
        };
        self.execute_oauth_exchange(context, request, recipe, resource)
            .await
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        let refresh_scope = request.scope.resource.clone();
        if refresh_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let (recipe, resource, _) = self
            .oauth2_recipe(
                None,
                Some(&request.scope.resource.user_id),
                request.provider.as_str(),
            )
            .await
            .map_err(|_| AuthProductError::RefreshFailed)?;
        self.execute_oauth_refresh(request, recipe, resource).await
    }

    async fn refresh_token_for_requester(
        &self,
        requester_extension: Option<ExtensionId>,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        let refresh_scope = request.scope.resource.clone();
        if refresh_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let (recipe, resource, _) = self
            .oauth2_recipe(
                requester_extension.as_ref(),
                Some(&request.scope.resource.user_id),
                request.provider.as_str(),
            )
            .await
            .map_err(|_| AuthProductError::RefreshFailed)?;
        self.execute_oauth_refresh(request, recipe, resource).await
    }

    async fn cleanup_exchange(
        &self,
        context: OAuthProviderExchangeContext,
        exchange: &OAuthProviderExchange,
    ) -> Result<(), AuthProductError> {
        let mut first_error = None;
        let mut handles = vec![exchange.access_secret.clone()];
        handles.extend(exchange.refresh_secret.clone());
        for handle in &handles {
            if let Err(error) = self
                .secret_store
                .delete(&context.scope.resource, handle)
                .await
                && first_error.is_none()
            {
                first_error = Some(http::map_secret_store_error(error));
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// The scopes this flow asks the vendor for (AUTH-4).
///
/// An empty request means the recipe's full ceiling. A HOST flow
/// (`requester_extension` is `None`) is authorizing on its own behalf, so an
/// explicit request is honored verbatim once validated.
///
/// An EXTENSION-scoped flow is different: it authorizes the vendor account
/// that every installed extension of that vendor SHARES, and the recipe
/// ceiling is already the union across those installed manifests
/// (`ironclaw_extension_host::unified_vendor_recipes`). The account holds one
/// scope set that each exchange replaces, and dispatch requires a
/// capability's scopes to already be on it, so asking for only the requesting
/// extension's slice forces a separate consent per sibling and leaves every
/// not-yet-authorized sibling returning `auth_required` (#7069). Such a
/// request is therefore validated as a LOWER BOUND — the caller's scopes must
/// still be within the ceiling — and the ceiling is what gets requested; the
/// returned value deliberately does not echo the input.
fn effective_requested_scopes(
    recipe: &OAuth2CodeRecipe,
    requested: Vec<ProviderScope>,
    requester_extension: Option<&ExtensionId>,
) -> Result<Vec<ProviderScope>, AuthProductError> {
    if !requested.is_empty() {
        validate_scopes_within_ceiling(recipe, &requested)?;
        if requester_extension.is_none() {
            return Ok(requested);
        }
    }
    recipe
        .scopes
        .iter()
        .map(|scope| ProviderScope::new(scope.clone()))
        .collect()
}

/// Drop callback scopes the vendor recipe no longer declares.
///
/// Used only on the EXTENSION-scoped arm of the requester-scoped exchange
/// (`requester_extension.is_some()`), where the persisted request is the
/// prepare-time shared-vendor ceiling and may name a scope a sibling
/// extension has since taken away. The HOST arm (`requester_extension`
/// is `None`) does not call this — it validates instead, since there is no
/// sibling extension whose uninstall could legitimately shrink the ceiling.
/// The result is always a subset of the current ceiling, so it is never
/// wider than the host path's [`validate_scopes_within_ceiling`] would have
/// permitted.
fn clamp_callback_scopes_to_ceiling(
    recipe: &OAuth2CodeRecipe,
    mut request: OAuthProviderCallbackRequest,
) -> OAuthProviderCallbackRequest {
    request
        .scopes
        .retain(|scope| scope_in_ceiling(recipe, scope));
    request
}

/// The one membership rule deciding whether a scope is inside a recipe's
/// ceiling. Both the clamp above and [`validate_scopes_within_ceiling`] answer
/// "may this caller keep this scope", so they must never diverge on it.
fn scope_in_ceiling(recipe: &OAuth2CodeRecipe, scope: &ProviderScope) -> bool {
    recipe
        .scopes
        .iter()
        .any(|ceiling| ceiling == scope.as_str())
}

fn validate_scopes_within_ceiling(
    recipe: &OAuth2CodeRecipe,
    requested: &[ProviderScope],
) -> Result<(), AuthProductError> {
    for scope in requested {
        if !scope_in_ceiling(recipe, scope) {
            return Err(AuthProductError::invalid_request(
                "requested scopes exceed the vendor recipe scope ceiling",
            ));
        }
    }
    Ok(())
}

/// Build the authorization URL from recipe data. The host appends every
/// reserved protocol parameter itself; the recipe contributes only endpoints,
/// the scope parameter name/joiner, and validated extra params — a recipe
/// that names a reserved parameter was already rejected by
/// `OAuth2CodeRecipe::validate` (re-run by the caller).
fn build_recipe_authorization_url(
    recipe: &OAuth2CodeRecipe,
    client: &exchange::EffectiveOAuthClient,
    redirect_uri: &OAuthRedirectUri,
    state: &OAuthState,
    pkce_verifier: &PkceVerifierSecret,
    scopes: &[ProviderScope],
) -> Result<OAuthAuthorizationUrl, AuthProductError> {
    let mut url = Url::parse(&client.authorization_endpoint)
        .map_err(|_| AuthProductError::MalformedConfig)?;
    if url.scheme() != "https" {
        return Err(AuthProductError::MalformedConfig);
    }
    // The endpoint may not predefine reserved parameters (host-owned).
    for (name, _) in url.query_pairs() {
        let name = name.to_ascii_lowercase();
        if ironclaw_extension_contracts::recipe::RESERVED_AUTHORIZE_PARAMS.contains(&name.as_str())
            || name == recipe.scope_param()
        {
            return Err(AuthProductError::MalformedConfig);
        }
    }
    let scope_text = scopes
        .iter()
        .map(ProviderScope::as_str)
        .collect::<Vec<_>>()
        .join(recipe.scope_join.separator());
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("client_id", client.client_id.as_str())
            .append_pair("redirect_uri", redirect_uri.as_str())
            .append_pair("response_type", "code")
            .append_pair(recipe.scope_param(), &scope_text)
            .append_pair("state", state.as_str());
        if recipe.pkce == PkceMode::S256 {
            let challenge = pkce_s256_challenge(pkce_verifier);
            pairs
                .append_pair("code_challenge", challenge.as_str())
                .append_pair("code_challenge_method", "S256");
        }
        for (name, value) in &recipe.extra_authorize_params {
            pairs.append_pair(name, value);
        }
    }
    OAuthAuthorizationUrl::new(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_callback_base_builds_vendor_redirects() {
        let base =
            EngineCallbackBase::new("https://host.example/api/reborn/product-auth/oauth").unwrap();
        assert_eq!(
            base.redirect_uri_for("acme").unwrap().as_str(),
            "https://host.example/api/reborn/product-auth/oauth/acme/callback"
        );
        assert!(EngineCallbackBase::new("http://host.example/oauth").is_err());
        assert!(EngineCallbackBase::new("http://127.0.0.1:3000/oauth").is_ok());
        assert!(EngineCallbackBase::new("https://host.example/oauth?x=1").is_err());
    }
}
