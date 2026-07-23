//! Recipe-driven token exchange, refresh, JSON-pointer extraction, and
//! identity claims. Vendor differences (endpoints, parameter names, response
//! field paths, rotation semantics) arrive as data; the flow behavior is
//! implemented exactly once here.

use chrono::Utc;
use ironclaw_host_api::{
    MissingScopeBehavior, OAuth2CodeRecipe, PkceMode, SecretHandle, Timestamp, TokenExchangeAuth,
};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::{
    AuthFlowId, AuthProductError, CredentialAccountId, OAuthClientId, OAuthProviderCallbackRequest,
    OAuthProviderExchange, OAuthProviderExchangeContext, OAuthProviderIdentity,
    OAuthProviderRefresh, OAuthProviderRefreshRequest, ProviderScope,
};

use super::AuthEngine;
use super::http;

/// The client material + endpoints one flow actually uses: recipe endpoints
/// for statically-credentialed vendors, discovered endpoints for
/// dynamically-registered (DCR) vendors.
#[derive(Clone)]
pub(super) struct EffectiveOAuthClient {
    pub(super) client_id: OAuthClientId,
    pub(super) client_secret: Option<SecretString>,
    pub(super) authorization_endpoint: String,
    pub(super) token_endpoint: String,
}

/// Minimal OAuth error body — only the stable `error` code is extracted; the
/// body itself is never logged, stored, or returned.
#[derive(Debug, Deserialize)]
struct OAuthErrorResponseBody {
    #[serde(default)]
    error: Option<String>,
}

/// Parsed (redacted) token response.
pub(super) struct ExtractedTokenResponse {
    pub(super) access_token: SecretString,
    pub(super) refresh_token: Option<SecretString>,
    pub(super) expires_in_seconds: Option<u64>,
    pub(super) scopes: Vec<ProviderScope>,
    /// The raw parsed JSON, retained (in memory only) for identity-pointer
    /// extraction from the token response.
    body: serde_json::Value,
}

impl AuthEngine {
    pub(super) async fn execute_oauth_exchange(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
        recipe: Box<OAuth2CodeRecipe>,
        resource: Option<String>,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        let scope = context.scope.resource.clone();
        let vendor = request.provider.as_str().to_string();
        let client = self
            .oauth_exchange_client_material(
                &scope,
                context.flow_id,
                &vendor,
                &recipe,
                resource.as_deref(),
            )
            .await
            .map_err(|_| AuthProductError::TokenExchangeFailed)?;
        let redirect_uri = self
            .callback_base
            .redirect_uri_for(&vendor)
            .map_err(|_| AuthProductError::TokenExchangeFailed)?;

        let (headers, body) = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("grant_type", "authorization_code")
                .append_pair("code", request.authorization_code.expose_secret())
                .append_pair("redirect_uri", redirect_uri.as_str());
            if recipe.pkce == PkceMode::S256 {
                form.append_pair("code_verifier", request.pkce_verifier.expose_secret());
            }
            if let Some(resource) = &resource {
                form.append_pair("resource", resource);
            }
            token_request_headers_and_body(&recipe, &client, form)
        };

        let response = self
            .execute_credential_post(&scope, &client.token_endpoint, headers, body)
            .await?;
        if !(200..300).contains(&response.status) {
            if (500..600).contains(&response.status) {
                return Err(AuthProductError::BackendUnavailable);
            }
            log_vendor_error(&vendor, response.status, &response.body, "token exchange");
            return Err(AuthProductError::TokenExchangeFailed);
        }
        let extracted = extract_token_response(
            &recipe,
            &response.body,
            &request.scopes,
            ScopeClamp::ToRecipeCeiling,
        )
        .inspect_err(|_| {
            tracing::debug!(vendor, "token response extraction failed");
        })?;

        let provider_identity = self
            .extract_identity(&scope, &recipe, &extracted)
            .await
            .map_err(|_| AuthProductError::TokenExchangeFailed)?;

