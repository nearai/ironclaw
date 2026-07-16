use std::path::Path;

pub(crate) fn redact_libsql_path(_path: &Path) -> String {
    "libsql://<redacted-local-path>".to_string()
}

#[cfg(any(feature = "postgres", test))]
pub(crate) fn redact_postgres_url(url: &str) -> String {
    if let Some(redacted) = redact_postgres_uri(url, "postgres://") {
        return redacted;
    }
    if let Some(redacted) = redact_postgres_uri(url, "postgresql://") {
        return redacted;
    }
    if let Some(redacted) = redact_postgres_key_value_config(url) {
        return redacted;
    }
    "postgres://<redacted>".to_string()
}

#[cfg(any(feature = "postgres", test))]
fn redact_postgres_uri(url: &str, scheme: &str) -> Option<String> {
    let rest = url.strip_prefix(scheme)?;
    let redacted_rest = match rest.find('@') {
        Some(at) => format!("<redacted>@{}", redact_uri_password_query(&rest[at + 1..])),
        None => redact_uri_password_query(rest),
    };
    Some(format!("{scheme}{redacted_rest}"))
}

#[cfg(any(feature = "postgres", test))]
fn redact_uri_password_query(rest: &str) -> String {
    let Some((prefix, query)) = rest.split_once('?') else {
        return rest.to_string();
    };
    let redacted_query = query
        .split('&')
        .map(|pair| {
            let key = pair.split_once('=').map(|(key, _)| key).unwrap_or(pair);
            if key.eq_ignore_ascii_case("password") {
                format!("{key}=<redacted>")
            } else {
                pair.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{prefix}?{redacted_query}")
}

#[cfg(any(feature = "postgres", test))]
fn redact_postgres_key_value_config(config: &str) -> Option<String> {
    let mut saw_assignment = false;
    let parts = config
        .split_whitespace()
        .map(|part| {
            let Some((key, _)) = part.split_once('=') else {
                return part.to_string();
            };
            saw_assignment = true;
            if key.eq_ignore_ascii_case("password") {
                format!("{key}=<redacted>")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>();
    if saw_assignment {
        Some(parts.join(" "))
    } else {
        None
    }
}
