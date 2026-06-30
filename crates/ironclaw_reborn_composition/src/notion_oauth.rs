use crate::oauth_provider_client::{ExchangeScopePolicy, HostOAuthProviderSpec};

pub(crate) const NOTION_PROVIDER_ID: &str = "notion";
const NOTION_OAUTH_CAPABILITY: &str = "ironclaw_auth.notion_oauth";
const NOTION_TOKEN_ENDPOINT: &str = "https://mcp.notion.com/token";
const NOTION_RESOURCE: &str = "https://mcp.notion.com/mcp";

pub(crate) fn notion_provider_spec() -> HostOAuthProviderSpec {
    HostOAuthProviderSpec {
        provider_id: NOTION_PROVIDER_ID,
        capability_id: NOTION_OAUTH_CAPABILITY,
        token_endpoint: NOTION_TOKEN_ENDPOINT,
        secret_handle_prefix: "notion",
        resource: Some(NOTION_RESOURCE),
        exchange_scope_policy: ExchangeScopePolicy::FallbackToRequested,
    }
}
