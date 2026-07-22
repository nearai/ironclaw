//! The auth-engine conformance suite (checklist AUTH-1..12): ONE table-driven
// arch-exempt: large_file, table-driven auth conformance remains one contract suite, plan #6175
//! contract over recipe data against a scripted vendor HTTP server. Vendors
//! are rows — the five real vendors' recipes are loaded from their bundled
//! manifests (`crates/ironclaw_first_party_extensions/assets`), synthetic rows
//! cover recipe shapes no current vendor exercises. There is deliberately no
//! per-vendor test suite anywhere else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthEngine, AuthEngineDeps, AuthErrorCode, AuthFlowId,
    AuthFlowKind, AuthFlowManager, AuthProductError, AuthProductScope, AuthProviderClient,
    AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountId,
    CredentialAccountLabel, EngineCallbackBase, EngineOAuthClientMaterial,
    EngineOAuthConfigurationSource, InMemoryAuthProductServices, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthCallbackClaimRequest, OAuthCallbackInput,
    OAuthClientId, OAuthProviderCallbackRequest, OAuthProviderExchange,
    OAuthProviderExchangeContext, OAuthProviderRefreshRequest, OpaqueStateHash, PkceVerifierHash,
    PkceVerifierSecret, PrepareOAuthFlowRequest, ProviderCallbackOutcome, ProviderScope,
    ResolvedVendorAuthRecipe, StaticAuthRecipeResolver,
};
use ironclaw_host_api::{
    InvocationId, RecipeClientCredentials, ResourceScope, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle,
    UserId, VendorAuthRecipe,
};
use ironclaw_secrets::{FilesystemSecretStore, SecretStore};
use secrecy::{ExposeSecret, SecretString};

// ---------------------------------------------------------------------------
// Scripted vendor server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CapturedRequest {
    fn form(&self) -> HashMap<String, String> {
        url::form_urlencoded::parse(&self.body)
            .into_owned()
            .collect()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

type ScriptedResponses = Mutex<HashMap<String, std::collections::VecDeque<(u16, Vec<u8>)>>>;

/// Scripted vendor HTTP server: responses are queued per exact URL; every
/// request is captured for assertion. Unscripted URLs return 404 so a test
/// fails loudly on unexpected egress.
#[derive(Debug, Default)]
struct ScriptedVendorServer {
    responses: ScriptedResponses,
    captured: Mutex<Vec<CapturedRequest>>,
}

impl ScriptedVendorServer {
    fn script(&self, url: &str, status: u16, body: serde_json::Value) {
        self.script_raw(url, status, body.to_string().into_bytes());
    }

    fn script_raw(&self, url: &str, status: u16, body: Vec<u8>) {
        self.responses
            .lock()
            .unwrap()
            .entry(url.to_string())
            .or_default()
            .push_back((status, body));
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.captured.lock().unwrap().clone()
    }

    fn requests_for(&self, url: &str) -> Vec<CapturedRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.url == url)
            .collect()
    }

    fn request_count(&self) -> usize {
        self.captured.lock().unwrap().len()
    }
}

