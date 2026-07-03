use ironclaw_host_api::{ExtensionId, NetworkPolicy, NetworkScheme, NetworkTargetPattern};

use super::manifest::{CALENDAR_EXTENSION_ID, GMAIL_EXTENSION_ID};

pub fn gsuite_network_policy_for(provider: &ExtensionId) -> Option<NetworkPolicy> {
    if matches!(
        provider.as_str(),
        GMAIL_EXTENSION_ID | CALENDAR_EXTENSION_ID
    ) {
        Some(google_api_network_policy())
    } else {
        None
    }
}

pub fn google_api_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![
            https("www.googleapis.com"),
            https("gmail.googleapis.com"),
            https("calendar.googleapis.com"),
            https("oauth2.googleapis.com"),
            https("accounts.google.com"),
        ],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10 * 1024 * 1024),
    }
}

fn https(host_pattern: impl Into<String>) -> NetworkTargetPattern {
    NetworkTargetPattern {
        scheme: Some(NetworkScheme::Https),
        host_pattern: host_pattern.into(),
        port: None,
    }
}
