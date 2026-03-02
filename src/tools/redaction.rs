use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";
const SENSITIVE_EXACT: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "api_key",
    "access_token",
    "refresh_token",
    "session_token",
    "id_token",
    "token",
    "password",
    "passwd",
    "secret",
    "client_secret",
    "private_key",
    "apikey",
    "apisecret",
];

fn split_camel_case_key_parts(key: &str) -> Vec<String> {
    if key.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = key.chars().collect();
    let mut parts = Vec::new();
    let mut start = 0;

    for i in 1..chars.len() {
        let prev = chars[i - 1];
        let cur = chars[i];
        let next = chars.get(i + 1).copied();

        let boundary = (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
            || (prev.is_ascii_alphabetic() && cur.is_ascii_digit())
            || (prev.is_ascii_digit() && cur.is_ascii_alphabetic())
            || (prev.is_ascii_uppercase()
                && cur.is_ascii_uppercase()
                && next.map(|n| n.is_ascii_lowercase()).unwrap_or(false));

        if boundary {
            parts.push(chars[start..i].iter().collect::<String>());
            start = i;
        }
    }

    parts.push(chars[start..].iter().collect::<String>());
    parts
}

fn tokenize_key_parts(key: &str) -> Vec<String> {
    let mut parts = Vec::new();

    for segment in key.split(|c: char| !c.is_ascii_alphanumeric()) {
        if segment.is_empty() {
            continue;
        }

        if segment
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            parts.push(segment.to_string());
        } else {
            parts.extend(split_camel_case_key_parts(segment));
        }
    }

    parts.into_iter().map(|p| p.to_ascii_lowercase()).collect()
}

fn has_any(parts: &[String], candidates: &[&str]) -> bool {
    parts
        .iter()
        .any(|part| candidates.iter().any(|candidate| part == candidate))
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if SENSITIVE_EXACT.contains(&lower.as_str()) {
        return true;
    }

    let parts = tokenize_key_parts(key);
    if parts.is_empty() {
        return false;
    }

    if has_any(&parts, &["password", "passwd", "secret", "credential"]) {
        return true;
    }

    if has_any(&parts, &["authorization", "cookie"]) {
        return true;
    }

    let has_token = has_any(&parts, &["token", "jwt"]);
    let has_key = has_any(&parts, &["key"]);
    let has_context = has_any(
        &parts,
        &[
            "auth",
            "oauth",
            "authorization",
            "api",
            "access",
            "refresh",
            "session",
            "bearer",
            "private",
            "client",
            "id",
        ],
    );

    has_context && (has_token || has_key)
}

fn redact_in_place(value: &mut Value) {
    match value {
        Value::Object(map) => redact_object(map),
        Value::Array(items) => {
            for item in items {
                redact_in_place(item);
            }
        }
        _ => {}
    }
}

fn redact_object(map: &mut Map<String, Value>) {
    for (key, val) in map {
        if is_sensitive_key(key) {
            *val = Value::String(REDACTED.to_string());
        } else {
            redact_in_place(val);
        }
    }
}

pub fn redact_sensitive_json(value: &Value) -> Value {
    let mut cloned = value.clone();
    redact_in_place(&mut cloned);
    cloned
}

#[cfg(test)]
mod tests {
    use super::{is_sensitive_key, redact_sensitive_json};

    #[test]
    fn redacts_exact_sensitive_keys() {
        let input = serde_json::json!({
            "headers": {
                "Authorization": "Bearer abc",
                "x-api-key": "k-123",
                "content-type": "application/json"
            },
            "password": "p@ss"
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out["headers"]["Authorization"], "[REDACTED]");
        assert_eq!(out["headers"]["x-api-key"], "[REDACTED]");
        assert_eq!(out["headers"]["content-type"], "application/json");
        assert_eq!(out["password"], "[REDACTED]");
    }

    #[test]
    fn redacts_nested_sensitive_keys() {
        let input = serde_json::json!({
            "body": {
                "clientSecret": "xyz",
                "nested": [{"authToken": "123"}, {"query": "ok"}]
            }
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out["body"]["clientSecret"], "[REDACTED]");
        assert_eq!(out["body"]["nested"][0]["authToken"], "[REDACTED]");
        assert_eq!(out["body"]["nested"][1]["query"], "ok");
    }

    #[test]
    fn does_not_over_redact_common_non_sensitive_keys() {
        assert!(!is_sensitive_key("author"));
        assert!(!is_sensitive_key("authorize_user"));
        assert!(!is_sensitive_key("token_count"));
        assert!(!is_sensitive_key("tokenize"));
        assert!(!is_sensitive_key("oauth_redirect_uri"));
    }

    #[test]
    fn still_redacts_expected_token_keys() {
        assert!(is_sensitive_key("auth_token"));
        assert!(is_sensitive_key("oauth_token"));
        assert!(is_sensitive_key("accessToken"));
        assert!(is_sensitive_key("apiKey"));
    }
}