#[async_trait]
impl RuntimeHttpEgress for ScriptedVendorServer {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        // The engine must pin the network policy to the endpoint host.
        let url = url::Url::parse(&request.url).expect("engine URLs parse");
        let host = url.host_str().unwrap_or_default().to_string();
        assert!(
            request
                .network_policy
                .allowed_targets
                .iter()
                .any(|target| target.host_pattern == host),
            "engine egress to {host} must carry a policy pinned to that host"
        );
        assert!(
            request.response_body_limit.is_some(),
            "responses are capped"
        );
        self.captured.lock().unwrap().push(CapturedRequest {
            method: format!("{:?}", request.method).to_lowercase(),
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
        });
        // Strict FIFO: each scripted response answers exactly one call; an
        // unscripted call fails loudly with a 404 body.
        let (status, body) = self
            .responses
            .lock()
            .unwrap()
            .get_mut(&request.url)
            .and_then(std::collections::VecDeque::pop_front)
            .unwrap_or((404, b"{}".to_vec()));
        Ok(RuntimeHttpEgressResponse {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body,
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Engine harness
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct StaticClientCredentials {
    by_vendor: HashMap<String, (String, Option<String>)>,
    non_secret_by_handle: HashMap<String, String>,
}

#[async_trait]
impl EngineOAuthConfigurationSource for StaticClientCredentials {
    async fn resolve(
        &self,
        vendor: &str,
        _credentials: &RecipeClientCredentials,
    ) -> Result<EngineOAuthClientMaterial, AuthProductError> {
        let (client_id, client_secret) = self
            .by_vendor
            .get(vendor)
            .ok_or(AuthProductError::MalformedConfig)?;
        Ok(EngineOAuthClientMaterial {
            client_id: OAuthClientId::new(client_id.clone())?,
            client_secret: client_secret.clone().map(SecretString::from),
        })
    }

    async fn resolve_non_secret_value(
        &self,
        _vendor: &str,
        handle: &SecretHandle,
    ) -> Result<Option<String>, AuthProductError> {
        Ok(self.non_secret_by_handle.get(handle.as_str()).cloned())
    }
}

const CALLBACK_BASE: &str = "https://host.example/api/reborn/product-auth/oauth";

struct Harness {
    engine: AuthEngine,
    server: Arc<ScriptedVendorServer>,
    secrets: Arc<dyn SecretStore>,
}

impl Harness {
    fn new(recipes: Vec<ResolvedVendorAuthRecipe>) -> Self {
        let non_secret_by_handle = recipes
            .iter()
            .flat_map(|resolved| match &resolved.recipe {
                VendorAuthRecipe::Oauth2Code(recipe) => recipe
                    .authorize_params_from_config
                    .values()
                    .map(|handle| {
                        (
                            handle.as_str().to_string(),
                            format!("configured-{}", handle.as_str()),
                        )
                    })
                    .collect::<Vec<_>>(),
                VendorAuthRecipe::ApiKey(_) => Vec::new(),
            })
            .collect();
        Self::with_configuration(recipes, non_secret_by_handle)
    }

    fn with_configuration(
        recipes: Vec<ResolvedVendorAuthRecipe>,
        non_secret_by_handle: HashMap<String, String>,
    ) -> Self {
        let mut by_vendor = HashMap::new();
        for recipe in &recipes {
            by_vendor.insert(
                recipe.vendor.clone(),
                (
                    format!("{}-client-id", recipe.vendor),
                    Some(format!("{}-client-secret", recipe.vendor)),
                ),
            );
        }
        let server = Arc::new(ScriptedVendorServer::default());
        let secrets: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::ephemeral());
        let engine = AuthEngine::new(AuthEngineDeps {
            recipes: Arc::new(StaticAuthRecipeResolver::new(recipes)),
            configuration: Arc::new(StaticClientCredentials {
                by_vendor,
                non_secret_by_handle,
            }),
            egress: Arc::clone(&server) as Arc<dyn RuntimeHttpEgress>,
            secret_store: Arc::clone(&secrets),
            callback_base: EngineCallbackBase::new(CALLBACK_BASE).expect("callback base"),
            dcr_client_name: "IronClaw test".to_string(),
        });
        Self {
            engine,
            server,
            secrets,
        }
    }

    async fn secret_value(&self, scope: &ResourceScope, handle: &SecretHandle) -> Option<String> {
        let lease = self.secrets.lease_once(scope, handle).await.ok()?;
        self.secrets
            .consume(scope, lease.id)
            .await
            .ok()
            .map(|material| material.expose_secret().to_string())
    }
}

fn test_scope() -> AuthProductScope {
    let resource =
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .expect("local scope");
    AuthProductScope::new(resource, AuthSurface::Callback)
}

fn hex64(fill: u8) -> String {
    format!("{fill:02x}").repeat(32)
}

fn callback_request(vendor: &str, scopes: Vec<ProviderScope>) -> OAuthProviderCallbackRequest {
    OAuthProviderCallbackRequest {
        provider: AuthProviderId::new(vendor).unwrap(),
        account_label: CredentialAccountLabel::new("account").unwrap(),
        authorization_code: OAuthAuthorizationCode::new(SecretString::from(
            "vendor-auth-code".to_string(),
        ))
        .unwrap(),
        authorization_code_hash: AuthorizationCodeHash::new(hex64(0xcc)).unwrap(),
        pkce_verifier: PkceVerifierSecret::new(SecretString::from(
            "pkce-verifier-value".to_string(),
        ))
        .unwrap(),
        pkce_verifier_hash: PkceVerifierHash::new(hex64(0xbb)).unwrap(),
        scopes,
    }
}

fn exchange_context(scope: &AuthProductScope) -> OAuthProviderExchangeContext {
    OAuthProviderExchangeContext {
        scope: scope.clone(),
        flow_id: AuthFlowId::new(),
    }
}

// ---------------------------------------------------------------------------
// Recipe rows
// ---------------------------------------------------------------------------

/// Load a vendor's recipe from its real bundled manifest — the engine suite
/// runs against exactly the data production resolves (AUTH-12).
fn manifest_recipe(package: &str, vendor: &str) -> ResolvedVendorAuthRecipe {
    let path = format!(
        "{}/../ironclaw_first_party_extensions/assets/{package}/manifest.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    let value: toml::Value = toml::from_str(&text).expect("manifest parses");
    let recipe_value = value
        .get("auth")
        .and_then(|auth| auth.get(vendor))
        .unwrap_or_else(|| panic!("{package} declares [auth.{vendor}]"))
        .clone();
    let recipe: VendorAuthRecipe = recipe_value.try_into().expect("recipe deserializes");
    recipe.validate().expect("bundled recipe validates");
    let token_exchange_resource = value
        .get("mcp")
        .and_then(|mcp| mcp.get("server"))
        .and_then(|server| server.as_str())
        .map(str::to_string);
    ResolvedVendorAuthRecipe {
        vendor: vendor.to_string(),
        recipe,
        token_exchange_resource,
    }
}

/// The unified shared-vendor recipe the production resolver builds: recipes
/// for one vendor are identical except `scopes`/`display_name`, and the scope
/// ceiling is the union across every declaring manifest
/// (`ironclaw_extension_host::unified_vendor_recipes`, overview §3.2). This
/// test-local mirror unions the real bundled manifests the same way so the
/// engine suite exercises the ceiling production actually resolves.
fn unified_manifest_recipe(vendor: &str, packages: &[&str]) -> ResolvedVendorAuthRecipe {
    let mut packages = packages.iter();
    let first = packages
        .next()
        .expect("unified recipe needs at least one package");
    let mut unified = manifest_recipe(first, vendor);
    for package in packages {
        let next = manifest_recipe(package, vendor);
        let (VendorAuthRecipe::Oauth2Code(unified_recipe), VendorAuthRecipe::Oauth2Code(incoming)) =
            (&mut unified.recipe, &next.recipe)
        else {
            panic!("unified_manifest_recipe unions oauth2_code recipes only");
        };
        for scope in &incoming.scopes {
            if !unified_recipe.scopes.contains(scope) {
                unified_recipe.scopes.push(scope.clone());
            }
        }
    }
    unified
}

fn synthetic_recipe(vendor: &str, toml_text: &str) -> ResolvedVendorAuthRecipe {
    let recipe: VendorAuthRecipe = toml::from_str(toml_text).expect("synthetic recipe parses");
    ResolvedVendorAuthRecipe {
        vendor: vendor.to_string(),
        recipe,
        token_exchange_resource: None,
    }
}

fn acme_recipe_toml(extra: &str) -> String {
    format!(
        r#"
method = "oauth2_code"
display_name = "Acme account"
authorization_endpoint = "https://auth.acme.example/authorize"
token_endpoint = "https://auth.acme.example/token"
scopes = ["msg:read", "msg:write"]
client_credentials = {{ client_id_handle = "acme_oauth_client_id", client_secret_handle = "acme_oauth_client_secret" }}
{extra}

[token_response]
access_token = "/access_token"
refresh_token = "/refresh_token"
expires_in = "/expires_in"
"#
    )
}

fn all_manifest_rows() -> Vec<ResolvedVendorAuthRecipe> {
    vec![
        manifest_recipe("slack", "slack"),
        manifest_recipe("gmail", "google"),
        manifest_recipe("notion-mcp", "notion"),
        manifest_recipe("github", "github"),
        manifest_recipe("nearai-mcp", "nearai"),
    ]
}

// ---------------------------------------------------------------------------
// AUTH-12: every bundled vendor is expressible as a recipe row
// ---------------------------------------------------------------------------

#[test]
fn all_five_vendors_load_as_recipe_rows_from_their_manifests() {
    let rows = all_manifest_rows();
    assert_eq!(rows.len(), 5);
    for row in &rows {
        row.recipe.validate().expect("recipe row validates");
    }
    // Method split is data, not code: three oauth2_code, two api_key.
    let oauth2 = rows
        .iter()
        .filter(|row| matches!(row.recipe, VendorAuthRecipe::Oauth2Code(_)))
        .count();
    assert_eq!(oauth2, 3);
}

// ---------------------------------------------------------------------------
// AUTH-2 / AUTH-4: authorize-URL construction, table-driven
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authorize_url_is_host_constructed_for_every_oauth_vendor_row() {
    struct Row {
        recipe: ResolvedVendorAuthRecipe,
        requested: Vec<&'static str>,
        scope_param: &'static str,
        expected_scope_text: &'static str,
    }
    let rows = vec![
        Row {
            recipe: manifest_recipe("slack", "slack"),
            requested: vec!["search:read", "chat:write"],
            scope_param: "user_scope",
            expected_scope_text: "search:read chat:write",
        },
        Row {
            recipe: manifest_recipe("gmail", "google"),
            requested: vec!["https://www.googleapis.com/auth/gmail.readonly"],
            scope_param: "scope",
            expected_scope_text: "https://www.googleapis.com/auth/gmail.readonly",
        },
    ];

    for row in rows {
        let vendor = row.recipe.vendor.clone();
        let harness = Harness::new(vec![row.recipe]);
        let scope = test_scope();
        let prepared = harness
            .engine
            .prepare_oauth_flow(PrepareOAuthFlowRequest {
                vendor: vendor.clone(),
                scope: scope.clone(),
                flow_id: AuthFlowId::new(),
                account_label: CredentialAccountLabel::new("account").unwrap(),
                requested_scopes: row
                    .requested
                    .iter()
                    .map(|scope| ProviderScope::new(scope.to_string()).unwrap())
                    .collect(),
            })
            .await
            .unwrap_or_else(|error| panic!("{vendor} prepare: {error}"));

        // Flow preparation never touches the vendor for statically
        // credentialed recipes.
        assert_eq!(harness.server.request_count(), 0, "{vendor}");

        let url = url::Url::parse(prepared.authorization_url.as_str()).unwrap();
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let count = |name: &str| pairs.iter().filter(|(key, _)| key == name).count();
        let value = |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        // Host-owned reserved parameters, present exactly once.
        for reserved in ["client_id", "redirect_uri", "response_type", "state"] {
            assert_eq!(count(reserved), 1, "{vendor} {reserved}");
        }
        assert_eq!(
            value("client_id").as_deref(),
            Some(format!("{vendor}-client-id").as_str())
        );
        assert_eq!(
            value("redirect_uri").as_deref(),
            Some(format!("{CALLBACK_BASE}/{vendor}/callback").as_str()),
            "the static per-vendor callback path is host-built (AUTH-13)"
        );
        assert_eq!(value("response_type").as_deref(), Some("code"));
        assert_eq!(count("code_challenge"), 1, "{vendor} pkce");
        assert_eq!(value("code_challenge_method").as_deref(), Some("S256"));
        // The scope parameter name and joiner come from the recipe.
        assert_eq!(
            value(row.scope_param).as_deref(),
            Some(row.expected_scope_text),
            "{vendor} scope param"
        );
        if row.scope_param != "scope" {
            assert_eq!(count("scope"), 0, "{vendor} must not emit a bare scope=");
        }
        assert!(
            prepared.authorization_url.as_str().starts_with("https://"),
            "{vendor}"
        );
    }
}

#[tokio::test]
async fn google_extra_authorize_params_come_from_recipe_data() {
    let harness = Harness::new(vec![manifest_recipe("gmail", "google")]);
    let prepared = harness
        .engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "google".to_string(),
            scope: test_scope(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: Vec::new(),
        })
        .await
        .unwrap();
    let url = url::Url::parse(prepared.authorization_url.as_str()).unwrap();
    let pairs: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs.get("access_type").map(String::as_str),
        Some("offline")
    );
    assert_eq!(pairs.get("prompt").map(String::as_str), Some("consent"));
    assert_eq!(
        pairs.get("include_granted_scopes").map(String::as_str),
        Some("true")
    );
    // Empty request defaults to the recipe's full scope ceiling.
    assert!(
        pairs
            .get("scope")
            .is_some_and(|scopes| scopes.contains("gmail.readonly"))
    );
}

#[tokio::test]
async fn configured_authorize_params_are_resolved_by_handle() {
    let row = synthetic_recipe(
        "acme",
        &acme_recipe_toml("authorize_params_from_config = { workspace = \"acme_workspace_id\" }"),
    );
    let harness = Harness::with_configuration(
        vec![row],
        HashMap::from([("acme_workspace_id".to_string(), "ACME-TEAM".to_string())]),
    );

    let prepared = harness
        .engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "acme".to_string(),
            scope: test_scope(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: Vec::new(),
        })
        .await
        .expect("declared non-secret configuration resolves");

