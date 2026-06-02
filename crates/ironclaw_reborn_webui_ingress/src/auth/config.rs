//! Configuration types for the WebChat v2 OAuth login surface.
//!
//! Host composition builds a [`GoogleOAuthConfig`] from operator
//! input (env vars, TOML config) and hands it to
//! [`webui_v2_auth_router`](super::webui_v2_auth_router) along with a
//! `SessionStore` and a `UserDirectory`. The composition layer is
//! responsible for picking which providers are enabled; this crate
//! never reads env vars directly so a binary that uses a different
//! config source can still wire it.

use secrecy::SecretString;

/// Google OAuth (OIDC) configuration. Mirrors the v1 gateway's
/// `GoogleOAuthConfig` shape so existing operator config can be
/// re-used by the v2 wire-up.
#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    /// OAuth 2.0 client id issued by Google Cloud Console.
    pub client_id: String,
    /// OAuth 2.0 client secret. Wrapped in [`SecretString`] so the
    /// `Debug` impl is redacted.
    pub client_secret: SecretString,
    /// Optional Google Workspace hosted domain restriction
    /// (e.g. `company.com`). When set, the authorization URL hints
    /// the account picker and the callback rejects any ID token
    /// whose `hd` claim does not match.
    pub allowed_hd: Option<String>,
}

/// GitHub OAuth configuration. Mirrors the v1 gateway's
/// `GitHubOAuthConfig` shape so existing operator config can be
/// re-used by the v2 wire-up.
///
/// GitHub's OAuth App flow does not support PKCE; CSRF is protected
/// solely by the `state` parameter the router mints, so there is no
/// hosted-domain analogue here — the provider just needs the client
/// credentials.
#[derive(Debug, Clone)]
pub struct GitHubOAuthConfig {
    /// OAuth App client id issued by GitHub.
    pub client_id: String,
    /// OAuth App client secret. Wrapped in [`SecretString`] so the
    /// `Debug` impl is redacted.
    pub client_secret: SecretString,
}

/// NEAR network the wallet login flow validates access keys against.
///
/// A small fixed set, so an enum rather than a free `String`
/// (`.claude/rules/types.md`): the network selects the default RPC
/// endpoint and is echoed to the SPA so the wallet connector targets
/// the matching chain. The wire form is snake_case (`mainnet` /
/// `testnet`) so it round-trips cleanly through the challenge JSON
/// and the SPA's `near-connect` connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NearNetwork {
    Mainnet,
    Testnet,
}

impl NearNetwork {
    /// Stable wire / UI identifier. Used in the challenge response and
    /// matched against the `near-connect` network parameter on the SPA.
    pub fn as_str(self) -> &'static str {
        match self {
            NearNetwork::Mainnet => "mainnet",
            NearNetwork::Testnet => "testnet",
        }
    }

    /// Default public RPC endpoint for the network. Used when
    /// [`NearAuthConfig::rpc_url`] is left unset by the operator.
    pub fn default_rpc_url(self) -> &'static str {
        match self {
            NearNetwork::Mainnet => "https://rpc.mainnet.near.org",
            NearNetwork::Testnet => "https://rpc.testnet.near.org",
        }
    }
}

/// NEAR wallet login configuration. Unlike the OAuth providers above,
/// NEAR uses a NEP-413 challenge/verify flow rather than an
/// authorization-code redirect, so it is wired separately from the
/// [`OAuthProvider`](super::provider::OAuthProvider) list — see
/// [`NearLoginProvider`](super::near::NearLoginProvider).
///
/// Mirrors the v1 gateway's `NearAuthConfig` shape (network + RPC URL)
/// so existing operator config can be re-used by the v2 wire-up.
#[derive(Debug, Clone)]
pub struct NearAuthConfig {
    /// Network access keys are validated against. Selects the default
    /// RPC endpoint and is advertised to the SPA wallet connector.
    pub network: NearNetwork,
    /// RPC endpoint used for the `view_access_key` query. When `None`,
    /// [`NearNetwork::default_rpc_url`] is used.
    pub rpc_url: Option<String>,
}

impl NearAuthConfig {
    /// Resolve the effective RPC URL: the operator override when set,
    /// otherwise the network's default public endpoint.
    pub fn resolved_rpc_url(&self) -> String {
        self.rpc_url
            .clone()
            .unwrap_or_else(|| self.network.default_rpc_url().to_string())
    }
}
