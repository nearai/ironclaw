use serde::{Deserialize, Serialize};

/// Input parameters for the Okta tool.
///
/// Actions map to Okta Management API (/api/v1/) and MyAccount API (/idp/myaccount/).
/// The tool reads the Okta domain from workspace at `okta/domain`.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OktaAction {
    /// Get the current user's Okta profile.
    GetProfile,

    /// Update fields on the current user's profile (partial update).
    UpdateProfile {
        /// Key-value pairs of profile fields to update.
        /// Common fields: firstName, lastName, email, mobilePhone, displayName,
        /// nickName, title, department, organization.
        fields: serde_json::Value,
    },

    /// List all SSO applications assigned to the current user.
    ListApps,

    /// Search assigned apps by name (case-insensitive substring match).
    SearchApps {
        /// Search query to match against app labels.
        query: String,
    },

    /// Get the SSO launch link for a specific app by its instance ID or label.
    GetAppSsoLink {
        /// App instance ID (e.g., "0oa1xxx") or app label to search for.
        app: String,
    },

    /// Get information about the Okta organization.
    GetOrgInfo,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// User profile from Okta.
#[derive(Debug, Serialize)]
pub struct UserProfile {
    pub id: String,
    pub status: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile_phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nick_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

/// Result of a profile update.
#[derive(Debug, Serialize)]
pub struct UpdateProfileResult {
    pub success: bool,
    pub profile: UserProfile,
}

/// An SSO app link (chiclet) assigned to the user.
#[derive(Debug, Serialize)]
pub struct AppLink {
    pub app_instance_id: String,
    pub label: String,
    pub link_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    pub app_name: String,
    pub hidden: bool,
}

/// Result of listing or searching apps.
#[derive(Debug, Serialize)]
pub struct ListAppsResult {
    pub apps: Vec<AppLink>,
    pub count: usize,
}

/// SSO launch link for a specific app.
#[derive(Debug, Serialize)]
pub struct AppSsoLinkResult {
    pub label: String,
    pub link_url: String,
    pub app_instance_id: String,
    pub app_name: String,
}

/// Okta organization info.
#[derive(Debug, Serialize)]
pub struct OrgInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subdomain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub technical_contact: Option<String>,
}
