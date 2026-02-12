use crate::near::agent::host;
use crate::types::*;

const WORKSPACE_DOMAIN_PATH: &str = "okta/domain";

/// Read the configured Okta domain from workspace, or return a helpful error.
fn get_domain() -> Result<String, String> {
    host::workspace_read(WORKSPACE_DOMAIN_PATH).ok_or_else(|| {
        "Okta domain not configured. Write your Okta domain to workspace path 'okta/domain' \
         using the memory_write tool (e.g., memory_write with path='okta/domain' and \
         content='mycompany.okta.com')."
            .to_string()
    })
}

/// Build the base URL for the Okta Management API.
fn management_base(domain: &str) -> String {
    format!("https://{}/api/v1", domain)
}

/// Make an Okta API call.
fn okta_api_call(method: &str, url: &str, body: Option<&str>) -> Result<String, String> {
    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json", "Accept": "application/json"}"#
    } else {
        r#"{"Accept": "application/json"}"#
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(
        host::LogLevel::Debug,
        &format!("Okta API: {} {}", method, url),
    );

    let response = host::http_request(method, url, headers, body_bytes.as_deref())?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        // Try to extract Okta's error summary for a better message.
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body_text) {
            if let Some(summary) = parsed["errorSummary"].as_str() {
                return Err(format!("Okta API error ({}): {}", response.status, summary));
            }
        }
        return Err(format!(
            "Okta API returned status {}: {}",
            response.status, body_text
        ));
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8: {}", e))
}

// ---------------------------------------------------------------------------
// Action implementations
// ---------------------------------------------------------------------------

/// GET /api/v1/users/me
pub fn get_profile() -> Result<String, String> {
    let domain = get_domain()?;
    let url = format!("{}/users/me", management_base(&domain));
    let response = okta_api_call("GET", &url, None)?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let profile = parse_user_profile(&parsed)?;
    serde_json::to_string(&profile).map_err(|e| e.to_string())
}