    let url = url::Url::parse(prepared.authorization_url.as_str()).unwrap();
    assert_eq!(
        url.query_pairs()
            .find(|(name, _)| name == "workspace")
            .map(|(_, value)| value.into_owned()),
        Some("ACME-TEAM".to_string())
    );
}

#[tokio::test]
async fn missing_configured_authorize_param_fails_before_provider_contact() {
    let row = synthetic_recipe(
        "acme",
        &acme_recipe_toml("authorize_params_from_config = { workspace = \"acme_workspace_id\" }"),
    );
    let harness = Harness::with_configuration(vec![row], HashMap::new());

    let error = harness
        .engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "acme".to_string(),
            scope: test_scope(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: Vec::new(),
        })
        .await
        .expect_err("missing authorize configuration must fail closed");

    assert_eq!(error.code(), AuthErrorCode::MalformedConfig);
    assert_eq!(harness.server.request_count(), 0);
}

#[tokio::test]
async fn recipes_cannot_supply_or_override_reserved_authorize_params() {
    // The resolver hands back a recipe that names reserved params — as if P1
    // manifest validation had been bypassed. The engine must reject it.
    for reserved in [
        "state",
        "redirect_uri",
        "code_challenge",
        "client_id",
        "response_type",
        "scope",
    ] {
        let row = synthetic_recipe(
            "acme",
            &acme_recipe_toml(&format!(
                "extra_authorize_params = {{ {reserved} = \"evil\" }}"
            )),
        );
        let harness = Harness::new(vec![row]);
        let error = harness
            .engine
            .prepare_oauth_flow(PrepareOAuthFlowRequest {
                vendor: "acme".to_string(),
                scope: test_scope(),
                flow_id: AuthFlowId::new(),
                account_label: CredentialAccountLabel::new("account").unwrap(),
                requested_scopes: Vec::new(),
            })
            .await
            .expect_err("reserved param must be rejected");
        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::MalformedConfig,
            "{reserved}"
        );
        assert_eq!(harness.server.request_count(), 0);
    }
}

#[tokio::test]
async fn authorization_endpoint_predefining_reserved_params_is_rejected() {
    let row = synthetic_recipe(
        "acme",
        &acme_recipe_toml("").replace(
            "https://auth.acme.example/authorize",
            "https://auth.acme.example/authorize?state=evil",
        ),
    );
    let harness = Harness::new(vec![row]);
    let error = harness
        .engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "acme".to_string(),
            scope: test_scope(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: Vec::new(),
        })
        .await
        .expect_err("endpoint predefining state must be rejected");
    assert_eq!(error.code(), ironclaw_auth::AuthErrorCode::MalformedConfig);
}

#[tokio::test]
async fn scope_widening_is_rejected_before_any_vendor_call() {
    let harness = Harness::new(vec![manifest_recipe("slack", "slack")]);
    let scope = test_scope();
    // Prepare-time widening.
    let error = harness
        .engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "slack".to_string(),
            scope: scope.clone(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: vec![ProviderScope::new("admin").unwrap()],
        })
        .await
        .expect_err("scope outside the ceiling must be rejected");
    assert_eq!(error.code(), ironclaw_auth::AuthErrorCode::InvalidRequest);

    // Exchange-time widening (defense in depth): rejected before egress.
    let error = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request("slack", vec![ProviderScope::new("admin").unwrap()]),
        )
        .await
        .expect_err("exchange with widened scopes must be rejected");
    assert_eq!(error.code(), ironclaw_auth::AuthErrorCode::InvalidRequest);
    assert_eq!(
        harness.server.request_count(),
        0,
        "widening must be rejected before the vendor call"
    );
}

// ---------------------------------------------------------------------------
// AUTH-5: token exchange — post_body/basic + pointer extraction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_exchange_supports_post_body_and_basic_client_auth() {
    for (auth_style, expect_basic) in [("post_body", false), ("basic", true)] {
        let row = synthetic_recipe(
            "acme",
            &acme_recipe_toml(&format!("exchange_auth = \"{auth_style}\"")),
        );
        let harness = Harness::new(vec![row]);
        let scope = test_scope();
        harness.server.script(
            "https://auth.acme.example/token",
            200,
            serde_json::json!({
                "access_token": "acme-access",
                "refresh_token": "acme-refresh",
                "expires_in": 3600
            }),
        );
        harness
            .engine
            .exchange_callback(
                exchange_context(&scope),
                callback_request("acme", vec![ProviderScope::new("msg:read").unwrap()]),
            )
            .await
            .unwrap_or_else(|error| panic!("{auth_style} exchange: {error}"));

        let requests = harness
            .server
            .requests_for("https://auth.acme.example/token");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        let form = request.form();
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(
            form.get("code").map(String::as_str),
            Some("vendor-auth-code")
        );
        assert_eq!(
            form.get("code_verifier").map(String::as_str),
            Some("pkce-verifier-value")
        );
        assert_eq!(
            form.get("redirect_uri").map(String::as_str),
            Some(format!("{CALLBACK_BASE}/acme/callback").as_str())
        );
        if expect_basic {
            let authorization = request.header("authorization").expect("basic header");
            assert!(authorization.starts_with("Basic "), "{authorization}");
            assert!(!form.contains_key("client_secret"));
            assert!(!form.contains_key("client_id"));
        } else {
            assert_eq!(
                form.get("client_id").map(String::as_str),
                Some("acme-client-id")
            );
            assert_eq!(
                form.get("client_secret").map(String::as_str),
                Some("acme-client-secret")
            );
            assert!(request.header("authorization").is_none());
        }
    }
}

