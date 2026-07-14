//! Reborn product-auth cluster.
//!
//! Groups the product-auth surface — public API/prompt types (`api`), OAuth
//! provider composition (`oauth`), durable flow/account state (`durable`),
//! WebUI route serving (`serve`), and runtime credential resolution/refresh
//! (`credentials`) — behind one internal module. The crate root re-exports the
//! same public items from here so the crate's public API is unchanged.

pub(crate) mod api;
pub(crate) mod credentials;
pub(crate) mod durable;
pub(crate) mod oauth;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod serve;

const RETIRED_SLACK_PERSONAL_PROVIDER_ID: &str = "slack_personal"; // taxonomy-allow: retired-normal-write-rejection

fn reject_retired_provider(
    provider: &ironclaw_auth::AuthProviderId,
) -> Result<(), ironclaw_auth::AuthProductError> {
    if provider.as_str() == RETIRED_SLACK_PERSONAL_PROVIDER_ID {
        return Err(ironclaw_auth::AuthProductError::InvalidRequest {
            reason: "the slack_personal provider was retired; use slack".to_string(),
        });
    }
    Ok(())
}
