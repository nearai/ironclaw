use std::collections::HashMap;

use crate::ProcessSandboxPlanError;

pub(crate) fn validate_host(host: &str) -> Result<(), ProcessSandboxPlanError> {
    if host.is_empty() {
        return Err(ProcessSandboxPlanError::InvalidHost {
            host: host.to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if host.contains('/') || host.contains(':') || host.chars().any(char::is_whitespace) {
        return Err(ProcessSandboxPlanError::InvalidHost {
            host: host.to_string(),
            reason: "must be a host name without scheme, port, path, or whitespace".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_header_name(name: &str) -> Result<(), ProcessSandboxPlanError> {
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| !matches!(byte, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'^' | b'_' | b'`' | b'a'..=b'z' | b'|' | b'~'))
    {
        return Err(ProcessSandboxPlanError::InvalidCredentialTarget);
    }
    Ok(())
}

pub(crate) fn validate_env_name(name: &str) -> Result<(), ProcessSandboxPlanError> {
    if name.is_empty()
        || name.starts_with(|ch: char| ch.is_ascii_digit())
        || !name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        || is_dangerous_entrypoint_env_name(name)
    {
        return Err(ProcessSandboxPlanError::InvalidEnvName {
            env: name.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_env_has_no_raw_sensitive_values(
    env: &HashMap<String, String>,
    allowed_placeholders: &[&str],
) -> Result<(), ProcessSandboxPlanError> {
    for (name, value) in env {
        if is_sensitive_env_name(name)
            && !allowed_placeholders
                .iter()
                .any(|placeholder| value == placeholder)
        {
            return Err(ProcessSandboxPlanError::RawSecretEnvValue { env: name.clone() });
        }
    }
    Ok(())
}

pub(crate) fn is_container_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('\0')
        && !path.contains(',')
        && !path.split('/').any(|segment| segment == "..")
}

fn is_sensitive_env_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    if [
        "API_KEY",
        "ACCESS_KEY",
        "PRIVATE_KEY",
        "ENCRYPTION_KEY",
        "SYMMETRIC_KEY",
        "SIGNING_KEY",
        "CLIENT_SECRET",
    ]
    .iter()
    .any(|pattern| name.contains(pattern))
    {
        return true;
    }
    name.split('_').any(|part| {
        matches!(
            part,
            "TOKEN" | "SECRET" | "PASSWORD" | "AUTH" | "CREDENTIAL" | "CREDENTIALS" | "BEARER"
        )
    })
}

fn is_dangerous_entrypoint_env_name(name: &str) -> bool {
    matches!(
        name,
        "BASH_ENV"
            | "CDPATH"
            | "ENV"
            | "IFS"
            | "LD_AUDIT"
            | "LD_LIBRARY_PATH"
            | "LD_PRELOAD"
            | "PATH"
            | "SHELLOPTS"
    )
}