#[tokio::test]
async fn pointer_extraction_reads_nested_fields_and_scope_fallback() {
    // Slack row: nested `authed_user` pointers + fallback_to_requested scope.
    let harness = Harness::new(vec![manifest_recipe("slack", "slack")]);
    let scope = test_scope();
    harness.server.script(
        "https://slack.com/api/oauth.v2.access",
        200,
        serde_json::json!({
            "ok": true,
            "app_id": "A100",
            "team": { "id": "T100" },
            "authed_user": {
                "id": "U100",
                "access_token": "xoxp-nested-token",
                // Comma-separated, as Slack returns granted user scopes.
                "scope": "search:read,chat:write"
            }
        }),
    );
    let context = exchange_context(&scope);
    let flow_id = context.flow_id;
    let exchange = harness
        .engine
        .exchange_callback(
            context,
            // Request both granted scopes so the A6 clamp (granted ∩ recipe
            // ceiling) is a no-op here and this case keeps proving
            // comma-separated multi-scope pointer extraction. The over-claim
            // clamp itself is covered by
            // `exchange_clamps_echoed_scopes_to_recipe_ceiling`.
            callback_request(
                "slack",
                vec![
                    ProviderScope::new("search:read").unwrap(),
                    ProviderScope::new("chat:write").unwrap(),
                ],
            ),
        )
        .await
        .expect("slack exchange");
    assert_eq!(
        exchange.scopes,
        vec![
            ProviderScope::new("search:read").unwrap(),
            ProviderScope::new("chat:write").unwrap(),
        ],
        "granted scopes come from the pointer path, commas normalized"
    );
    // Identity from the token response (AUTH-7).
    let identity = exchange.provider_identity.expect("identity");
    assert_eq!(identity.subject.as_str(), "U100");
    assert_eq!(identity.team_id.as_deref(), Some("T100"));
    assert_eq!(identity.app_id.as_deref(), Some("A100"));
    // The stored access secret holds the NESTED token.
    let stored = harness
        .secret_value(&scope.resource, &exchange.access_secret)
        .await
        .expect("stored access token");
    assert_eq!(stored, "xoxp-nested-token");
    assert!(
        exchange.refresh_secret.is_none(),
        "slack returned no refresh token"
    );
    let _ = flow_id;

    // Fallback: no granted scopes in the response → requested scopes stored.
    harness.server.script(
        "https://slack.com/api/oauth.v2.access",
        200,
        serde_json::json!({
            "ok": true,
            "app_id": "A100",
            "team": { "id": "T100" },
            "authed_user": { "id": "U100", "access_token": "xoxp-2" }
        }),
    );
    let exchange = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request("slack", vec![ProviderScope::new("search:read").unwrap()]),
        )
        .await
        .expect("slack exchange without echoed scopes");
    assert_eq!(
        exchange.scopes,
        vec![ProviderScope::new("search:read").unwrap()],
        "missing scope falls back to requested (fallback_to_requested)"
    );
}

/// A6 · Scope downgrade / over-claim on the echoed-scope path (RFC 9700 §2.3).
/// The vendor echoes a scope set that both over-grants a scope no recipe ever
/// declared (`admin.conversations:write`, outside the ceiling) and omits one
/// that was requested (`channels:read`). The stored grant must be exactly
/// `granted ∩ recipe ceiling` — the outside-ceiling scope is dropped (no
/// over-claim) and the grant is never widened to the requested set. Generic
/// clamp, no vendor branch.
#[tokio::test]
async fn exchange_clamps_echoed_scopes_to_recipe_ceiling() {
    let harness = Harness::new(vec![manifest_recipe("slack", "slack")]);
    let scope = test_scope();
    harness.server.script(
        "https://slack.com/api/oauth.v2.access",
        200,
        serde_json::json!({
            "ok": true,
            "app_id": "A100",
            "team": { "id": "T100" },
            "authed_user": {
                "id": "U100",
                "access_token": "xoxp-clamp-token",
                // Grants a scope outside the recipe ceiling (dropped) and
                // omits channels:read (requested — never widened back in).
                "scope": "search:read,admin.conversations:write"
            }
        }),
    );
    let exchange = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request(
                "slack",
                vec![
                    ProviderScope::new("search:read").unwrap(),
                    ProviderScope::new("channels:read").unwrap(),
                ],
            ),
        )
        .await
        .expect("slack exchange");
    assert_eq!(
        exchange.scopes,
        vec![ProviderScope::new("search:read").unwrap()],
        "stored grant is granted ∩ ceiling: the undeclared scope is dropped, \
         channels:read never widened in"
    );
}

/// The shared-vendor cumulative-grant regression (gmail → google-docs
/// sign-out): several extensions share one vendor account, each connect
/// requests only its own extension's scopes, and a cumulative-grant vendor
/// (recipe data: Google's `include_granted_scopes` authorize param) echoes
/// previously granted scopes on every exchange. The clamp must keep every
/// echoed scope inside the UNIFIED recipe ceiling — clamping to this flow's
/// request would strip the first extension's scopes from the shared account
/// and sign it out.
#[tokio::test]
async fn exchange_preserves_cumulative_grant_within_unified_ceiling() {
    // The production resolver unions scopes across every manifest declaring
    // the vendor (`unified_vendor_recipes`); mirror that union over the real
    // gmail + google-docs manifests.
    let harness = Harness::new(vec![unified_manifest_recipe(
        "google",
        &["gmail", "google-docs"],
    )]);
    let scope = test_scope();
    let docs_scopes = vec![
        ProviderScope::new("https://www.googleapis.com/auth/documents").unwrap(),
        ProviderScope::new("https://www.googleapis.com/auth/documents.readonly").unwrap(),
    ];
    // The docs connect happens second: gmail's scopes were granted earlier,
    // so Google echoes the cumulative grant alongside the newly consented
    // docs scopes (space-separated, as Google returns them).
    let cumulative_grant = [
        "https://www.googleapis.com/auth/documents",
        "https://www.googleapis.com/auth/documents.readonly",
        "https://www.googleapis.com/auth/gmail.readonly",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/gmail.modify",
    ]
    .join(" ");
    harness.server.script(
        "https://oauth2.googleapis.com/token",
        200,
        serde_json::json!({
            "access_token": "ya29-cumulative",
            "refresh_token": "1//refresh-cumulative",
            "expires_in": 3599,
            "scope": cumulative_grant
        }),
    );
    let exchange = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request("google", docs_scopes),
        )
        .await
        .expect("google exchange");
    let stored: Vec<&str> = exchange.scopes.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        stored,
        vec![
            "https://www.googleapis.com/auth/documents",
            "https://www.googleapis.com/auth/documents.readonly",
            "https://www.googleapis.com/auth/gmail.readonly",
            "https://www.googleapis.com/auth/gmail.send",
            "https://www.googleapis.com/auth/gmail.modify",
        ],
        "the cumulative grant within the unified ceiling is preserved — \
         gmail's scopes survive the google-docs connect"
    );
}

#[tokio::test]
async fn missing_scope_without_fallback_fails_the_exchange() {
    // Google's recipe extracts `/scope` with the default `reject` behavior.
    let harness = Harness::new(vec![manifest_recipe("gmail", "google")]);
    let scope = test_scope();
    harness.server.script(
        "https://oauth2.googleapis.com/token",
        200,
        serde_json::json!({ "access_token": "ya29-token", "expires_in": 3599 }),
    );
    let error = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request(
                "google",
                vec![ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap()],
            ),
        )
        .await
        .expect_err("missing scope with reject behavior fails");
    assert_eq!(
        error.code(),
        ironclaw_auth::AuthErrorCode::TokenExchangeFailed
    );
}

#[tokio::test]
async fn vendor_error_responses_are_size_capped_and_never_echoed() {
    let harness = Harness::new(vec![synthetic_recipe("acme", &acme_recipe_toml(""))]);
    let scope = test_scope();
    // A huge, secret-bearing vendor error body.
    let marker = "SUPER-SECRET-VENDOR-BODY";
    harness.server.script_raw(
        "https://auth.acme.example/token",
        400,
        format!(
            "{{\"error\":\"invalid_request\",\"detail\":\"{}\"}}",
            marker.repeat(4096)
        )
        .into_bytes(),
    );
    let error = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request("acme", vec![ProviderScope::new("msg:read").unwrap()]),
        )
        .await
        .expect_err("vendor 4xx fails the exchange");
    assert_eq!(
        error.code(),
        ironclaw_auth::AuthErrorCode::TokenExchangeFailed
    );
    assert!(
        !format!("{error:?}").contains(marker),
        "vendor bodies must never surface in errors"
    );
}