        let access_secret =
            exchange_token_handle(&vendor, context.flow_id, scope.invocation_id, "access")?;
        let refresh_secret = extracted
            .refresh_token
            .as_ref()
            .map(|_| {
                exchange_token_handle(&vendor, context.flow_id, scope.invocation_id, "refresh")
            })
            .transpose()?;
        let scopes = extracted.scopes.clone();
        let stored = self
            .store_token_pair(scope, access_secret, refresh_secret, &extracted)
            .await?;
        if recipe.client_credentials.is_some() {
            self.delete_flow_client_snapshot(&context.scope.resource, context.flow_id)
                .await;
        }

        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret: stored.access_secret,
            refresh_secret: stored.refresh_secret,
            scopes,
            account_id: None,
            provider_identity,
        })
    }

    pub(super) async fn execute_oauth_refresh(
        &self,
        request: OAuthProviderRefreshRequest,
        recipe: Box<OAuth2CodeRecipe>,
        resource: Option<String>,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        let scope = request.scope.resource.clone();
        let vendor = request.provider.as_str().to_string();
        let refresh_token = self
            .read_refresh_token(&scope, &request.refresh_secret)
            .await?;
        let client = self
            .oauth_client_material(&scope, &vendor, &recipe, resource.as_deref(), false)
            .await
            .map_err(|_| AuthProductError::RefreshFailed)?;

        let (headers, body) = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("grant_type", "refresh_token")
                .append_pair("refresh_token", refresh_token.expose_secret());
            if let Some(resource) = &resource {
                form.append_pair("resource", resource);
            }
            token_request_headers_and_body(&recipe, &client, form)
        };

        let response = self
            .execute_credential_post(&scope, &client.token_endpoint, headers, body)
            .await?;
        if !(200..300).contains(&response.status) {
            if (500..600).contains(&response.status) {
                return Err(AuthProductError::BackendUnavailable);
            }
            let error_code = serde_json::from_slice::<OAuthErrorResponseBody>(&response.body)
                .ok()
                .and_then(|body| body.error);
            tracing::warn!(
                vendor,
                status = response.status,
                oauth_error_code = error_code.as_deref().unwrap_or("<unparseable>"),
                "oauth refresh rejected by the vendor token endpoint"
            );
            if error_code.as_deref() == Some("invalid_grant") {
                return Err(AuthProductError::InvalidGrant);
            }
            return Err(AuthProductError::RefreshFailed);
        }
        let extracted = extract_token_response(
            &recipe,
            &response.body,
            &request.scopes,
            ScopeClamp::PreserveGranted,
        )
        .map_err(|error| match error {
            AuthProductError::BackendUnavailable => AuthProductError::BackendUnavailable,
            _ => AuthProductError::RefreshFailed,
        })?;

        let rotates = recipe
            .refresh
            .as_ref()
            .is_some_and(|refresh| refresh.rotates_refresh_token);
        // `rotates_refresh_token`, both ways (AUTH-6):
        // - rotating vendors return a replacement refresh token that must be
        //   persisted (write order below: refresh first, then access);
        // - non-rotating vendors keep the original refresh token valid, so a
        //   response without one preserves the existing stored handle —
        //   never orphaning the still-valid refresh token.
        let new_refresh_handle = extracted
            .refresh_token
            .as_ref()
            .map(|_| refresh_token_handle(&vendor, request.account_id, "refresh"))
            .transpose()?;
        if rotates && new_refresh_handle.is_none() {
            tracing::warn!(
                vendor,
                "vendor declared rotates_refresh_token but returned no replacement; \
                 keeping the previous refresh token"
            );
        }
        let access_secret = refresh_token_handle(&vendor, request.account_id, "access")?;
        let scopes = if extracted.scopes.is_empty() {
            request.scopes.clone()
        } else {
            extracted.scopes.clone()
        };
        let stored = self
            .store_token_pair(scope, access_secret, new_refresh_handle, &extracted)
            .await?;

        Ok(OAuthProviderRefresh {
            provider: request.provider,
            access_secret: stored.access_secret,
            refresh_secret: stored
                .refresh_secret
                .or_else(|| Some(request.refresh_secret.clone())),
            scopes,
        })
    }

    async fn read_refresh_token(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretString, AuthProductError> {
        let lease = self
            .secret_store
            .lease_once(scope, handle)
            .await
            .map_err(http::map_refresh_secret_error)?;
        self.secret_store
            .consume(scope, lease.id)
            .await
            .map_err(http::map_refresh_secret_error)
    }

    /// Identity extraction (AUTH-7): from the token response, or from the
    /// declared identity endpoint called with the freshly-issued credential.
    async fn extract_identity(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        recipe: &OAuth2CodeRecipe,
        token_response: &ExtractedTokenResponse,
    ) -> Result<Option<OAuthProviderIdentity>, AuthProductError> {
        let Some(identity) = &recipe.identity else {
            return Ok(None);
        };
        let claims_body: serde_json::Value = match &identity.endpoint {
            Some(endpoint) => {
                let response = self
                    .execute_vendor_get(
                        scope,
                        endpoint.url.as_str(),
                        vec![(
                            "authorization".to_string(),
                            format!("Bearer {}", token_response.access_token.expose_secret()),
                        )],
                    )
                    .await?;
                if !(200..300).contains(&response.status) {
                    tracing::debug!(
                        status = response.status,
                        "identity endpoint rejected the fresh credential"
                    );
                    return Err(AuthProductError::TokenExchangeFailed);
                }
                serde_json::from_slice(&response.body)
                    .map_err(|_| AuthProductError::TokenExchangeFailed)?
            }
            None => token_response.body.clone(),
        };
        let subject = pointer_string(&claims_body, identity.account_id.as_str())
            .ok_or(AuthProductError::TokenExchangeFailed)?;
        let claim = |name: &str| {
            identity
                .claims
                .get(name)
                .and_then(|pointer| pointer_string(&claims_body, pointer.as_str()))
        };
        OAuthProviderIdentity::new(
            subject,
            claim("team_id"),
            claim("enterprise_id"),
            claim("app_id"),
        )
        .map(Some)
        .map_err(|_| AuthProductError::TokenExchangeFailed)
    }

    /// Crash-safety write order: the rotated REFRESH secret is persisted
    /// FIRST, then the ACCESS secret carrying `expires_at`. If a crash
    /// separates the writes, the old access secret stays in place, expiry
    /// detection triggers a fresh refresh, and a fresh `expires_at` is never
    /// paired with a stale refresh token.
    async fn store_token_pair(
        &self,
        scope: ironclaw_host_api::ResourceScope,
        access_secret: SecretHandle,
        refresh_secret: Option<SecretHandle>,
        tokens: &ExtractedTokenResponse,
    ) -> Result<StoredTokenPair, AuthProductError> {
        let access_expires_at: Option<Timestamp> = tokens
            .expires_in_seconds
            // `expires_in: 0` means a non-expiring token; storing it literally
            // would mint an already-expired credential.
            .filter(|seconds| *seconds > 0)
            .and_then(|seconds| {
                let signed = seconds.min(i32::MAX as u64) as i64;
                Utc::now().checked_add_signed(chrono::Duration::seconds(signed))
            });

        let refresh_secret = match (refresh_secret, &tokens.refresh_token) {
            (Some(handle), Some(refresh_token)) => {
                self.secret_store
                    .put(
                        scope.clone(),
                        handle.clone(),
                        SecretString::from(refresh_token.expose_secret().to_string()),
                        None,
                    )
                    .await
                    .map_err(http::map_secret_store_error)?;
                Some(handle)
            }
            (None, None) => None,
            // A handle without a token (or vice versa) is an engine bug.
            _ => return Err(AuthProductError::BackendUnavailable),
        };

        // Access secret last; on failure the just-written refresh secret is
        // deliberately kept — deleting it would turn a transient storage
        // hiccup into a permanently unrecoverable credential.
        self.secret_store
            .put(
                scope,
                access_secret.clone(),
                SecretString::from(tokens.access_token.expose_secret().to_string()),
                access_expires_at,
            )
            .await
            .map_err(http::map_secret_store_error)?;

        Ok(StoredTokenPair {
            access_secret,
            refresh_secret,
        })
    }
}

