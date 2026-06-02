//! NEAR wallet login for the WebChat v2 auth surface.
//!
//! NEAR does not fit the OAuth authorization-code flow the
//! [`OAuthProvider`](super::provider::OAuthProvider) trait models, so
//! it is wired separately: a NEP-413 challenge/verify pair rather than
//! a redirect + callback. The [`NearLoginProvider`] owns the nonce
//! store, the RPC client, and the verification pipeline; the route
//! handlers in `auth/routes.rs` call [`NearLoginProvider::challenge`]
//! and [`NearLoginProvider::verify`] and reuse the SAME
//! [`SessionStore`](crate::session::SessionStore) +
//! [`UserDirectory`](super::user_directory::UserDirectory) seam the
//! OAuth callback uses.
//!
//! Flow:
//! 1. `GET /auth/near/challenge` → mint a single-use nonce + the
//!    message the wallet signs (`NearChallenge`).
//! 2. The SPA asks the wallet to sign the NEP-413 payload and POSTs
//!    `{ account_id, public_key, signature, nonce }` back.
//! 3. `POST /auth/near/verify` → consume the nonce, verify the Ed25519
//!    signature is bound to it, confirm the key is an active access
//!    key on the account via RPC, and project a normalized
//!    [`OAuthUserProfile`].

mod nonce;
mod verify;

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::config::{NearAuthConfig, NearNetwork};
use super::error::ProviderInitError;
use super::profile::OAuthUserProfile;
use super::provider_name::OAuthProviderName;
use nonce::NearNonceStore;
use verify::{
    canonical_public_key, decode_nonce_bytes, decode_public_key, decode_signature, login_message,
    validate_account_id, verify_access_key, verify_near_signature,
};

pub(crate) use verify::NearVerifyError;

/// Stable provider identifier advertised on `/auth/providers` and used
/// in the `/auth/near/*` route paths.
const NEAR_PROVIDER_NAME: &str = "near";

/// Per-call timeout on the `view_access_key` RPC request. The default
/// `reqwest::Client` has no timeout, which would let a hung RPC pin the
/// verify handler indefinitely. Matches the v1 gateway's 10s budget.
const NEAR_RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Challenge payload returned by `GET /auth/near/challenge`. The
/// wallet signs `message` over the NEP-413 framing; `nonce` is the
/// hex-encoded single-use challenge the SPA echoes back on verify;
/// `network` lets the SPA target the matching chain in its wallet
/// connector.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct NearChallenge {
    pub nonce: String,
    pub message: String,
    pub recipient: String,
    pub network: String,
}

/// Verify request body POSTed to `/auth/near/verify`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NearVerifyRequest {
    pub account_id: String,
    pub public_key: String,
    pub signature: String,
    pub nonce: String,
}

/// NEAR wallet login provider — owns the nonce store, RPC client, and
/// the NEP-413 verification pipeline.
pub struct NearLoginProvider {
    name: OAuthProviderName,
    network: NearNetwork,
    rpc_url: String,
    http: reqwest::Client,
    nonce_store: NearNonceStore,
}

impl NearLoginProvider {
    /// Build a provider from operator-supplied [`NearAuthConfig`],
    /// using the resolved RPC endpoint (operator override or the
    /// network default).
    ///
    /// Fallible for the same reason [`GitHubProvider::new`](super::github::GitHubProvider::new)
    /// is: a `reqwest::Client` build failure (rustls / tokio runtime)
    /// surfaces as a `Result` so host composition fails startup loudly
    /// rather than silently dropping the timeout.
    pub fn new(config: NearAuthConfig) -> Result<Self, ProviderInitError> {
        let rpc_url = config.resolved_rpc_url();
        Self::with_rpc_endpoint_inner(config.network, rpc_url)
    }

    /// Test / dev-only constructor that points the RPC client at a
    /// local mock endpoint. Gated like the GitHub provider's
    /// `with_endpoints` so caller-level tests in the sibling `tests/`
    /// crate can reach it.
    #[cfg(any(test, feature = "dev-in-memory-session"))]
    pub fn with_rpc_endpoint(
        network: NearNetwork,
        rpc_url: impl Into<String>,
    ) -> Result<Self, ProviderInitError> {
        Self::with_rpc_endpoint_inner(network, rpc_url.into())
    }

    fn with_rpc_endpoint_inner(
        network: NearNetwork,
        rpc_url: String,
    ) -> Result<Self, ProviderInitError> {
        let http = reqwest::Client::builder()
            .timeout(NEAR_RPC_TIMEOUT)
            .user_agent("IronClaw-WebChat-v2")
            .build()
            .map_err(|err| ProviderInitError(err.to_string()))?;
        Ok(Self {
            name: OAuthProviderName::new(NEAR_PROVIDER_NAME)
                .expect("\"near\" satisfies the OAuthProviderName grammar"), // safety: literal is lowercase ascii, 4 chars; covered by OAuthProviderName grammar tests
            network,
            rpc_url,
            http,
            nonce_store: NearNonceStore::new(),
        })
    }

    pub(crate) fn name(&self) -> &OAuthProviderName {
        &self.name
    }

    /// Mint a fresh challenge. Single-use nonce + the exact message the
    /// wallet must sign.
    pub(crate) fn challenge(&self) -> NearChallenge {
        let nonce = self.nonce_store.generate();
        let message = login_message(&nonce);
        NearChallenge {
            nonce,
            message,
            recipient: verify::NEAR_RECIPIENT.to_string(),
            network: self.network.as_str().to_string(),
        }
    }

    /// Run the full verification pipeline and project a normalized
    /// profile on success. The nonce is consumed FIRST (single-use even
    /// on a later failure) so a replayed verify cannot re-spend it.
    pub(crate) async fn verify(
        &self,
        req: &NearVerifyRequest,
    ) -> Result<OAuthUserProfile, NearVerifyError> {
        // Single-use nonce check up front: consuming before any other
        // validation guarantees a replayed request cannot re-run the
        // pipeline against the same challenge.
        if !self.nonce_store.consume(&req.nonce) {
            return Err(NearVerifyError::InvalidNonce);
        }

        validate_account_id(&req.account_id)?;
        let public_key = decode_public_key(&req.public_key)?;
        let signature = decode_signature(&req.signature)?;
        let nonce_bytes = decode_nonce_bytes(&req.nonce)?;
        let message = login_message(&req.nonce);

        verify_near_signature(
            &public_key,
            &signature,
            &message,
            &nonce_bytes,
            verify::NEAR_RECIPIENT,
        )?;

        let canonical = canonical_public_key(&public_key);
        verify_access_key(&self.http, &self.rpc_url, &req.account_id, &canonical).await?;

        // NEAR has no email; the directory resolves the login by the
        // provider-unique account id. `email_verified = false` keeps an
        // email-matching `UserDirectory` from ever treating a NEAR
        // login as an email-bearing identity.
        Ok(OAuthUserProfile {
            provider_user_id: req.account_id.clone(),
            email: None,
            email_verified: false,
            display_name: Some(req.account_id.clone()),
        })
    }
}

impl std::fmt::Debug for NearLoginProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearLoginProvider")
            .field("network", &self.network)
            .field("rpc_url", &self.rpc_url)
            .finish_non_exhaustive()
    }
}