/// POST /api/v1/users/me (partial update via Management API)
pub fn update_profile(fields: &serde_json::Value) -> Result<String, String> {
    let domain = get_domain()?;
    let url = format!("{}/users/me", management_base(&domain));

    // Wrap fields under "profile" key for Okta's expected format.
    let payload = serde_json::json!({ "profile": fields });
    let body = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    let response = okta_api_call("POST", &url, Some(&body))?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let profile = parse_user_profile(&parsed)?;
    let result = UpdateProfileResult {
        success: true,
        profile,
    };
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

/// GET /api/v1/users/me/appLinks
pub fn list_apps() -> Result<String, String> {
    let domain = get_domain()?;
    let url = format!("{}/users/me/appLinks", management_base(&domain));
    let response = okta_api_call("GET", &url, None)?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let apps = parse_app_links(&parsed)?;
    let count = apps.len();
    let result = ListAppsResult { apps, count };
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

/// Search apps by label (case-insensitive substring match).
pub fn search_apps(query: &str) -> Result<String, String> {
    let domain = get_domain()?;
    let url = format!("{}/users/me/appLinks", management_base(&domain));
    let response = okta_api_call("GET", &url, None)?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let all_apps = parse_app_links(&parsed)?;
    let query_lower = query.to_lowercase();

    let apps: Vec<AppLink> = all_apps
        .into_iter()
        .filter(|app| {
            app.label.to_lowercase().contains(&query_lower)
                || app.app_name.to_lowercase().contains(&query_lower)
        })
        .collect();

    let count = apps.len();
    let result = ListAppsResult { apps, count };
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

/// Find an app by ID or label and return its SSO launch link.
pub fn get_app_sso_link(app: &str) -> Result<String, String> {
    let domain = get_domain()?;
    let url = format!("{}/users/me/appLinks", management_base(&domain));
    let response = okta_api_call("GET", &url, None)?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let all_apps = parse_app_links(&parsed)?;
    let app_lower = app.to_lowercase();

    // Try exact ID match first, then case-insensitive label match.
    let found = all_apps
        .iter()
        .find(|a| a.app_instance_id == app)
        .or_else(|| {
            all_apps
                .iter()
                .find(|a| a.label.to_lowercase() == app_lower)
        })
        .or_else(|| {
            all_apps
                .iter()
                .find(|a| a.label.to_lowercase().contains(&app_lower))
        });

    match found {
        Some(app_link) => {
            let result = AppSsoLinkResult {
                label: app_link.label.clone(),
                link_url: app_link.link_url.clone(),
                app_instance_id: app_link.app_instance_id.clone(),
                app_name: app_link.app_name.clone(),
            };
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        None => {
            let available: Vec<String> = all_apps.iter().map(|a| a.label.clone()).collect();
            Err(format!(
                "App '{}' not found. Available apps: {}",
                app,
                available.join(", ")
            ))
        }
    }
}

/// GET /idp/myaccount/organization
pub fn get_org_info() -> Result<String, String> {
    let domain = get_domain()?;
    let url = format!("https://{}/idp/myaccount/organization", domain);

    // MyAccount API requires the okta-version header.
    let response = okta_api_call_with_headers(
        "GET",
        &url,
        None,
        r#"{"Accept": "application/json; okta-version=1.0.0"}"#,
    )?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let result = OrgInfo {
        id: parsed["id"].as_str().unwrap_or("").to_string(),
        name: parsed["name"].as_str().unwrap_or("").to_string(),
        subdomain: parsed["subdomain"].as_str().map(|s| s.to_string()),
        website: parsed["website"].as_str().map(|s| s.to_string()),
        support_phone: parsed["supportPhoneNumber"].as_str().map(|s| s.to_string()),
        technical_contact: parsed["technicalContact"].as_str().map(|s| s.to_string()),
    };
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Like `okta_api_call` but with custom headers (for MyAccount API versioning).
fn okta_api_call_with_headers(
    method: &str,
    url: &str,
    body: Option<&str>,
    headers: &str,
) -> Result<String, String> {
    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(
        host::LogLevel::Debug,
        &format!("Okta API: {} {}", method, url),
    );

    let response = host::http_request(method, url, headers, body_bytes.as_deref())?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body_text) {
            if let Some(summary) = parsed["errorSummary"].as_str() {
                return Err(format!("Okta API error ({}): {}", response.status, summary));
            }
        }
        return Err(format!(
            "Okta API returned status {}: {}",
            response.status, body_text
        ));
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8: {}", e))
}

fn parse_user_profile(v: &serde_json::Value) -> Result<UserProfile, String> {
    let p = &v["profile"];
    Ok(UserProfile {
        id: v["id"].as_str().unwrap_or("").to_string(),
        status: v["status"].as_str().unwrap_or("").to_string(),
        first_name: p["firstName"].as_str().unwrap_or("").to_string(),
        last_name: p["lastName"].as_str().unwrap_or("").to_string(),
        email: p["email"].as_str().unwrap_or("").to_string(),
        login: p["login"].as_str().unwrap_or("").to_string(),
        mobile_phone: p["mobilePhone"].as_str().map(|s| s.to_string()),
        display_name: p["displayName"].as_str().map(|s| s.to_string()),
        nick_name: p["nickName"].as_str().map(|s| s.to_string()),
        title: p["title"].as_str().map(|s| s.to_string()),
        department: p["department"].as_str().map(|s| s.to_string()),
        organization: p["organization"].as_str().map(|s| s.to_string()),
        timezone: p["timezone"].as_str().map(|s| s.to_string()),
        locale: p["locale"].as_str().map(|s| s.to_string()),
    })
}

fn parse_app_links(v: &serde_json::Value) -> Result<Vec<AppLink>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "Expected array of app links from Okta".to_string())?;

    Ok(arr
        .iter()
        .map(|a| AppLink {
            app_instance_id: a["appInstanceId"].as_str().unwrap_or("").to_string(),
            label: a["label"].as_str().unwrap_or("").to_string(),
            link_url: a["linkUrl"].as_str().unwrap_or("").to_string(),
            logo_url: a["logoUrl"].as_str().map(|s| s.to_string()),
            app_name: a["appName"].as_str().unwrap_or("").to_string(),
            hidden: a["hidden"].as_bool().unwrap_or(false),
        })
        .collect())
}