pub(super) struct StoredTokenPair {
    pub(super) access_secret: SecretHandle,
    pub(super) refresh_secret: Option<SecretHandle>,
}

/// Client authentication on a token-class request (AUTH-5): `post_body`
/// appends the client credentials to the form; `basic` sends an RFC 6749
/// §2.3.1 Basic header. Consumes the form so the non-`Send` serializer never
/// crosses an await point.
fn token_request_headers_and_body(
    recipe: &OAuth2CodeRecipe,
    client: &EffectiveOAuthClient,
    mut form: url::form_urlencoded::Serializer<String>,
) -> (Vec<(String, String)>, Vec<u8>) {
    let mut headers = vec![
        (
            "content-type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        ),
        ("accept".to_string(), "application/json".to_string()),
    ];
    match recipe.exchange_auth {
        TokenExchangeAuth::PostBody => {
            form.append_pair("client_id", client.client_id.as_str());
            if let Some(secret) = &client.client_secret {
                form.append_pair("client_secret", secret.expose_secret());
            }
        }
        TokenExchangeAuth::Basic => {
            headers.push(http::basic_auth_header(
                &client.client_id,
                client.client_secret.as_ref(),
            ));
        }
    }
    (headers, form.finish().into_bytes())
}

/// Whether the A6 exchange-scope clamp (store `granted ∩ recipe ceiling`)
/// applies. Only the initial authorization-code exchange clamps; refresh
/// preserves the granted set so a vendor's rotation response is recorded as-is
/// (the account's scopes were already clamped at exchange time).
#[derive(Clone, Copy)]
pub(super) enum ScopeClamp {
    ToRecipeCeiling,
    PreserveGranted,
}

/// Bounded JSON-pointer extraction of the token response (AUTH-5).
pub(super) fn extract_token_response(
    recipe: &OAuth2CodeRecipe,
    body: &[u8],
    requested_scopes: &[ProviderScope],
    clamp: ScopeClamp,
) -> Result<ExtractedTokenResponse, AuthProductError> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| AuthProductError::TokenExchangeFailed)?;
    let map = &recipe.token_response;
    let access_token = pointer_string(&value, map.access_token.as_str())
        .filter(|token| !token.trim().is_empty())
        .map(SecretString::from)
        .ok_or(AuthProductError::TokenExchangeFailed)?;
    let refresh_token = map
        .refresh_token
        .as_ref()
        .and_then(|pointer| pointer_string(&value, pointer.as_str()))
        .filter(|token| !token.trim().is_empty())
        .map(SecretString::from);
    let expires_in_seconds = map
        .expires_in
        .as_ref()
        .and_then(|pointer| value.pointer(pointer.as_str()))
        .and_then(|field| {
            field
                .as_u64()
                .or_else(|| field.as_str().and_then(|text| text.parse().ok()))
        });
    let scopes = match &map.scope {
        None => Vec::new(),
        Some(extraction) => {
            let granted = pointer_string(&value, extraction.path.as_str())
                .map(|text| parse_scope_list(&text))
                .filter(|scopes| !scopes.is_empty());
            match granted {
                Some(granted) => {
                    let granted = granted
                        .into_iter()
                        .map(ProviderScope::new)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| AuthProductError::TokenExchangeFailed)?;
                    match clamp {
                        // A6 · Clamp the echoed grant to the recipe's declared
                        // scope ceiling (RFC 9700 §2.3): a scope no recipe ever
                        // declared is dropped (no over-claim). A scope granted
                        // beyond THIS flow's request but within the ceiling is
                        // kept — vendors with cumulative grants (opted into via
                        // recipe data, e.g. an `include_granted_scopes`-style
                        // authorize param) echo previously granted scopes on
                        // every exchange, and the stored account is shared by
                        // every extension using the vendor, so discarding them
                        // would silently sign the other extensions out (the
                        // shared-vendor-account sign-out regression). The
                        // per-flow request
                        // still drives the authorize URL and the downgrade warn
                        // below; it is not the storage bound. Generic and
                        // spec-agnostic — every vendor gets the same clamp.
                        // Applied only on the initial exchange; refresh
                        // preserves the granted set.
                        ScopeClamp::ToRecipeCeiling => {
                            let clamped: Vec<ProviderScope> = granted
                                .iter()
                                .filter(|scope| {
                                    recipe
                                        .scopes
                                        .iter()
                                        .any(|ceiling| ceiling == scope.as_str())
                                })
                                .cloned()
                                .collect();
                            let outside_ceiling = granted.len().saturating_sub(clamped.len());
                            let missing_requested = requested_scopes
                                .iter()
                                .filter(|scope| !clamped.contains(scope))
                                .count();
                            if outside_ceiling > 0 || missing_requested > 0 {
                                // Count-only guard log — never the scope values or
                                // the response body; the stored grant is never
                                // wider than granted ∩ ceiling.
                                tracing::warn!(
                                    requested_scope_count = requested_scopes.len(),
                                    granted_scope_count = clamped.len(),
                                    outside_ceiling_scope_count = outside_ceiling,
                                    missing_requested_scope_count = missing_requested,
                                    "oauth exchange grant differs from this flow's request; storing granted ∩ recipe ceiling"
                                );
                            }
                            if clamped.is_empty() {
                                // The vendor echoed only scopes outside every
                                // declared ceiling — no legitimate granted scope
                                // to store, so fall through to the missing-scope
                                // behavior exactly as an omitted scope would.
                                scopes_when_grant_absent(extraction.missing, requested_scopes)?
                            } else {
                                clamped
                            }
                        }
                        ScopeClamp::PreserveGranted => granted,
                    }
                }
                None => scopes_when_grant_absent(extraction.missing, requested_scopes)?,
            }
        }
    };
    Ok(ExtractedTokenResponse {
        access_token,
        refresh_token,
        expires_in_seconds,
        scopes,
        body: value,
    })
}

