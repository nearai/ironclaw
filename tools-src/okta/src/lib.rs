//! Okta WASM Tool for IronClaw.
//!
//! Provides user profile management, SSO app catalog browsing, and
//! launch links for all applications under Okta single sign-on.
//!
//! # Setup
//!
//! 1. Configure OAuth2 with PKCE (see capabilities.json instructions)
//! 2. Write your Okta domain to workspace: `memory_write(path="okta/domain", content="mycompany.okta.com")`
//! 3. All actions read the domain from workspace automatically
//!
//! # Capabilities Required
//!
//! - HTTP: `*.okta.com/api/v1/*`, `*.okta.com/idp/myaccount/*` (GET, POST, PUT)
//! - Secrets: `okta_oauth_token` (injected as Bearer token)
//! - Workspace: `okta/` prefix (read-only, for domain config)
//!
//! # Supported Actions
//!
//! - `get_profile`: Fetch the current user's profile
//! - `update_profile`: Update profile fields
//! - `list_apps`: List all SSO apps assigned to the user
//! - `search_apps`: Search apps by name
//! - `get_app_sso_link`: Get the SSO launch URL for a specific app
//! - `get_org_info`: Get organization details

mod api;
mod types;

use types::OktaAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct OktaTool;

impl exports::near::agent::tool::Guest for OktaTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        r#"{
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get_profile", "update_profile", "list_apps", "search_apps", "get_app_sso_link", "get_org_info"],
                    "description": "The Okta operation to perform"
                },
                "fields": {
                    "type": "object",
                    "description": "Profile fields to update (e.g., firstName, lastName, email, mobilePhone, displayName, title, department). Required for: update_profile"
                },
                "query": {
                    "type": "string",
                    "description": "Case-insensitive search query to match against app labels and names. Required for: search_apps"
                },
                "app": {
                    "type": "string",
                    "description": "App instance ID (e.g., '0oa1xxx') or app label (e.g., 'Google Workspace'). Required for: get_app_sso_link"
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Okta SSO tool for managing your profile and accessing all applications under \
         single sign-on. Supports viewing/updating your Okta profile, listing all assigned \
         SSO apps, searching apps by name, and getting direct SSO launch links. Requires \
         Okta domain in workspace at 'okta/domain' and an OAuth token with \
         okta.users.read.self, okta.users.manage.self, and okta.apps.read scopes."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("okta_oauth_token") {
        return Err(
            "Okta OAuth token not configured. Please add the 'okta_oauth_token' secret \
             via OAuth2 flow or set the OKTA_OAUTH_TOKEN environment variable."
                .to_string(),
        );
    }

    let action: OktaAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Okta action: {:?}", action),
    );

    match action {
        OktaAction::GetProfile => api::get_profile(),
        OktaAction::UpdateProfile { fields } => api::update_profile(&fields),
        OktaAction::ListApps => api::list_apps(),
        OktaAction::SearchApps { query } => api::search_apps(&query),
        OktaAction::GetAppSsoLink { app } => api::get_app_sso_link(&app),
        OktaAction::GetOrgInfo => api::get_org_info(),
    }
}

export!(OktaTool);