// ---------------------------------------------------------------------------
// AUTH-7: identity via declared endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn identity_extracts_from_declared_endpoint_with_fresh_credential() {
    let row = synthetic_recipe(
        "acme",
        &format!(
            "{}\n[identity]\nendpoint = {{ url = \"https://api.acme.example/whoami\" }}\naccount_id = \"/user/id\"\nteam_id = \"/org/id\"\n",
            acme_recipe_toml("")
        ),
    );
    let harness = Harness::new(vec![row]);
    let scope = test_scope();
    harness.server.script(
        "https://auth.acme.example/token",
        200,
        serde_json::json!({ "access_token": "acme-access-token", "expires_in": 3600 }),
    );
    harness.server.script(
        "https://api.acme.example/whoami",
        200,
        serde_json::json!({ "user": { "id": "acme-user-9" }, "org": { "id": "acme-org-1" } }),
    );
    let exchange = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request("acme", vec![ProviderScope::new("msg:read").unwrap()]),
        )
        .await
        .expect("exchange with identity endpoint");
    let identity = exchange.provider_identity.expect("identity");
    assert_eq!(identity.subject.as_str(), "acme-user-9");
    assert_eq!(identity.team_id.as_deref(), Some("acme-org-1"));
    // The endpoint was called with the freshly issued credential.
    let whoami = harness
        .server
        .requests_for("https://api.acme.example/whoami");
    assert_eq!(whoami.len(), 1);
    assert_eq!(whoami[0].method, "get");
    assert_eq!(
        whoami[0].header("authorization"),
        Some("Bearer acme-access-token")
    );

    // A rejected identity fetch fails the whole exchange: no grant without a
    // validated identity (AUTH-7).
    harness.server.script(
        "https://auth.acme.example/token",
        200,
        serde_json::json!({ "access_token": "acme-access-2", "expires_in": 3600 }),
    );
    harness.server.script(
        "https://api.acme.example/whoami",
        401,
        serde_json::json!({}),
    );
    let error = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request("acme", vec![ProviderScope::new("msg:read").unwrap()]),
        )
        .await
        .expect_err("identity rejection fails the exchange");
    assert_eq!(
        error.code(),
        ironclaw_auth::AuthErrorCode::TokenExchangeFailed
    );
}

// ---------------------------------------------------------------------------
// AUTH-6: refresh rotation both ways, invalid_grant, revoke idempotency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn refresh_honors_rotates_refresh_token_both_ways() {
    let scope = test_scope();
    let account_id = CredentialAccountId::new();

    // Rotating vendor: the replacement refresh token must be persisted.
    let rotating = synthetic_recipe(
        "acme",
        &format!(
            "{}\n[refresh]\nrotates_refresh_token = true\n",
            acme_recipe_toml("")
        ),
    );
    let harness = Harness::new(vec![rotating]);
    let old_handle = SecretHandle::new("acme-seeded-refresh").unwrap();
    harness
        .secrets
        .put(
            scope.resource.clone(),
            old_handle.clone(),
            SecretString::from("old-refresh-token".to_string()),
            None,
        )
        .await
        .unwrap();
    harness.server.script(
        "https://auth.acme.example/token",
        200,
        serde_json::json!({
            "access_token": "rotated-access",
            "refresh_token": "rotated-refresh",
            "expires_in": 3600
        }),
    );
    let refresh = harness
        .engine
        .refresh_token(OAuthProviderRefreshRequest {
            provider: AuthProviderId::new("acme").unwrap(),
            scope: scope.clone(),
            account_id,
            refresh_secret: old_handle.clone(),
            scopes: vec![ProviderScope::new("msg:read").unwrap()],
        })
        .await
        .expect("rotating refresh");
    let new_handle = refresh.refresh_secret.expect("rotated refresh handle");
    assert_ne!(
        new_handle, old_handle,
        "rotation persists a NEW refresh handle"
    );
    assert_eq!(
        harness
            .secret_value(&scope.resource, &new_handle)
            .await
            .as_deref(),
        Some("rotated-refresh")
    );
    let form = harness
        .server
        .requests_for("https://auth.acme.example/token")[0]
        .form();
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("refresh_token")
    );
    assert_eq!(
        form.get("refresh_token").map(String::as_str),
        Some("old-refresh-token")
    );

    // Non-rotating vendor: a response without a refresh token preserves the
    // existing stored handle — the still-valid refresh token is never orphaned.
    let non_rotating = synthetic_recipe(
        "acme",
        &format!(
            "{}\n[refresh]\nrotates_refresh_token = false\n",
            acme_recipe_toml("")
        ),
    );
    let harness = Harness::new(vec![non_rotating]);
    let old_handle = SecretHandle::new("acme-seeded-refresh").unwrap();
    harness
        .secrets
        .put(
            scope.resource.clone(),
            old_handle.clone(),
            SecretString::from("stable-refresh-token".to_string()),
            None,
        )
        .await
        .unwrap();
    harness.server.script(
        "https://auth.acme.example/token",
        200,
        serde_json::json!({ "access_token": "fresh-access", "expires_in": 3600 }),
    );
    let refresh = harness
        .engine
        .refresh_token(OAuthProviderRefreshRequest {
            provider: AuthProviderId::new("acme").unwrap(),
            scope: scope.clone(),
            account_id,
            refresh_secret: old_handle.clone(),
            scopes: vec![ProviderScope::new("msg:read").unwrap()],
        })
        .await
        .expect("non-rotating refresh");
    assert_eq!(
        refresh.refresh_secret,
        Some(old_handle.clone()),
        "non-rotating vendors keep the existing refresh handle"
    );
    assert_eq!(
        harness
            .secret_value(&scope.resource, &refresh.access_secret)
            .await
            .as_deref(),
        Some("fresh-access")
    );
    assert_eq!(
        refresh.scopes,
        vec![ProviderScope::new("msg:read").unwrap()],
        "scopes fall back to the existing grant when the vendor echoes none"
    );
}