/// Fallback scope set when the vendor echoed no usable granted scope — either
/// none at all, or only scopes outside the request. RFC 6749 §3.3 makes an
/// omitted scope ⇒ requested legitimate (`FallbackToRequested`); `Reject` fails
/// closed. Shared by the no-scope and empty-intersection (A6) paths so both
/// honor the recipe's declared missing-scope behavior identically.
fn scopes_when_grant_absent(
    missing: MissingScopeBehavior,
    requested_scopes: &[ProviderScope],
) -> Result<Vec<ProviderScope>, AuthProductError> {
    match missing {
        MissingScopeBehavior::Reject => Err(AuthProductError::TokenExchangeFailed),
        MissingScopeBehavior::FallbackToRequested => {
            // Count-only log — a narrower actual grant must not be silently
            // widened without a trace.
            tracing::warn!(
                requested_scope_count = requested_scopes.len(),
                "token response carried no granted scopes; falling back to requested"
            );
            Ok(requested_scopes.to_vec())
        }
    }
}

/// Scope lists arrive space- or comma-separated depending on the vendor;
/// normalize both.
fn parse_scope_list(text: &str) -> Vec<String> {
    text.split([' ', ','])
        .filter(|scope| !scope.trim().is_empty())
        .map(|scope| scope.trim().to_string())
        .collect()
}

fn pointer_string(value: &serde_json::Value, pointer: &str) -> Option<String> {
    let field = value.pointer(pointer)?;
    match field {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn exchange_token_handle(
    vendor: &str,
    flow_id: AuthFlowId,
    invocation_id: ironclaw_host_api::InvocationId,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "{vendor}-oauth-{token_kind}-{flow_id}-{invocation_id}"
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

fn refresh_token_handle(
    vendor: &str,
    account_id: CredentialAccountId,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("{vendor}-oauth-refresh-{token_kind}-{account_id}"))
        .map_err(|_| AuthProductError::BackendUnavailable)
}

/// Log a vendor rejection without the body: only the stable OAuth `error`
/// code is extracted (never token material, never the raw payload).
fn log_vendor_error(vendor: &str, status: u16, body: &[u8], operation: &'static str) {
    let error_code = serde_json::from_slice::<OAuthErrorResponseBody>(body)
        .ok()
        .and_then(|body| body.error);
    tracing::warn!(
        vendor,
        status,
        oauth_error_code = error_code.as_deref().unwrap_or("<unparseable>"),
        operation,
        "vendor rejected the request"
    );
}
