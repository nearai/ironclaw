use std::collections::HashMap;

use crate::SandboxPlanError;

pub(crate) fn validate_docker_image_reference(image: &str) -> Result<(), SandboxPlanError> {
    if image.is_empty() {
        return Err(SandboxPlanError::InvalidImage {
            reason: "must not be empty".to_string(),
        });
    }
    if image.starts_with('-') {
        return Err(SandboxPlanError::InvalidImage {
            reason: "must not start with '-'".to_string(),
        });
    }
    if image.chars().any(char::is_whitespace) {
        return Err(SandboxPlanError::InvalidImage {
            reason: "must not contain whitespace".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_host(host: &str) -> Result<(), SandboxPlanError> {
    if host.is_empty() {
        return Err(SandboxPlanError::InvalidHost {
            host: host.to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if host.contains('/') || host.contains(':') || host.chars().any(char::is_whitespace) {
        return Err(SandboxPlanError::InvalidHost {
            host: host.to_string(),
            reason: "must be a host name without scheme, port, path, or whitespace".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_header_name(name: &str) -> Result<(), SandboxPlanError> {
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| !matches!(byte, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'^' | b'_' | b'`' | b'a'..=b'z' | b'|' | b'~'))
    {
        return Err(SandboxPlanError::InvalidCredentialTarget);
    }
    Ok(())
}

pub(crate) fn validate_env_name(name: &str) -> Result<(), SandboxPlanError> {
    if name.is_empty()
        || name.starts_with(|ch: char| ch.is_ascii_digit())
        || !name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(SandboxPlanError::InvalidEnvName {
            env: name.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_env_has_no_raw_sensitive_values(
    env: &HashMap<String, String>,
    allowed_placeholders: &[&str],
) -> Result<(), SandboxPlanError> {
    for (name, value) in env {
        if is_sensitive_env_name(name)
            && !allowed_placeholders
                .iter()
                .any(|placeholder| value == placeholder)
        {
            return Err(SandboxPlanError::RawSecretEnvValue { env: name.clone() });
        }
    }
    Ok(())
}

pub(crate) fn is_container_absolute_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains('\0') && !path.split('/').any(|segment| segment == "..")
}

fn is_sensitive_env_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "API_KEY",
        "ACCESS_KEY",
        "AUTH",
    ]
    .iter()
    .any(|marker| name.contains(marker))
}