#[tokio::test]
async fn refresh_invalid_grant_is_a_typed_permanent_failure() {
    let harness = Harness::new(vec![synthetic_recipe("acme", &acme_recipe_toml(""))]);
    let scope = test_scope();
    let handle = SecretHandle::new("acme-refresh").unwrap();
    harness
        .secrets
        .put(
            scope.resource.clone(),
            handle.clone(),
            SecretString::from("revoked-refresh".to_string()),
            None,
        )
        .await
        .unwrap();
    harness.server.script(
        "https://auth.acme.example/token",
        400,
        serde_json::json!({ "error": "invalid_grant" }),
    );
    let error = harness
        .engine
        .refresh_token(OAuthProviderRefreshRequest {
            provider: AuthProviderId::new("acme").unwrap(),
            scope: scope.clone(),
            account_id: CredentialAccountId::new(),
            refresh_secret: handle,
            scopes: Vec::new(),
        })
        .await
        .expect_err("invalid_grant is a refresh failure");
    assert!(
        matches!(error, AuthProductError::InvalidGrant),
        "invalid_grant maps to the typed permanent-revocation error, got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// DCR (notion row): register once, reuse, standard oauth2_code after
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dcr_vendor_registers_once_and_runs_standard_oauth_afterwards() {
    let harness = Harness::new(vec![manifest_recipe("notion-mcp", "notion")]);
    let scope = test_scope();
    harness.server.script(
        "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource",
        200,
        serde_json::json!({ "authorization_servers": ["https://mcp.notion.com"] }),
    );
    harness.server.script(
        "https://mcp.notion.com/.well-known/oauth-authorization-server",
        200,
        serde_json::json!({
            "authorization_endpoint": "https://mcp.notion.com/discovered/authorize",
            "token_endpoint": "https://mcp.notion.com/discovered/token",
            "registration_endpoint": "https://mcp.notion.com/register"
        }),
    );
    harness.server.script(
        "https://mcp.notion.com/register",
        201,
        serde_json::json!({ "client_id": "notion-dcr-client-1" }),
    );

    let prepare = |flow_id| {
        harness.engine.prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "notion".to_string(),
            scope: scope.clone(),
            flow_id,
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: Vec::new(),
        })
    };
    let prepared = prepare(AuthFlowId::new()).await.expect("first notion flow");
    let url = url::Url::parse(prepared.authorization_url.as_str()).unwrap();
    assert!(
        url.as_str()
            .starts_with("https://mcp.notion.com/discovered/authorize"),
        "authorize URL uses the DISCOVERED endpoint, not the manifest placeholder"
    );
    let pairs: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs.get("client_id").map(String::as_str),
        Some("notion-dcr-client-1"),
        "the registered client id is used"
    );
    let registration = &harness
        .server
        .requests_for("https://mcp.notion.com/register")[0];
    let registered: serde_json::Value = serde_json::from_slice(&registration.body).unwrap();
    assert_eq!(
        registered["redirect_uris"][0],
        format!("{CALLBACK_BASE}/notion/callback"),
        "registration pins the static vendor callback (AUTH-13)"
    );

    // Second flow: the persisted registered client is reused — no second
    // registration, no second discovery round-trip.
    let discovery_calls_before = harness.server.request_count();
    prepare(AuthFlowId::new())
        .await
        .expect("second notion flow");
    assert_eq!(
        harness.server.request_count(),
        discovery_calls_before,
        "second flow reuses the persisted registered client"
    );

    // The token exchange then runs the standard oauth2_code flow against the
    // discovered token endpoint, carrying the RFC 8707 resource indicator.
    // Notion rotates single-use refresh tokens on ~1h access tokens; the
    // recipe must capture both or the credential silently dies at expiry
    // (the A4 regression).
    harness.server.script(
        "https://mcp.notion.com/discovered/token",
        200,
        serde_json::json!({
            "access_token": "notion-access",
            "refresh_token": "notion-refresh-1",
            "expires_in": 3600
        }),
    );
    let exchange = harness
        .engine
        .exchange_callback(
            exchange_context(&scope),
            callback_request("notion", Vec::new()),
        )
        .await
        .expect("notion exchange");
    assert!(exchange.provider_identity.is_none());
    let refresh_secret = exchange
        .refresh_secret
        .as_ref()
        .expect("notion recipe captures the rotating refresh token (A4)");
    assert_eq!(
        harness.secret_value(&scope.resource, refresh_secret).await,
        Some("notion-refresh-1".to_string()),
        "the captured refresh token is stored for the refresh path"
    );
    let token_request = &harness
        .server
        .requests_for("https://mcp.notion.com/discovered/token")[0];
    let form = token_request.form();
    assert_eq!(
        form.get("resource").map(String::as_str),
        Some("https://mcp.notion.com/mcp"),
        "the [mcp].server resource indicator rides the token request as data"
    );
    assert_eq!(
        form.get("client_id").map(String::as_str),
        Some("notion-dcr-client-1")
    );
}

/// A compromised protected-resource metadata document must not be able to
/// redirect dynamic client registration to an attacker host that shares only
/// a multi-part public suffix with the resource. The pre-PSL validation took
/// the final two host labels, so `mcp.example.co.uk` and `attacker.co.uk`
/// both resolved to "co.uk" and the attacker issuer passed (the PR #6116
/// security-high finding). The discriminating assertion is the request count:
/// the engine must stop after the protected-resource metadata fetch and never
/// contact the attacker authorization server.
#[tokio::test]
async fn dcr_issuer_sharing_only_a_public_suffix_is_rejected_before_any_attacker_contact() {
    let mut recipe = manifest_recipe("notion-mcp", "notion");
    recipe.token_exchange_resource = Some("https://mcp.example.co.uk/mcp".to_string());
    let harness = Harness::new(vec![recipe]);
    harness.server.script(
        "https://mcp.example.co.uk/mcp/.well-known/oauth-protected-resource",
        200,
        serde_json::json!({ "authorization_servers": ["https://attacker.co.uk"] }),
    );

    let result = harness
        .engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "notion".to_string(),
            scope: test_scope(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: Vec::new(),
        })
        .await;

    assert!(
        result.is_err(),
        "an issuer on a foreign registrable domain under a shared public suffix must fail the flow"
    );
    assert_eq!(
        harness.server.request_count(),
        1,
        "the engine stops at the protected-resource metadata fetch; no request reaches the attacker host"
    );
    assert!(
        harness
            .server
            .requests_for("https://attacker.co.uk/.well-known/oauth-authorization-server")
            .is_empty(),
        "the attacker authorization-server metadata is never fetched"
    );
}

/// The positive companion: an issuer under the SAME registrable domain as the
/// resource — including on a multi-part public suffix — keeps working end to
/// end (discovery, registration, authorize URL), so the fix cannot overblock
/// legitimate vendors whose auth host is a sibling of the MCP host.
#[tokio::test]
async fn dcr_issuer_on_same_registrable_domain_under_multi_part_suffix_registers() {
    let mut recipe = manifest_recipe("notion-mcp", "notion");
    recipe.token_exchange_resource = Some("https://mcp.example.co.uk/mcp".to_string());
    let harness = Harness::new(vec![recipe]);
    harness.server.script(
        "https://mcp.example.co.uk/mcp/.well-known/oauth-protected-resource",
        200,
        serde_json::json!({ "authorization_servers": ["https://auth.example.co.uk"] }),
    );
    harness.server.script(
        "https://auth.example.co.uk/.well-known/oauth-authorization-server",
        200,
        serde_json::json!({
            "authorization_endpoint": "https://auth.example.co.uk/authorize",
            "token_endpoint": "https://auth.example.co.uk/token",
            "registration_endpoint": "https://auth.example.co.uk/register"
        }),
    );
    harness.server.script(
        "https://auth.example.co.uk/register",
        201,
        serde_json::json!({ "client_id": "couk-dcr-client-1" }),
    );

    let prepared = harness
        .engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "notion".to_string(),
            scope: test_scope(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            requested_scopes: Vec::new(),
        })
        .await
        .expect("sibling issuer under the same registrable domain is accepted");
    assert!(
        prepared
            .authorization_url
            .as_str()
            .starts_with("https://auth.example.co.uk/authorize"),
        "the discovered sibling authorize endpoint is used"
    );
}

/// A4 pin on the REAL bundled manifest: Notion issues ~1h access tokens with
/// single-use rotating refresh tokens, so its recipe must declare the
/// `refresh_token`/`expires_in` capture pointers and the rotation flag. The
/// pointer-driven engine captures nothing that is not declared — without
/// these the token is stored non-expiring and every Notion connection dies
/// at the first expiry with no recovery.
#[test]
fn notion_recipe_declares_refresh_and_expiry_capture() {
    let resolved = manifest_recipe("notion-mcp", "notion");
    let VendorAuthRecipe::Oauth2Code(recipe) = &resolved.recipe else {
        panic!("notion declares oauth2_code");
    };
    assert!(
        recipe.token_response.refresh_token.is_some(),
        "notion token_response must capture /refresh_token"
    );
    assert!(
        recipe.token_response.expires_in.is_some(),
        "notion token_response must capture /expires_in (else stored non-expiring)"
    );
    assert!(
        recipe
            .refresh
            .as_ref()
            .is_some_and(|refresh| refresh.rotates_refresh_token),
        "notion refresh tokens rotate single-use; the recipe must declare it"
    );
}

// ---------------------------------------------------------------------------
// AUTH-3: flow state machine — exactly-once callback, cross-flow rejection
// ---------------------------------------------------------------------------

fn new_flow(
    scope: &AuthProductScope,
    state_hash: OpaqueStateHash,
    pkce_hash: PkceVerifierHash,
) -> NewAuthFlow {
    let expires_at = Utc::now() + Duration::minutes(5);
    NewAuthFlow {
        id: None,
        scope: scope.clone(),
        kind: AuthFlowKind::IntegrationCredential,
        provider: AuthProviderId::new("acme").unwrap(),
        challenge: AuthChallenge::OAuthUrl {
            authorization_url: OAuthAuthorizationUrl::new("https://auth.acme.example/authorize")
                .unwrap(),
            expires_at,
        },
        continuation: AuthContinuationRef::SetupOnly,
        update_binding: None,
        opaque_state_hash: Some(state_hash),
        pkce_verifier_hash: Some(pkce_hash),
        expires_at,
    }
}

fn authorized_outcome(exchange_provider: &str) -> ProviderCallbackOutcome {
    ProviderCallbackOutcome::Authorized {
        exchange: Box::new(OAuthProviderExchange {
            provider: AuthProviderId::new(exchange_provider).unwrap(),
            account_label: CredentialAccountLabel::new("account").unwrap(),
            authorization_code_hash: AuthorizationCodeHash::new(hex64(0xcc)).unwrap(),
            pkce_verifier_hash: PkceVerifierHash::new(hex64(0xbb)).unwrap(),
            access_secret: SecretHandle::new("acme-access").unwrap(),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("msg:read").unwrap()],
            account_id: None,
            provider_identity: None,
        }),
    }
}

#[tokio::test]
async fn exactly_one_transition_consumes_a_callback() {
    let services = InMemoryAuthProductServices::new();
    let scope = test_scope();
    let state_hash = OpaqueStateHash::new(hex64(0xaa)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xbb)).unwrap();
    let flow = services
        .create_flow(new_flow(&scope, state_hash.clone(), pkce_hash.clone()))
        .await
        .unwrap();

    services
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash.clone(),
                provider: AuthProviderId::new("acme").unwrap(),
                pkce_verifier_hash: pkce_hash.clone(),
            },
        )
        .await
        .expect("first claim");
    services
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash.clone(),
                outcome: authorized_outcome("acme"),
            },
        )
        .await
        .expect("first completion");

    // The callback is consumed: a second completion must fail.
    let error = services
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash,
                outcome: authorized_outcome("acme"),
            },
        )
        .await
        .expect_err("a callback is consumed by exactly one transition");
    assert_eq!(
        error.code(),
        ironclaw_auth::AuthErrorCode::FlowAlreadyTerminal
    );
}

#[tokio::test]
async fn cross_flow_callbacks_are_rejected() {
    let services = InMemoryAuthProductServices::new();
    let scope = test_scope();
    let state_a = OpaqueStateHash::new(hex64(0x0a)).unwrap();
    let state_b = OpaqueStateHash::new(hex64(0x0b)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xbb)).unwrap();
    let flow_a = services
        .create_flow(new_flow(&scope, state_a.clone(), pkce_hash.clone()))
        .await
        .unwrap();
    let _flow_b = services
        .create_flow(new_flow(&scope, state_b.clone(), pkce_hash.clone()))
        .await
        .unwrap();

    // Flow B's state presented against flow A: rejected before completion.
    let error = services
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow_a.id,
                opaque_state_hash: state_b,
                provider: AuthProviderId::new("acme").unwrap(),
                pkce_verifier_hash: pkce_hash,
            },
        )
        .await
        .expect_err("cross-flow state must not claim another flow");
    assert_ne!(
        error.code(),
        ironclaw_auth::AuthErrorCode::BackendUnavailable
    );
}

// ---------------------------------------------------------------------------
// Engine-owned keepalive sweep: recipes declare idle lifetimes, the engine
// sweeps once for every declaring vendor (AUTH-6 keepalive leg)
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};

use ironclaw_auth::keepalive::{sweep_once, tick_once};
use ironclaw_auth::{
    AlwaysLeaderKeepaliveLock, CredentialAccount, CredentialAccountLookupRequest,
    CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    KeepaliveCandidateSource, KeepaliveLeaderLock, KeepaliveSweepDeps, KeepaliveSweepFuture,
    KeepaliveSweepSettings, LeaderOutcome, NewCredentialAccount,
    ProviderBackedCredentialAccountService,
};
use tokio_util::sync::CancellationToken;

fn keepalive_recipe_toml(vendor: &str, keepalive_idle_seconds: Option<u32>) -> String {
    let refresh = match keepalive_idle_seconds {
        Some(seconds) => format!("[refresh]\nkeepalive_idle_seconds = {seconds}\n"),
        None => String::new(),
    };
    format!(
        r#"
method = "oauth2_code"
display_name = "{vendor} account"
authorization_endpoint = "https://auth.{vendor}.example/authorize"
token_endpoint = "https://auth.{vendor}.example/token"
scopes = ["msg:read"]
client_credentials = {{ client_id_handle = "{vendor}_client_id", client_secret_handle = "{vendor}_client_secret" }}

{refresh}
[token_response]
access_token = "/access_token"
refresh_token = "/refresh_token"
expires_in = "/expires_in"
"#
    )
}

/// Candidate source the tests control explicitly. The sweep's recipe gating
/// and defensive `Configured`-only filter are the behavior under test; the
/// production source contract (Configured + refresh handle, all vendors) is
/// pinned at the composition tier.
#[derive(Default)]
struct ScriptedCandidateSource {
    accounts: Mutex<Vec<CredentialAccount>>,
    calls: AtomicUsize,
}

impl ScriptedCandidateSource {
    fn set(&self, accounts: Vec<CredentialAccount>) {
        *self.accounts.lock().unwrap() = accounts;
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl KeepaliveCandidateSource for ScriptedCandidateSource {
    async fn list_keepalive_candidates(&self) -> Vec<CredentialAccount> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.accounts.lock().unwrap().clone()
    }
}

/// A lock that never grants leadership; the sweep future must be dropped
/// untouched.
struct NeverLeaderLock;

#[async_trait]
impl KeepaliveLeaderLock for NeverLeaderLock {
    async fn run_as_leader(&self, _sweep: KeepaliveSweepFuture) -> LeaderOutcome<()> {
        LeaderOutcome::NotLeader
    }
}

/// Engine + scripted vendor server + ephemeral stores wired into sweep deps,
/// with the refresh port going through the REAL engine-owned refresh path
/// (`ProviderBackedCredentialAccountService` over `AuthEngine`).
struct SweepFixture {
    server: Arc<ScriptedVendorServer>,
    secrets: Arc<dyn SecretStore>,
    services: Arc<InMemoryAuthProductServices>,
    source: Arc<ScriptedCandidateSource>,
    deps: KeepaliveSweepDeps,
    scope: AuthProductScope,
}

impl SweepFixture {
    fn new(recipes: Vec<ResolvedVendorAuthRecipe>) -> Self {
        let mut by_vendor = HashMap::new();
        for recipe in &recipes {
            by_vendor.insert(
                recipe.vendor.clone(),
                (
                    format!("{}-client-id", recipe.vendor),
                    Some(format!("{}-client-secret", recipe.vendor)),
                ),
            );
        }
        let server = Arc::new(ScriptedVendorServer::default());
        let secrets: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::ephemeral());
        let engine = Arc::new(AuthEngine::new(AuthEngineDeps {
            recipes: Arc::new(StaticAuthRecipeResolver::new(recipes.clone())),
            configuration: Arc::new(StaticClientCredentials {
                by_vendor,
                non_secret_by_handle: HashMap::new(),
            }),
            egress: Arc::clone(&server) as Arc<dyn RuntimeHttpEgress>,
            secret_store: Arc::clone(&secrets),
            callback_base: EngineCallbackBase::new(CALLBACK_BASE).expect("callback base"),
            dcr_client_name: "IronClaw test".to_string(),
        }));
        let services = Arc::new(InMemoryAuthProductServices::new());
        let refresh = Arc::new(ProviderBackedCredentialAccountService::new(
            services.clone(),
            services.clone(),
            engine as Arc<dyn AuthProviderClient>,
        ));
        let source = Arc::new(ScriptedCandidateSource::default());
        let deps = KeepaliveSweepDeps {
            candidates: source.clone(),
            recipes: Arc::new(StaticAuthRecipeResolver::new(recipes)),
            refresh,
            leader_lock: Arc::new(AlwaysLeaderKeepaliveLock),
        };
        Self {
            server,
            secrets,
            services,
            source,
            deps,
            scope: test_scope(),
        }
    }

    async fn seed_account(&self, vendor: &str) -> CredentialAccount {
        let refresh_handle = SecretHandle::new(format!("{vendor}-seeded-refresh")).unwrap();
        self.secrets
            .put(
                self.scope.resource.clone(),
                refresh_handle.clone(),
                SecretString::from(format!("{vendor}-seeded-refresh-token")),
                None,
            )
            .await
            .unwrap();
        let account = self
            .services
            .create_account(NewCredentialAccount {
                scope: self.scope.clone(),
                provider: AuthProviderId::new(vendor).unwrap(),
                label: CredentialAccountLabel::new(vendor).unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: None,
                refresh_secret: Some(refresh_handle),
                scopes: vec![ProviderScope::new("msg:read").unwrap()],
            })
            .await
            .expect("seed account");
        let mut accounts = self.source.accounts.lock().unwrap().clone();
        accounts.push(account.clone());
        self.source.set(accounts);
        account
    }

    async fn stored_account(&self, account: &CredentialAccount) -> CredentialAccount {
        self.services
            .get_account(CredentialAccountLookupRequest::new(
                self.scope.clone(),
                account.id,
            ))
            .await
            .expect("lookup")
            .expect("account exists")
    }

    fn token_url(vendor: &str) -> String {
        format!("https://auth.{vendor}.example/token")
    }
}

const WEEK_SECONDS: u32 = 604_800;
const MONTH_SECONDS: u32 = 2_592_000;

#[tokio::test]
async fn keepalive_sweep_refreshes_due_accounts_of_declaring_vendors_only() {
    let fixture = SweepFixture::new(vec![
        synthetic_recipe("alpha", &keepalive_recipe_toml("alpha", Some(WEEK_SECONDS))),
        synthetic_recipe("beta", &keepalive_recipe_toml("beta", Some(MONTH_SECONDS))),
        synthetic_recipe("gamma", &keepalive_recipe_toml("gamma", None)),
    ]);
    let alpha = fixture.seed_account("alpha").await;
    let beta = fixture.seed_account("beta").await;
    let gamma = fixture.seed_account("gamma").await;

    fixture.server.script(
        &SweepFixture::token_url("alpha"),
        200,
        serde_json::json!({
            "access_token": "kept-alive-access",
            "expires_in": 3600
        }),
    );

    // Frozen clock 4 days ahead: alpha (7d lifetime) is past its half-life,
    // beta (30d lifetime) is fresh, gamma declares no lifetime at all.
    let now = Utc::now() + Duration::days(4);
    sweep_once(
        &fixture.deps,
        &KeepaliveSweepSettings::default(),
        &CancellationToken::new(),
        now,
    )
    .await;

    let alpha_requests = fixture
        .server
        .requests_for(&SweepFixture::token_url("alpha"));
    assert_eq!(alpha_requests.len(), 1, "alpha is refreshed exactly once");
    let form = alpha_requests[0].form();
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("refresh_token")
    );
    assert_eq!(
        form.get("refresh_token").map(String::as_str),
        Some("alpha-seeded-refresh-token")
    );
    assert_eq!(
        fixture.server.request_count(),
        1,
        "fresh and non-declaring vendors see no vendor traffic"
    );

    let refreshed = fixture.stored_account(&alpha).await;
    assert!(
        refreshed.updated_at > alpha.updated_at,
        "a keepalive refresh resets the idle clock"
    );
    assert_eq!(refreshed.status, CredentialAccountStatus::Configured);
    assert_eq!(
        fixture.stored_account(&beta).await.updated_at,
        beta.updated_at,
        "fresh accounts are untouched"
    );
    assert_eq!(
        fixture.stored_account(&gamma).await.updated_at,
        gamma.updated_at,
        "non-declaring vendors are never swept"
    );
}

#[tokio::test]
async fn keepalive_sweep_skips_the_tick_when_not_leader() {
    let fixture = SweepFixture::new(vec![synthetic_recipe(
        "alpha",
        &keepalive_recipe_toml("alpha", Some(WEEK_SECONDS)),
    )]);
    fixture.seed_account("alpha").await;

    let mut deps = fixture.deps.clone();
    deps.leader_lock = Arc::new(NeverLeaderLock);
    tick_once(
        &deps,
        &KeepaliveSweepSettings::default(),
        &CancellationToken::new(),
        Utc::now() + Duration::days(4),
    )
    .await;

    assert_eq!(
        fixture.source.call_count(),
        0,
        "non-leaders never enumerate candidates"
    );
    assert_eq!(
        fixture.server.request_count(),
        0,
        "non-leaders never touch a token endpoint"
    );
}

#[tokio::test]
async fn keepalive_refresh_failure_follows_engine_account_state_rules() {
    let fixture = SweepFixture::new(vec![
        synthetic_recipe("alpha", &keepalive_recipe_toml("alpha", Some(WEEK_SECONDS))),
        synthetic_recipe("beta", &keepalive_recipe_toml("beta", Some(MONTH_SECONDS))),
    ]);
    let alpha = fixture.seed_account("alpha").await;
    let beta = fixture.seed_account("beta").await;

    // alpha's vendor revoked the refresh token; beta refreshes fine.
    fixture.server.script(
        &SweepFixture::token_url("alpha"),
        400,
        serde_json::json!({ "error": "invalid_grant" }),
    );
    fixture.server.script(
        &SweepFixture::token_url("beta"),
        200,
        serde_json::json!({ "access_token": "beta-access", "expires_in": 3600 }),
    );

    // 16 days ahead both accounts are past their half-lives.
    let now = Utc::now() + Duration::days(16);
    sweep_once(
        &fixture.deps,
        &KeepaliveSweepSettings::default(),
        &CancellationToken::new(),
        now,
    )
    .await;

    assert_eq!(
        fixture
            .server
            .requests_for(&SweepFixture::token_url("alpha"))
            .len(),
        1
    );
    assert_eq!(
        fixture
            .server
            .requests_for(&SweepFixture::token_url("beta"))
            .len(),
        1,
        "one account's permanent failure does not abort the sweep"
    );
    let revoked = fixture.stored_account(&alpha).await;
    assert_eq!(
        revoked.status,
        CredentialAccountStatus::Revoked,
        "invalid_grant follows the engine's account-state rules"
    );
    assert!(
        fixture.stored_account(&beta).await.updated_at > beta.updated_at,
        "the healthy account still refreshed"
    );

    // Next tick: the revoked account is no longer refreshable and must be
    // excluded even if a loose candidate source still lists it.
    let refreshed_beta = fixture.stored_account(&beta).await;
    fixture.source.set(vec![revoked, refreshed_beta]);
    fixture.server.script(
        &SweepFixture::token_url("beta"),
        200,
        serde_json::json!({ "access_token": "beta-access-2", "expires_in": 3600 }),
    );
    sweep_once(
        &fixture.deps,
        &KeepaliveSweepSettings::default(),
        &CancellationToken::new(),
        now,
    )
    .await;
    assert_eq!(
        fixture
            .server
            .requests_for(&SweepFixture::token_url("alpha"))
            .len(),
        1,
        "a revoked account is never re-swept"
    );
}

#[test]
fn google_manifests_declare_the_keepalive_idle_lifetime_identically() {
    let gmail = manifest_recipe("gmail", "google");
    assert_eq!(
        gmail.recipe.keepalive_idle_threshold(),
        Some(std::time::Duration::from_secs(u64::from(WEEK_SECONDS))),
        "google refresh tokens idle-die after 7 days (testing publishing status)"
    );
    for package in [
        "google-calendar",
        "google-docs",
        "google-drive",
        "google-sheets",
        "google-slides",
    ] {
        let sibling = manifest_recipe(package, "google");
        assert_eq!(
            sibling.recipe.keepalive_idle_threshold(),
            gmail.recipe.keepalive_idle_threshold(),
            "{package} must declare the same keepalive lifetime as gmail"
        );
        assert!(
            gmail.recipe.compatible_for_shared_vendor(&sibling.recipe),
            "{package} must stay shared-vendor compatible with gmail"
        );
    }
}
